use crate::common;

use anyhow::Result;
use common::{make_v1_index, ods_binary};
use ods::commands::pull::{
    resolve_index, run_with_fetcher, Args, OciBlobFetcher,
};
use ods::index::OdsReleaseIndex;
use sha2::Digest;
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use tempfile::TempDir;

fn sample_release_index() -> OdsReleaseIndex {
    make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("1.0.1", "sha256:a1b2c3d4e5f60718293a4b5c6d7e8f90123456789abcdef0123456789abcdef0")],
    )])
}

fn run_mock_http_server(response_body: Vec<u8>) -> (String, mpsc::Sender<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        loop {
            if rx.try_recv().is_ok() {
                break;
            }
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
                    response_body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.write_all(&response_body);
                let _ = stream.flush();
            }
            thread::sleep(std::time::Duration::from_millis(10));
        }
    });
    (format!("http://127.0.0.1:{}/myindex.json", port), tx)
}

fn extract_http_path(buf: &[u8]) -> Option<String> {
    let line_end = buf.iter().position(|&b| b == b'\r' || b == b'\n')?;
    let line_str = std::str::from_utf8(&buf[..line_end]).ok()?;
    let mut parts = line_str.split_whitespace();
    let _method = parts.next()?;
    let full_path = parts.next()?;
    let path = full_path.split('?').next().unwrap_or(full_path);
    Some(path.to_string())
}

/// Binds an ephemeral port and hands back its base URL before anything is served, so a
/// caller can build routes (an index naming its own mirror, say) that need to know the
/// port in advance. Pair with `serve_routed_mock_server`.
fn bind_routed_mock_server() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let port = listener.local_addr().unwrap().port();
    (listener, format!("http://127.0.0.1:{}", port))
}

/// Serves `routes` on an already-bound listener, routing by exact request path and
/// answering anything else with a 404 — enough to stand in for an OCI registry's
/// `/manifests/<digest>` and `/blobs/<digest>` endpoints in a real subprocess pull.
fn serve_routed_mock_server(listener: TcpListener, routes: BTreeMap<String, Vec<u8>>) -> mpsc::Sender<()> {
    let routes = std::sync::Arc::new(routes);
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        loop {
            if rx.try_recv().is_ok() {
                break;
            }
            if let Ok((mut stream, _)) = listener.accept() {
                let _ = stream.set_nonblocking(false);
                let routes = routes.clone();
                thread::spawn(move || {
                    let mut buf = [0u8; 4096];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    if n > 0 {
                        if let Some(path) = extract_http_path(&buf[..n]) {
                            if let Some(body) = routes.get(&path) {
                                let header = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                                    body.len()
                                );
                                let _ = stream.write_all(header.as_bytes());
                                let _ = stream.write_all(body);
                                let _ = stream.flush();
                            } else {
                                let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                                let _ = stream.write_all(resp.as_bytes());
                                let _ = stream.flush();
                            }
                        }
                    }
                });
            }
            thread::sleep(std::time::Duration::from_millis(5));
        }
    });
    tx
}

/// Binds and serves in one call, for a caller with no need to know the port before the
/// routes are built.
fn run_routed_mock_server(routes: BTreeMap<String, Vec<u8>>) -> (String, mpsc::Sender<()>) {
    let (listener, base_url) = bind_routed_mock_server();
    let tx = serve_routed_mock_server(listener, routes);
    (base_url, tx)
}

/// Builds a real, tiny release (three files) on disk and its OCI manifest, returning the
/// manifest's digest, the total layer size (a dataset row's `bytes`), and the routes a
/// `run_routed_mock_server` needs to serve it at `/v2/ods-data/manifests/<digest>` and
/// `/v2/ods-data/blobs/<digest>`.
fn build_pull_fixture(tmp: &std::path::Path, release_date: &str, version: &str) -> (String, u64, BTreeMap<String, Vec<u8>>) {
    let prov = serde_json::json!({ "written_by": "an older ods, and never a layer" });
    let prov_bytes = serde_json::to_vec_pretty(&prov).unwrap();
    let prov_sha = format!("sha256:{:x}", sha2::Sha256::digest(&prov_bytes));

    let orgs_bytes = common::fixture_parquet_bytes(release_date, "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933", version, "sample orgs parquet bytes for the pull index flag test");
    let orgs_sha = format!("sha256:{:x}", sha2::Sha256::digest(&orgs_bytes));

    let dp = serde_json::json!({ "name": "ods", "version": version, "resources": [] });
    let dp_bytes = serde_json::to_vec_pretty(&dp).unwrap();
    let dp_sha = format!("sha256:{:x}", sha2::Sha256::digest(&dp_bytes));

    let fixture_dir = tmp.join(format!("fixture_{}", release_date));
    fs::create_dir_all(&fixture_dir).unwrap();
    fs::write(fixture_dir.join("orgs.parquet"), &orgs_bytes).unwrap();
    fs::write(fixture_dir.join("_provenance.json"), &prov_bytes).unwrap();
    fs::write(fixture_dir.join(ods::datapackage::DATAPACKAGE_FILENAME), &dp_bytes).unwrap();

    let (manifest, manifest_bytes) =
        ods::commands::make_oci::build_manifest_from_dir(&fixture_dir).unwrap();
    let manifest_digest = manifest.digest().unwrap();
    let total_size: u64 = manifest.layers.iter().map(|l| l.size).sum();

    let mut routes = BTreeMap::new();
    routes.insert(format!("/v2/ods-data/manifests/{}", manifest_digest), manifest_bytes);
    routes.insert(format!("/v2/ods-data/blobs/{}", prov_sha), prov_bytes);
    routes.insert(format!("/v2/ods-data/blobs/{}", orgs_sha), orgs_bytes);
    routes.insert(format!("/v2/ods-data/blobs/{}", dp_sha), dp_bytes);

    (manifest_digest, total_size, routes)
}

struct MockOciFetcher {
    pub remote_index: Option<OdsReleaseIndex>,
    pub responses: BTreeMap<String, Vec<u8>>,
}

impl OciBlobFetcher for MockOciFetcher {
    fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>> {
        for (key, val) in &self.responses {
            if url == key || url.ends_with(key) {
                return Ok(val.clone());
            }
        }
        anyhow::bail!("Mock asset not found for URL: {}", url)
    }

    fn fetch_release_index(&self) -> Result<Option<OdsReleaseIndex>> {
        Ok(self.remote_index.clone())
    }
}

// ----------------------------------------------------------------------------
// Task 1 — The flag
// ----------------------------------------------------------------------------

#[test]
fn test_index_flag_file_path_list() {
    let tmp = TempDir::new().unwrap();
    let index_file = tmp.path().join("myindex.json");
    let index = sample_release_index();
    let index_bytes = serde_json::to_vec_pretty(&index).unwrap();
    fs::write(&index_file, &index_bytes).unwrap();

    let output = ods_binary()
        .current_dir(tmp.path())
        .arg("pull")
        .arg("--index")
        .arg("./myindex.json")
        .arg("--list")
        .output()
        .expect("execute ods pull --index ./myindex.json --list");

    assert!(output.status.success(), "Command must exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("2026-07-31"), "stdout should list 2026-07-31");
    assert!(stdout.contains("1.0.1"), "stdout should list 1.0.1");
}

#[test]
fn test_index_flag_http_url_list() {
    let tmp = TempDir::new().unwrap();
    let index = sample_release_index();
    let index_bytes = serde_json::to_vec_pretty(&index).unwrap();

    let (url, stop_server) = run_mock_http_server(index_bytes);

    let output = ods_binary()
        .current_dir(tmp.path())
        .arg("pull")
        .arg("--index")
        .arg(&url)
        .arg("--list")
        .output()
        .expect("execute ods pull --index <url> --list");

    let _ = stop_server.send(());

    assert!(output.status.success(), "Command must exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("2026-07-31"), "stdout should list 2026-07-31");
    assert!(stdout.contains("1.0.1"), "stdout should list 1.0.1");
}

// ----------------------------------------------------------------------------
// Task 2 — Say when it does not work
// ----------------------------------------------------------------------------

#[test]
fn test_index_flag_cannot_read_error_verbatim() {
    let tmp = TempDir::new().unwrap();

    let output = ods_binary()
        .current_dir(tmp.path())
        .arg("pull")
        .arg("--index")
        .arg("./nope.json")
        .arg("--list")
        .output()
        .expect("execute ods pull --index ./nope.json --list");

    assert!(!output.status.success(), "Command must exit non-zero on unreadable index");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let expected = "✖ Cannot read release index './nope.json': No such file or directory";
    assert!(
        stderr.contains(expected),
        "stderr must contain verbatim '{}', got: {}",
        expected,
        stderr
    );
}

#[test]
fn test_index_flag_cannot_parse_error_verbatim() {
    let tmp = TempDir::new().unwrap();
    let notjson_file = tmp.path().join("notjson.txt");
    fs::write(&notjson_file, b"this is not json").unwrap();

    let output = ods_binary()
        .current_dir(tmp.path())
        .arg("pull")
        .arg("--index")
        .arg("./notjson.txt")
        .arg("--list")
        .output()
        .expect("execute ods pull --index ./notjson.txt --list");

    assert!(!output.status.success(), "Command must exit non-zero on unparseable index");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let expected_line1 = "✖ Cannot parse release index './notjson.txt' as an ODS release index";
    let expected_line2 = "  Expected $schema https://ods.fyi/schema/releases.v1.json";
    assert!(
        stderr.contains(expected_line1),
        "stderr must contain line 1 verbatim: '{}', got: {}",
        expected_line1,
        stderr
    );
    assert!(
        stderr.contains(expected_line2),
        "stderr must contain line 2 verbatim: '{}', got: {}",
        expected_line2,
        stderr
    );
}

// An index in the old format is refused by name, never read as some other failure.
#[test]
fn test_index_flag_refuses_an_old_format_index() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("old.json"), common::OLD_FORMAT_INDEX).unwrap();

    let output = ods_binary()
        .current_dir(tmp.path())
        .args(["pull", "--index", "./old.json", "--list"])
        .output()
        .expect("execute ods pull --index ./old.json --list");

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    println!("{stderr}");
    assert!(
        stderr.contains("✖ Cannot read release index './old.json': it's in the old format, with trud_release_date and dataset_version"),
        "{stderr}"
    );
    assert!(stderr.contains("  This ods reads the current format: name, source, and releases[].source.version, .hash and .bytes"), "{stderr}");
}

#[test]
fn test_empty_releases_index_prints_message_and_exits_zero() {
    let tmp = TempDir::new().unwrap();
    let empty_file = tmp.path().join("empty.json");
    fs::write(
        &empty_file,
        serde_json::to_vec(&ods::index::OdsReleaseIndex { mirrors: vec![], ..ods::index::OdsReleaseIndex::default() }).unwrap(),
    )
    .unwrap();

    let output = ods_binary()
        .current_dir(tmp.path())
        .arg("pull")
        .arg("--index")
        .arg("./empty.json")
        .arg("--list")
        .output()
        .expect("execute ods pull --index ./empty.json --list");

    assert!(output.status.success(), "Command must exit 0 on empty index");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("(No dataset releases available in index)"),
        "stdout should state no releases available, got: {}",
        stdout
    );
}

#[test]
fn test_index_flag_env_var_and_flag_precedence() {
    let tmp = TempDir::new().unwrap();

    // 1. When only ODS_RELEASE_INDEX_URL points to a missing file/URL,
    // it falls back quietly to the baked index and exits 0, listing what the baked index lists.
    let output_env_only = ods_binary()
        .current_dir(tmp.path())
        .env("ODS_RELEASE_INDEX_URL", "http://127.0.0.1:9999/does-not-exist.json")
        .arg("pull")
        .arg("--list")
        .output()
        .expect("execute with ODS_RELEASE_INDEX_URL");

    assert!(
        output_env_only.status.success(),
        "ODS_RELEASE_INDEX_URL fallback must exit 0"
    );
    let stdout_env_only = String::from_utf8_lossy(&output_env_only.stdout);
    let baked = ods::index::OdsReleaseIndex::baked().unwrap();
    let baked_dates: Vec<&str> = baked
        .releases
        .iter()
        .filter(|r| !r.datasets.is_empty())
        .map(|r| r.source.version.as_str())
        .collect();
    if baked_dates.is_empty() {
        assert!(
            stdout_env_only.contains("(No dataset releases available in index)"),
            "Quiet fallback to the empty baked index, got: {}",
            stdout_env_only
        );
    }
    for date in &baked_dates {
        assert!(
            stdout_env_only.contains(date),
            "Quiet fallback to the baked index lists {}, got: {}",
            date,
            stdout_env_only
        );
    }

    // 2. When both ODS_RELEASE_INDEX_URL and --index are set, --index wins: the pull
    // completes from the file's own mirror, and the block's `verified` row names the
    // file, not the (unreachable) env var URL.
    let (manifest_digest, total_size, routes) = build_pull_fixture(tmp.path(), "2026-07-31", "1.0.1");
    let (base_url, stop_oci) = run_routed_mock_server(routes);

    let mut index = sample_release_index();
    index.mirrors = vec![ods::index::MirrorEntry { url: format!("{}/v2/ods-data", base_url) }];
    index.releases[0].datasets[0].manifest_digest = manifest_digest;
    index.releases[0].datasets[0].bytes = total_size;

    let myindex_file = tmp.path().join("myindex.json");
    fs::write(&myindex_file, serde_json::to_vec_pretty(&index).unwrap()).unwrap();

    let output_both = ods_binary()
        .current_dir(tmp.path())
        .env("ODS_RELEASE_INDEX_URL", "http://127.0.0.1:9999/does-not-exist.json")
        .arg("pull")
        .arg("--index")
        .arg("./myindex.json")
        .output()
        .expect("execute with both env var and --index");

    let _ = stop_oci.send(());

    assert!(
        output_both.status.success(),
        "--index must succeed, got: {}",
        String::from_utf8_lossy(&output_both.stderr)
    );
    let stderr_both = String::from_utf8_lossy(&output_both.stderr);
    assert!(
        stderr_both.contains("verified      sha256 from ./myindex.json"),
        "--index must take precedence over ODS_RELEASE_INDEX_URL, and the block's verified row must name the file, got: {}",
        stderr_both
    );
}

// ----------------------------------------------------------------------------
// Task 3 — The index still merges
// ----------------------------------------------------------------------------

#[test]
fn test_supplied_index_resolves_and_pulls_absent_release() -> Result<()> {
    let tmp = TempDir::new()?;
    let workspace = tmp.path().join("ods_data");

    // Prepare layer files for a mock OCI pull
    let prov = serde_json::json!({ "written_by": "an older ods, and never a layer" });
    let prov_bytes = serde_json::to_vec_pretty(&prov)?;
    let prov_sha = format!("sha256:{:x}", sha2::Sha256::digest(&prov_bytes));

    let orgs_bytes = common::fixture_parquet_bytes("2026-07-31", "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933", "1.0.1", "sample orgs parquet bytes");
    let orgs_sha = format!("sha256:{:x}", sha2::Sha256::digest(&orgs_bytes));

    let dp = serde_json::json!({
        "name": "ods",
        "version": "1.0.1",
        "resources": []
    });
    let dp_bytes = serde_json::to_vec_pretty(&dp)?;
    let dp_sha = format!("sha256:{:x}", sha2::Sha256::digest(&dp_bytes));

    let fixture_dir = tmp.path().join("fixture_task3");
    fs::create_dir_all(&fixture_dir)?;
    fs::write(fixture_dir.join("orgs.parquet"), &orgs_bytes)?;
    fs::write(fixture_dir.join("_provenance.json"), &prov_bytes)?;
    fs::write(fixture_dir.join(ods::datapackage::DATAPACKAGE_FILENAME), &dp_bytes)?;

    let (manifest, manifest_bytes) = ods::commands::make_oci::build_manifest_from_dir(&fixture_dir)?;
    let manifest_digest = manifest.digest()?;

    let supplied_index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("1.0.1", &manifest_digest)],
    )]);

    let index_file = tmp.path().join("supplied_index.json");
    fs::write(&index_file, serde_json::to_vec_pretty(&supplied_index)?)?;

    let mut responses = BTreeMap::new();
    responses.insert(format!("manifests/{}", manifest_digest), manifest_bytes);
    responses.insert(format!("blobs/{}", prov_sha), prov_bytes);
    responses.insert(format!("blobs/{}", orgs_sha), orgs_bytes);
    responses.insert(format!("blobs/{}", dp_sha), dp_bytes);

    let fetcher = MockOciFetcher {
        remote_index: None,
        responses,
    };

    run_with_fetcher(
        Args {
            index: Some(index_file.to_string_lossy().to_string()),
            ..Default::default()
        },
        &workspace,
        &fetcher,
    )?;

    let rel_dir = workspace.join("releases").join("2026-07-31");
    assert!(rel_dir.exists(), "Release directory must exist after pull");
    assert!(rel_dir.join("orgs.parquet").exists());
    // A pull writes the layers and the `datapackage.json` view, and stores no manifest.
    assert!(rel_dir.join("datapackage.json").exists(), "pulled release must have the datapackage.json view");
    assert!(!rel_dir.join("oci").exists(), "a pull stores no manifest");

    Ok(())
}

#[test]
fn test_supplied_index_selects_and_does_not_merge_or_reject_differing_digest() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");
    fs::create_dir_all(&workspace).unwrap();

    let custom_digest = "sha256:2222222222222222222222222222222222222222222222222222222222222222";

    let custom_index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("1.0.1", custom_digest)],
    )]);

    let custom_file = tmp.path().join("custom_index.json");
    fs::write(&custom_file, serde_json::to_vec_pretty(&custom_index).unwrap()).unwrap();

    let fetcher = MockOciFetcher {
        remote_index: None,
        responses: BTreeMap::new(),
    };

    // Test resolve_index directly selects the supplied index
    let (selected, _) = resolve_index(
        &workspace,
        Some(custom_file.to_str().unwrap()),
        true,
        true,
        &fetcher,
    )
    .unwrap();

    assert_eq!(
        selected.releases[0].datasets[0].manifest_digest,
        custom_digest,
        "Selected index must be the supplied one, not merged with baked"
    );

    // Also test via run_with_fetcher
    let res = run_with_fetcher(
        Args {
            index: Some(custom_file.to_str().unwrap().to_string()),
            list: true,
            ..Default::default()
        },
        &workspace,
        &fetcher,
    );
    assert!(res.is_ok());
}

#[test]
fn test_pull_index_flag_does_not_mutate_cached_workspace_index() -> Result<()> {
    let tmp = TempDir::new()?;
    let workspace = tmp.path().join("ods_data");
    fs::create_dir_all(&workspace)?;

    // Create an initial workspace _releases.json
    let initial_index = sample_release_index();
    let initial_bytes = serde_json::to_vec_pretty(&initial_index)?;
    let ws_index_file = workspace.join("_releases.json");
    fs::write(&ws_index_file, &initial_bytes)?;

    // Create a different index for --index
    let mut supplied_index = sample_release_index();
    supplied_index.releases[0].source.version = "2026-08-31".to_string();
    for ds in &mut supplied_index.releases[0].datasets {
        ds.version = format!("2026-08-31_{}", ds.dataset_version());
    }
    let supplied_file = tmp.path().join("supplied_releases.json");
    fs::write(&supplied_file, serde_json::to_vec_pretty(&supplied_index)?)?;

    let output = ods_binary()
        .current_dir(tmp.path())
        .arg("pull")
        .arg("--index")
        .arg(&supplied_file)
        .arg("--list")
        .output()?;

    assert!(output.status.success(), "Command must succeed");
    let current_bytes = fs::read(&ws_index_file)?;
    assert_eq!(
        initial_bytes, current_bytes,
        "Workspace _releases.json must remain byte-identical after pull --index"
    );

    Ok(())
}

#[test]
fn test_ods_release_index_url_is_used_and_saves_exact_bytes_and_names_it_verified() -> Result<()> {
    let tmp = TempDir::new()?;

    // The index is served from the same ephemeral server as its own mirror, so the mirror
    // URL needs the port before the index bytes are built.
    let (listener, base_url) = bind_routed_mock_server();

    let (manifest_digest, total_size, mut routes) = build_pull_fixture(tmp.path(), "2026-07-31", "1.0.1");
    let mut index = sample_release_index();
    index.mirrors = vec![ods::index::MirrorEntry { url: format!("{}/v2/ods-data", base_url) }];
    index.releases[0].datasets[0].manifest_digest = manifest_digest;
    index.releases[0].datasets[0].bytes = total_size;
    let index_bytes = serde_json::to_vec_pretty(&index)?;
    routes.insert("/releases.json".to_string(), index_bytes.clone());

    let index_url = format!("{}/releases.json", base_url);
    let stop_server = serve_routed_mock_server(listener, routes);

    let output = ods_binary()
        .current_dir(tmp.path())
        .env("ODS_RELEASE_INDEX_URL", &index_url)
        .arg("pull")
        .output()?;

    let _ = stop_server.send(());

    assert!(
        output.status.success(),
        "Command must succeed, got: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    let expected_verified = format!("verified      sha256 from {}", index_url);
    assert!(
        stderr.contains(&expected_verified),
        "block's verified row must name ODS_RELEASE_INDEX_URL, got:\n{}",
        stderr
    );

    let ws_index = tmp.path().join("ods_data").join("_releases.json");
    assert!(ws_index.exists(), "Workspace _releases.json must be written");
    let saved_bytes = fs::read(&ws_index)?;
    assert_eq!(
        index_bytes, saved_bytes,
        "Saved _releases.json must match served bytes verbatim"
    );

    Ok(())
}

#[test]
fn test_pull_index_fetch_invalid_json_does_not_mutate_workspace_index() -> Result<()> {
    let tmp = TempDir::new()?;
    let workspace = tmp.path().join("ods_data");
    fs::create_dir_all(&workspace)?;

    let initial_index = sample_release_index();
    let initial_bytes = serde_json::to_vec_pretty(&initial_index)?;
    let ws_index_file = workspace.join("_releases.json");
    fs::write(&ws_index_file, &initial_bytes)?;

    let (url, stop_server) = run_mock_http_server(b"invalid unparseable json content".to_vec());

    let output = ods_binary()
        .current_dir(tmp.path())
        .env("ODS_RELEASE_INDEX_URL", &url)
        .arg("pull")
        .arg("--list")
        .output()?;

    let _ = stop_server.send(());

    assert!(output.status.success(), "Command must exit 0 falling back to cache");
    let current_bytes = fs::read(&ws_index_file)?;
    assert_eq!(
        initial_bytes, current_bytes,
        "Workspace _releases.json must remain unchanged when served unparseable index"
    );

    Ok(())
}

// `ods pull --list --format json` names each dataset as the release index does.
#[test]
fn test_pull_list_json_speaks_the_index_vocabulary() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("index.json"), serde_json::to_vec_pretty(&sample_release_index()).unwrap()).unwrap();

    let output = ods_binary()
        .current_dir(tmp.path())
        .args(["pull", "--index", "./index.json", "--list", "--format", "json"])
        .output()
        .expect("execute ods pull --list --format json");

    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let items: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert_eq!(
        items,
        serde_json::json!([{
            "source_version": "2026-07-31",
            "version": "2026-07-31_1.0.1",
            "status": "remote",
            "manifest_digest": "sha256:a1b2c3d4e5f60718293a4b5c6d7e8f90123456789abcdef0123456789abcdef0"
        }])
    );
}

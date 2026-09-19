mod common;

use anyhow::Result;
use common::make_v1_index;
use ods::commands::pull::{
    resolve_index_with_baked, run_with_fetcher, run_with_fetcher_and_baked, Args, OciBlobFetcher,
};
use ods::index::OdsReleaseIndex;
use sha2::Digest;
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use tempfile::TempDir;

fn ods_binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ods"))
}

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

#[test]
fn test_empty_releases_index_prints_message_and_exits_zero() {
    let tmp = TempDir::new().unwrap();
    let empty_file = tmp.path().join("empty.json");
    fs::write(
        &empty_file,
        b"{\"$schema\":\"https://ods.fyi/schema/releases.v1.json\",\"trud_signing_key_fingerprint\":\"71ED5964BAE53E83556320A42BE59DADEE84BEB0\",\"mirrors\":[],\"releases\":[]}",
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
    // it falls back quietly to the baked index (which is currently empty) and exits 0.
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
    assert!(
        stdout_env_only.contains("(No dataset releases available in index)"),
        "Quiet fallback to baked index without error"
    );

    // 2. When both ODS_RELEASE_INDEX_URL and --index are set, --index wins.
    let myindex_file = tmp.path().join("myindex.json");
    let index = sample_release_index();
    fs::write(&myindex_file, serde_json::to_vec_pretty(&index).unwrap()).unwrap();

    let output_both = ods_binary()
        .current_dir(tmp.path())
        .env("ODS_RELEASE_INDEX_URL", "http://127.0.0.1:9999/does-not-exist.json")
        .arg("pull")
        .arg("--index")
        .arg("./myindex.json")
        .arg("--list")
        .output()
        .expect("execute with both env var and --index");

    assert!(output_both.status.success(), "--index must succeed");
    let stdout_both = String::from_utf8_lossy(&output_both.stdout);
    let stderr_both = String::from_utf8_lossy(&output_both.stderr);
    assert!(
        stdout_both.contains("2026-07-31"),
        "--index must take precedence over ODS_RELEASE_INDEX_URL"
    );
    assert!(
        !stderr_both.contains("* Index:"),
        "When --index is given, no fetch runs and '* Index:' line must be absent from stderr, got: {}",
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
    let prov = ods::provenance::OdsProvenance {
        trud_release_date: Some("2026-07-31".to_string()),
        dataset_version: Some("1.0.1".to_string()),
        trud_release_sha256: Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string()),
        trud_release_sha256_verified: Some(ods::provenance::TrudVerificationSource::TrudApi),
        ..Default::default()
    };
    let prov_bytes = serde_json::to_vec_pretty(&prov)?;
    let prov_sha = format!("sha256:{:x}", sha2::Sha256::digest(&prov_bytes));

    let orgs_bytes = b"sample orgs parquet bytes".to_vec();
    let orgs_sha = format!("sha256:{:x}", sha2::Sha256::digest(&orgs_bytes));

    let fixture_dir = tmp.path().join("fixture_task3");
    fs::create_dir_all(&fixture_dir)?;
    fs::write(fixture_dir.join("orgs.parquet"), &orgs_bytes)?;
    fs::write(fixture_dir.join(ods::provenance::PROVENANCE_FILENAME), &prov_bytes)?;

    let (manifest, manifest_bytes) = ods::commands::make_oci::build_manifest_from_dir(&fixture_dir, &prov, "1.0.1")?;
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
    assert!(rel_dir.join("_provenance.json").exists());

    Ok(())
}

#[test]
fn test_contradicting_digest_fails_with_security_error_naming_both_digests() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");
    fs::create_dir_all(&workspace).unwrap();

    let baked_digest = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
    let tampered_digest = "sha256:2222222222222222222222222222222222222222222222222222222222222222";

    let baked_index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("1.0.1", baked_digest)],
    )]);

    let tampered_index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("1.0.1", tampered_digest)],
    )]);

    let tampered_file = tmp.path().join("tampered_index.json");
    fs::write(&tampered_file, serde_json::to_vec_pretty(&tampered_index).unwrap()).unwrap();

    let fetcher = MockOciFetcher {
        remote_index: None,
        responses: BTreeMap::new(),
    };

    // Test resolve_index_with_baked directly
    let err = resolve_index_with_baked(
        &workspace,
        &fetcher,
        Some(baked_index.clone()),
        Some(tampered_file.to_str().unwrap()),
    )
    .unwrap_err();

    let err_str = err.to_string();
    assert!(
        err.downcast_ref::<ods::index::SecurityError>().is_some(),
        "Error must downcast to SecurityError"
    );
    assert!(
        err_str.contains(baked_digest),
        "Error message must name baked digest: {}",
        err_str
    );
    assert!(
        err_str.contains(tampered_digest),
        "Error message must name fetched/supplied digest: {}",
        err_str
    );

    // Also test via run_with_fetcher_and_baked
    let run_err = run_with_fetcher_and_baked(
        Args {
            index: Some(tampered_file.to_str().unwrap().to_string()),
            list: true,
            ..Default::default()
        },
        &workspace,
        &fetcher,
        Some(baked_index),
    )
    .unwrap_err();

    let run_err_str = run_err.to_string();
    assert!(
        run_err_str.contains(baked_digest) && run_err_str.contains(tampered_digest),
        "run_with_fetcher_and_baked must propagate SecurityError naming both digests: {}",
        run_err_str
    );
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
    supplied_index.releases[0].trud_release_date = "2026-08-31".to_string();
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
fn test_ods_release_index_url_prints_stderr_notice_and_saves_exact_bytes() -> Result<()> {
    let tmp = TempDir::new()?;
    let index = sample_release_index();
    let index_bytes = serde_json::to_vec_pretty(&index)?;

    let (url, stop_server) = run_mock_http_server(index_bytes.clone());

    let output = ods_binary()
        .current_dir(tmp.path())
        .env("ODS_RELEASE_INDEX_URL", &url)
        .arg("pull")
        .arg("--list")
        .output()?;

    let _ = stop_server.send(());

    assert!(output.status.success(), "Command must succeed");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let expected_notice = format!("* Index: {} (ODS_RELEASE_INDEX_URL)", url);
    assert!(
        stderr.contains(&expected_notice),
        "stderr must contain notice line '{}', got:\n{}",
        expected_notice,
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

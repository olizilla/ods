use crate::common;

use anyhow::Result;
use common::{create_mock_trud_zip, make_v1_index, ods_binary};
use ods::commands::fetch::{
    run_local_archive_with_fetchers, Args as FetchArgs, TrudFetcher, TrudReleaseItem,
};
use ods::progress::{Progress, ProgressCaps};
use ods::provenance::{compute_file_sha256, TRUD_ARCHIVE_PACKAGE_FILENAME};

/// The `datapackage` object `dir`'s `orgs.parquet` carries, as raw JSON.
fn embedded_of(dir: &std::path::Path) -> String {
    ods::provenance::read_embedded_value(&dir.join("orgs.parquet"))
        .unwrap()
        .expect("orgs.parquet carries a datapackage object")
}
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

struct BufferWriter(Arc<Mutex<Vec<u8>>>);
impl std::io::Write for BufferWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct MockTrudApiFetcher {
    releases: Vec<TrudReleaseItem>,
}

impl TrudFetcher for MockTrudApiFetcher {
    fn fetch_releases(&self) -> Result<Vec<TrudReleaseItem>> {
        Ok(self.releases.clone())
    }

    fn download_archive(
        &self,
        _url: &str,
        _dest_path: &Path,
        _on_bytes: &(dyn Fn(u64) + Send + Sync),
    ) -> Result<()> {
        anyhow::bail!("download_archive not implemented in MockTrudApiFetcher");
    }
}

struct FailingTrudApiFetcher {
    error_message: String,
}

impl TrudFetcher for FailingTrudApiFetcher {
    fn fetch_releases(&self) -> Result<Vec<TrudReleaseItem>> {
        anyhow::bail!("{}", self.error_message);
    }

    fn download_archive(
        &self,
        _url: &str,
        _dest_path: &Path,
        _on_bytes: &(dyn Fn(u64) + Send + Sync),
    ) -> Result<()> {
        anyhow::bail!("download_archive not implemented in FailingTrudApiFetcher");
    }
}

fn list_directory_recursive(dir: &Path) -> Vec<PathBuf> {
    if !dir.exists() {
        return vec![];
    }
    let mut files: Vec<PathBuf> = walkdir::WalkDir::new(dir)
        .into_iter()
        .flatten()
        .map(|e| e.path().to_path_buf())
        .collect();
    files.sort();
    files
}

#[test]
fn test_make_zip_absent_from_index_is_unverified() {
    let tmp = TempDir::new().unwrap();
    let zip_path = create_mock_trud_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_20260731000001.zip");
    let out_dir = tmp.path().join("out");

    let output = ods_binary()
        .arg("make")
        .arg("-i")
        .arg(&zip_path)
        .arg("-o")
        .arg(&out_dir)
        .arg("--force")
        .output()
        .expect("execute ods make");

    assert!(output.status.success(), "ods make must succeed, stderr:\n{}", String::from_utf8_lossy(&output.stderr));
    let stderr = String::from_utf8_lossy(&output.stderr);
    let filename = zip_path.file_name().unwrap().to_str().unwrap();
    assert!(
        stderr.contains(&format!("! {} isn't a TRUD release ods knows", filename)),
        "stderr must report unverified: {}",
        stderr
    );
    assert!(stderr.contains("Built without provenance. You can explore it with find, info and role, but not cite or publish it."));

    assert!(!out_dir.join("trud").exists(), "nothing is written under trud/ for a build");
    for table in ["orgs", "roles", "relationships", "successions"] {
        let value = ods::provenance::read_embedded_value(&out_dir.join(format!("{table}.parquet"))).unwrap();
        assert_eq!(value, None, "{table}.parquet carries no datapackage key");
    }
    // The view says only what the files are: their resources, and that their source and terms
    // are unknown.
    let view: serde_json::Value = serde_json::from_str(&fs::read_to_string(out_dir.join("datapackage.json")).unwrap()).unwrap();
    let keys: Vec<&str> = view.as_object().unwrap().keys().map(|k| k.as_str()).collect();
    assert_eq!(keys, ["$schema", "description", "resources"], "{view}");
    assert_eq!(view["resources"].as_array().unwrap().len(), 4);
}

#[test]
fn test_trud_pull_local_archive_in_index_is_verified_by_the_index() {
    let tmp = TempDir::new().unwrap();
    let zip_path = create_mock_trud_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_20260731000001.zip");
    let sha256 = compute_file_sha256(&zip_path).unwrap();
    let file_size = fs::metadata(&zip_path).unwrap().len();

    let index = make_v1_index(&[(
        "2026-07-31",
        &sha256,
        file_size,
        &[("1.0.1", "sha256:0000000000000000000000000000000000000000000000000000000000000000")],
    )]);
    let index_file = tmp.path().join("index.json");
    fs::write(&index_file, serde_json::to_string_pretty(&index).unwrap()).unwrap();

    let ws = tmp.path().join("ods_data");
    let output = ods_binary()
        .arg("trud")
        .arg("pull")
        .arg("--local-archive")
        .arg(&zip_path)
        .arg("--index")
        .arg(&index_file)
        .arg("-w")
        .arg(&ws)
        .output()
        .expect("execute trud pull");

    assert!(output.status.success(), "stderr:\n{}", String::from_utf8_lossy(&output.stderr));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("local archive, SHA-256 verified by ods release index"),
        "stderr was: {}",
        stderr
    );

    let prov_file = ws.join("releases").join("2026-07-31").join("trud").join(TRUD_ARCHIVE_PACKAGE_FILENAME);
    assert!(prov_file.exists());
}

#[test]
fn test_make_zip_in_index_is_verified_by_the_index() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    fs::create_dir_all(&ws).unwrap();

    let zip_path = create_mock_trud_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_20260731000001.zip");
    let sha256 = compute_file_sha256(&zip_path).unwrap();
    let file_size = fs::metadata(&zip_path).unwrap().len();

    let index = make_v1_index(&[(
        "2026-07-31",
        &sha256,
        file_size,
        &[("1.0.1", "sha256:0000000000000000000000000000000000000000000000000000000000000000")],
    )]);
    ods::index::OdsReleaseIndex::save_to_workspace_bytes(
        &serde_json::to_vec_pretty(&index).unwrap(),
        &ws,
    )
    .unwrap();

    let out_dir = ws.join("releases").join("2026-07-31");
    let output = ods_binary()
        .current_dir(&ws)
        .arg("make")
        .arg("-i")
        .arg(&zip_path)
        .arg("-o")
        .arg(&out_dir)
        .output()
        .expect("execute ods make");

    assert!(output.status.success(), "stderr:\n{}", String::from_utf8_lossy(&output.stderr));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(&format!("✓ {}  SHA-256 verified by ods release index", zip_path.display())),
        "stderr was: {}",
        stderr
    );

    // The files carry the provenance the index vouched for; a bare zip gets no trud/ record.
    let record = ods::provenance::read_release(&out_dir).unwrap();
    assert_eq!(record.facts().expect("provenance embedded").release_date, "2026-07-31");
}

#[test]
fn test_trud_pull_local_archive_absent_from_index_matching_trud_api_is_verified_by_trud() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    fs::create_dir_all(&ws).unwrap();

    let zip_path = create_mock_trud_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_20260731000001.zip");
    let sha256 = compute_file_sha256(&zip_path).unwrap();
    let file_size = fs::metadata(&zip_path).unwrap().len();

    // Index has a different date, so 2026-07-31 is absent
    let index = make_v1_index(&[(
        "2026-06-26",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        3000000,
        &[("1.0.0", "sha256:0000000000000000000000000000000000000000000000000000000000000000")],
    )]);
    let index_file = tmp.path().join("index.json");
    fs::write(&index_file, serde_json::to_string_pretty(&index).unwrap()).unwrap();

    let trud_fetcher = MockTrudApiFetcher {
        releases: vec![TrudReleaseItem {
            id: "341".to_string(),
            name: Some("Release 7.0.0".to_string()),
            release_date: "2026-07-31".to_string(),
            archive_file_name: "hscorgrefdataxml_data_7.0.0_20260731000001.zip".to_string(),
            archive_file_sha256: sha256.clone(),
            archive_file_size: file_size,
            download_url: "https://example.com/zip".to_string(),
            ..Default::default()
        }],
    };

    let buffer = Arc::new(Mutex::new(Vec::new()));
    let caps = ProgressCaps {
        is_tty: false,
        no_color: true,
        quiet: false,
        verbose: false,
        width: 80,
    };
    let progress = Progress::new(caps, Box::new(BufferWriter(buffer.clone())));

    let args = FetchArgs {
        local_archive: Some(zip_path),
        workspace: Some(ws.clone()),
        index: Some(index_file.to_str().unwrap().to_string()),
        api_key: Some("dummy_key".to_string()),
        ..Default::default()
    };

    let oci_fetcher = ods::commands::pull::HttpOciFetcher;
    let res = run_local_archive_with_fetchers(
        &args,
        &ws,
        args.local_archive.as_ref().unwrap(),
        &progress,
        Some(&trud_fetcher),
        Some(&oci_fetcher),
    );
    assert!(res.is_ok(), "{:?}", res.err());

    let output = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
    assert!(
        output.contains("local archive, SHA-256 verified by TRUD API"),
        "output was: {}",
        output
    );

    let prov_file = ws.join("releases").join("2026-07-31").join("trud").join(TRUD_ARCHIVE_PACKAGE_FILENAME);
    assert!(prov_file.exists());
}

#[test]
fn test_trud_pull_local_archive_hash_mismatch_exits_1_and_leaves_workspace_untouched() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    let releases_dir = ws.join("releases");
    fs::create_dir_all(&releases_dir).unwrap();

    let zip_path =
        create_mock_trud_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_20260731000001.zip");
    let _actual_sha = compute_file_sha256(&zip_path).unwrap();

    let bad_sha = "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF";
    let index = make_v1_index(&[(
        "2026-07-31",
        bad_sha,
        100000,
        &[("1.0.1", "sha256:0000000000000000000000000000000000000000000000000000000000000000")],
    )]);
    let index_file = tmp.path().join("index.json");
    fs::write(&index_file, serde_json::to_string_pretty(&index).unwrap()).unwrap();

    let before_tree = list_directory_recursive(&releases_dir);

    let output = ods_binary()
        .arg("trud")
        .arg("pull")
        .arg("--local-archive")
        .arg(&zip_path)
        .arg("--index")
        .arg(&index_file)
        .arg("-w")
        .arg(&ws)
        .output()
        .expect("execute trud pull");

    assert!(!output.status.success(), "must exit 1 on checksum mismatch");
    assert_eq!(output.status.code(), Some(1));

    let stderr = String::from_utf8_lossy(&output.stderr);
    let filename = zip_path.file_name().unwrap().to_str().unwrap();
    assert!(
        stderr.contains(&format!("✖ {} isn't a TRUD release ods knows", filename)),
        "stderr:\n{}",
        stderr
    );
    assert!(stderr.contains(
        "Build it outside the workspace with -o <dir>, or run ods pull for a newer release index."
    ));

    let after_tree = list_directory_recursive(&releases_dir);
    assert_eq!(
        before_tree, after_tree,
        "ods_data/releases/ must be completely untouched after mismatch!"
    );
}

#[test]
fn test_trud_verify_with_no_key_succeeds_when_in_index() {
    let tmp = TempDir::new().unwrap();
    let zip_path = create_mock_trud_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_20260731000001.zip");
    let sha256 = compute_file_sha256(&zip_path).unwrap();
    let file_size = fs::metadata(&zip_path).unwrap().len();

    let index = make_v1_index(&[(
        "2026-07-31",
        &sha256,
        file_size,
        &[("1.0.1", "sha256:0000000000000000000000000000000000000000000000000000000000000000")],
    )]);
    let index_file = tmp.path().join("index.json");
    fs::write(&index_file, serde_json::to_string_pretty(&index).unwrap()).unwrap();

    let output = ods_binary()
        .current_dir(tmp.path())
        .env_remove("TRUD_API_KEY")
        .arg("trud")
        .arg("verify")
        .arg(&zip_path)
        .arg("--index")
        .arg(&index_file)
        .output()
        .expect("execute trud verify");

    assert!(output.status.success(), "stderr:\n{}", String::from_utf8_lossy(&output.stderr));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("✓ 2026-07-31") && stderr.contains("SHA-256 verified by ods release index"),
        "stderr was:\n{}",
        stderr
    );
}

#[test]
fn test_trud_verify_with_no_key_fails_when_absent_from_index() {
    let tmp = TempDir::new().unwrap();
    let zip_path = create_mock_trud_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_20260731000001.zip");

    // Index does not have 2026-07-31
    let index = make_v1_index(&[(
        "2026-06-26",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        3000000,
        &[("1.0.0", "sha256:0000000000000000000000000000000000000000000000000000000000000000")],
    )]);
    let index_file = tmp.path().join("index.json");
    fs::write(&index_file, serde_json::to_string_pretty(&index).unwrap()).unwrap();

    let output = ods_binary()
        .current_dir(tmp.path())
        .env_remove("TRUD_API_KEY")
        .arg("trud")
        .arg("verify")
        .arg(&zip_path)
        .arg("--index")
        .arg(&index_file)
        .output()
        .expect("execute trud verify");

    assert!(!output.status.success(), "must fail when absent from index with no key");
    assert_eq!(output.status.code(), Some(1));

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("✖ 2026-07-31  no published SHA-256 to compare against"),
        "stderr was:\n{}",
        stderr
    );
    assert!(
        stderr.contains("The ods release index has no entry for 2026-07-31. Set TRUD_API_KEY to check it against TRUD."),
        "stderr was:\n{}",
        stderr
    );
}

#[test]
fn test_trud_verify_cli_missing_path_and_rejected_flag() {
    // Missing required positional <PATH>
    let output = ods_binary()
        .arg("trud")
        .arg("verify")
        .output()
        .expect("execute trud verify with no path");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("<PATH>"),
        "stderr should indicate missing <PATH>: {stderr}"
    );

    // Rejected --path flag
    let output2 = ods_binary()
        .arg("trud")
        .arg("verify")
        .arg("--path")
        .arg("some_path.zip")
        .output()
        .expect("execute trud verify with --path flag");
    assert!(!output2.status.success());
    let stderr2 = String::from_utf8_lossy(&output2.stderr);
    assert!(
        stderr2.contains("unexpected argument '--path'"),
        "stderr should reject --path: {stderr2}"
    );
}

#[test]
fn test_trud_pull_local_archive_failing_trud_api_stays_unverified_and_prints_warning() -> Result<()> {
    let tmp = TempDir::new()?;
    let ws = tmp.path().join("workspace");
    fs::create_dir_all(&ws)?;
    let zip_path = create_mock_trud_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_20260731000001.zip");

    // Index does not have 2026-07-31
    let index = make_v1_index(&[]);
    let index_file = tmp.path().join("index.json");
    fs::write(&index_file, serde_json::to_string_pretty(&index)?)?;

    let buffer = Arc::new(Mutex::new(Vec::new()));
    let caps = ProgressCaps {
        is_tty: false,
        no_color: true,
        quiet: false,
        verbose: false,
        width: 80,
    };
    let progress = Progress::new(caps, Box::new(BufferWriter(buffer.clone())));

    let failing_fetcher = FailingTrudApiFetcher {
        error_message: "connection refused (os error 111)".to_string(),
    };

    let args = FetchArgs {
        local_archive: Some(zip_path),
        workspace: Some(ws.clone()),
        index: Some(index_file.to_str().unwrap().to_string()),
        api_key: Some("dummy_key".to_string()),
        ..Default::default()
    };

    let res = run_local_archive_with_fetchers(
        &args,
        &ws,
        args.local_archive.as_ref().unwrap(),
        &progress,
        Some(&failing_fetcher),
        Option::<&ods::commands::pull::HttpOciFetcher>::None,
    );

    assert!(
        res.is_err(),
        "must fail when TRUD API fails and archive not in index"
    );

    let output = String::from_utf8(buffer.lock().unwrap().clone())?;
    assert!(
        output.contains("! TRUD API check failed: connection refused (os error 111)"),
        "output was: {output}"
    );

    let prov_file = ws
        .join("releases")
        .join("2026-07-31")
        .join("trud").join(TRUD_ARCHIVE_PACKAGE_FILENAME);
    assert!(
        !prov_file.exists(),
        "pull must refuse and write no provenance"
    );

    Ok(())
}

#[test]
fn test_trud_verify_failing_trud_api_exits_1_and_names_failure() -> Result<()> {
    let tmp = TempDir::new()?;
    let zip_path = create_mock_trud_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_20260731000001.zip");

    // Index does not have 2026-07-31
    let index = make_v1_index(&[]);
    let index_file = tmp.path().join("index.json");
    fs::write(&index_file, serde_json::to_string_pretty(&index)?)?;

    let buffer = Arc::new(Mutex::new(Vec::new()));
    let caps = ProgressCaps {
        is_tty: false,
        no_color: true,
        quiet: false,
        verbose: false,
        width: 80,
    };
    let progress = Progress::new(caps, Box::new(BufferWriter(buffer.clone())));

    let failing_fetcher = FailingTrudApiFetcher {
        error_message: "connection refused (os error 111)".to_string(),
    };

    let args = FetchArgs {
        verify_only: Some(zip_path.clone()),
        index: Some(index_file.to_str().unwrap().to_string()),
        api_key: Some("dummy_key".to_string()),
        ..Default::default()
    };

    let res = ods::commands::fetch::run_verify_only_with_fetchers(
        &args,
        tmp.path(),
        &zip_path,
        &progress,
        Some(&failing_fetcher),
        Option::<&ods::commands::pull::HttpOciFetcher>::None,
    );

    assert!(res.is_err(), "must exit 1 on failing TRUD API during verify");

    let output = String::from_utf8(buffer.lock().unwrap().clone())?;
    assert!(
        output.contains("! TRUD API check failed: connection refused (os error 111)"),
        "output was: {output}"
    );
    assert!(
        output.contains("TRUD API check failed: connection refused (os error 111)"),
        "output was: {output}"
    );
    assert!(
        !output.contains("Set TRUD_API_KEY to check it against TRUD"),
        "output should not mention missing key when key is set: {output}"
    );

    Ok(())
}

#[test]
fn test_r11_zip_matched_by_hash_across_all_release_rows_not_filename() {
    let tmp = TempDir::new().unwrap();
    // Filename says 1999-01-01, but the index row has 2026-07-31 for this hash
    let zip_path = create_mock_trud_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_19990101000001.zip");
    let sha256 = compute_file_sha256(&zip_path).unwrap();
    let file_size = fs::metadata(&zip_path).unwrap().len();

    let index = make_v1_index(&[(
        "2026-07-31",
        &sha256,
        file_size,
        &[("1.0.1", "sha256:0000000000000000000000000000000000000000000000000000000000000000")],
    )]);
    let index_file = tmp.path().join("index.json");
    fs::write(&index_file, serde_json::to_string_pretty(&index).unwrap()).unwrap();

    let out_dir = tmp.path().join("out");
    let output = ods_binary()
        .arg("make")
        .arg("-i")
        .arg(&zip_path)
        .arg("-o")
        .arg(&out_dir)
        .arg("--index")
        .arg(&index_file)
        .output()
        .expect("execute ods make");

    assert!(output.status.success(), "ods make must succeed, stderr:\n{}", String::from_utf8_lossy(&output.stderr));
    let record = ods::provenance::read_release(&out_dir).unwrap();
    let facts = record.facts().expect("the index vouched for the zip, so the files carry provenance");
    assert_eq!(facts.release_date, "2026-07-31");
    assert_eq!(facts.source_sha256_upper(), sha256);
    assert_eq!(facts.source.bytes, file_size);
    assert_eq!(facts.license().name, ods::terms::LICENSE);
    assert_eq!(facts.license().attribution, ods::terms::ATTRIBUTION);
}

#[test]
fn test_r11_same_zip_renamed_archive_zip_matches_with_byte_identical_provenance() {
    let tmp = TempDir::new().unwrap();
    let zip_path = create_mock_trud_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_19990101000001.zip");
    let sha256 = compute_file_sha256(&zip_path).unwrap();
    let file_size = fs::metadata(&zip_path).unwrap().len();

    let index = make_v1_index(&[(
        "2026-07-31",
        &sha256,
        file_size,
        &[("1.0.1", "sha256:0000000000000000000000000000000000000000000000000000000000000000")],
    )]);
    let index_file = tmp.path().join("index.json");
    fs::write(&index_file, serde_json::to_string_pretty(&index).unwrap()).unwrap();

    let out1 = tmp.path().join("out1");
    let output1 = ods_binary()
        .arg("make")
        .arg("-i")
        .arg(&zip_path)
        .arg("-o")
        .arg(&out1)
        .arg("--index")
        .arg(&index_file)
        .output()
        .expect("execute ods make");
    assert!(output1.status.success());

    // Rename to archive.zip (not a TRUD pattern)
    let renamed_zip = tmp.path().join("archive.zip");
    fs::copy(&zip_path, &renamed_zip).unwrap();

    let out2 = tmp.path().join("out2");
    let output2 = ods_binary()
        .arg("make")
        .arg("-i")
        .arg(&renamed_zip)
        .arg("-o")
        .arg(&out2)
        .arg("--index")
        .arg(&index_file)
        .output()
        .expect("execute ods make");
    assert!(output2.status.success(), "ods make on archive.zip must succeed, stderr:\n{}", String::from_utf8_lossy(&output2.stderr));

    assert_eq!(embedded_of(&out1), embedded_of(&out2), "the embedded object must be identical for a renamed archive");
}

fn serve_mock_trud(key: &str, date: &str, zip_name: &str, zip_sha256: &str, zip_size: u64) -> String {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let content = format!("/keys/{key}/content/items/341");
    let url = |name: &str| format!("{base}{content}/{name}");
    let mut routes: std::collections::HashMap<String, Vec<u8>> = std::collections::HashMap::new();

    let release_item = serde_json::json!({
        "id": zip_name,
        "name": format!("Release {}", date),
        "releaseDate": date,
        "archiveFileUrl": url(zip_name),
        "archiveFileName": zip_name,
        "archiveFileSizeBytes": zip_size,
        "archiveFileSha256": zip_sha256,
        "checksumFileUrl": url("trud_x.xml"),
        "checksumFileName": "trud_x.xml",
        "signatureFileUrl": url("trud_x.xml.asc"),
        "signatureFileName": "trud_x.xml.sig",
        "publicKeyFileUrl": url("trud-public-key.pgp"),
        "publicKeyFileName": "trud-public-key.pgp",
    });
    let listing = serde_json::json!({ "apiVersion": "1", "releases": [release_item] });
    routes.insert(format!("/keys/{key}/items/341/releases"), listing.to_string().into_bytes());
    for name in ["trud_x.xml", "trud_x.xml.asc", "trud-public-key.pgp"] {
        routes.insert(format!("{content}/{name}"), b"NHS's bytes".to_vec());
    }

    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            let path = request.split_whitespace().nth(1).unwrap_or("").to_string();
            let clean_path = path.split('?').next().unwrap_or(&path);
            match routes.get(clean_path) {
                Some(body) => {
                    let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                    let _ = stream.write_all(head.as_bytes());
                    let _ = stream.write_all(body);
                }
                None => {
                    let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                }
            }
        }
    });
    base
}

#[test]
fn test_r11_provenance_byte_identical_across_all_four_paths() {
    let tmp = TempDir::new().unwrap();
    let zip_name = "hscorgrefdataxml_data_7.0.0_20260731000001.zip";
    let zip_path = create_mock_trud_zip(tmp.path(), zip_name);
    let sha256 = compute_file_sha256(&zip_path).unwrap();
    let file_size = fs::metadata(&zip_path).unwrap().len();

    let index = make_v1_index(&[(
        "2026-07-31",
        &sha256,
        file_size,
        &[("1.0.1", "sha256:0000000000000000000000000000000000000000000000000000000000000000")],
    )]);
    let index_file = tmp.path().join("index.json");
    fs::write(&index_file, serde_json::to_string_pretty(&index).unwrap()).unwrap();

    let trud_base = serve_mock_trud("testkey", "2026-07-31", zip_name, &sha256, file_size);

    // Route 1: ods make <zip> -o <dir>
    let make_out = tmp.path().join("make_out");
    let out1 = ods_binary()
        .arg("make")
        .arg("-i")
        .arg(&zip_path)
        .arg("-o")
        .arg(&make_out)
        .arg("--index")
        .arg(&index_file)
        .output()
        .unwrap();
    assert!(out1.status.success(), "make failed: {}", String::from_utf8_lossy(&out1.stderr));
    let prov_make = embedded_of(&make_out);

    // Route 2: ods trud pull --local-archive <zip> into workspace, then ods make
    let ws2 = tmp.path().join("ws2");
    fs::create_dir_all(&ws2).unwrap();
    // Builds the release a pull left in `ws2`, and returns the object its files carry.
    let build_ws2 = |label: &str| -> String {
        let release_dir = ws2.join("releases/2026-07-31");
        let out = ods_binary()
            .arg("make")
            .arg("-i")
            .arg(&release_dir)
            .arg("-o")
            .arg(&release_dir)
            .arg("--index")
            .arg(&index_file)
            .output()
            .unwrap();
        assert!(out.status.success(), "make after {label} failed: {}", String::from_utf8_lossy(&out.stderr));
        embedded_of(&release_dir)
    };

    let out2 = ods_binary()
        .arg("trud")
        .arg("pull")
        .arg("--local-archive")
        .arg(&zip_path)
        .arg("-w")
        .arg(&ws2)
        .arg("--index")
        .arg(&index_file)
        .output()
        .unwrap();
    assert!(out2.status.success(), "local-archive pull failed: {}", String::from_utf8_lossy(&out2.stderr));
    let prov_local = build_ws2("the local-archive pull");

    // Route 3: heal_release_dir's refresh via ods trud pull --force, then ods make
    let out3 = ods_binary()
        .arg("trud")
        .arg("pull")
        .arg("2026-07-31")
        .arg("--force")
        .arg("-w")
        .arg(&ws2)
        .arg("--index")
        .arg(&index_file)
        .env("TRUD_API_KEY", "testkey")
        .env("ODS_TRUD_API_URL", &trud_base)
        .output()
        .unwrap();
    assert!(out3.status.success(), "force pull failed: {}", String::from_utf8_lossy(&out3.stderr));
    let record: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(ws2.join("releases/2026-07-31").join("trud").join(TRUD_ARCHIVE_PACKAGE_FILENAME)).unwrap(),
    )
    .unwrap();
    let names: Vec<&str> = record["resources"].as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["archive", "checksum", "signature", "key"], "the TRUD API pull records all four files");
    let prov_force = build_ws2("the force refresh");

    // Route 4: the TRUD archive package's derivation, directly
    let prov_direct = ods::provenance::TrudArchivePackage::for_trud_release("2026-07-31", zip_name, &sha256, file_size, &[])
        .unwrap()
        .embedded(ods::datapackage::DATASET_VERSION)
        .unwrap()
        .to_compact_json()
        .unwrap();

    assert_eq!(prov_make, prov_local, "make and local-archive provenance must be byte-identical");
    assert_eq!(prov_local, prov_force, "local-archive and force refresh provenance must be byte-identical");
    assert_eq!(prov_force, prov_direct, "force refresh and the direct derivation must be byte-identical");
}

#[test]
fn test_make_release_refuses_unprovenanced_build() {
    let tmp = TempDir::new().unwrap();
    let zip_path = create_mock_trud_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_20260731000001.zip");
    let out_dir = tmp.path().join("unprovenanced");

    // Make unmatched without provenance
    let make_out = ods_binary()
        .arg("make")
        .arg("-i")
        .arg(&zip_path)
        .arg("-o")
        .arg(&out_dir)
        .arg("--force")
        .output()
        .unwrap();
    assert!(make_out.status.success());
    assert!(!out_dir.join("trud").exists());

    // Try ods make release -i <dir>
    let rel_out = ods_binary()
        .arg("make")
        .arg("release")
        .arg("-i")
        .arg(&out_dir)
        .output()
        .unwrap();
    assert!(!rel_out.status.success(), "make release must refuse unprovenanced release");
    let stderr = String::from_utf8_lossy(&rel_out.stderr);
    assert!(stderr.contains("carry no provenance"), "stderr was: {stderr}");
}

#[test]
fn test_trud_audit_refuses_unprovenanced_release() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("workspace");
    let releases_dir = ws.join("releases");
    let rel_2026 = releases_dir.join("2026-07-31");
    fs::create_dir_all(&rel_2026).unwrap();

    // A Parquet file built without provenance, and a releases.json marker so workspace is valid
    ods::commands::parquet::write_stub_parquet(
        &rel_2026.join("orgs.parquet"),
        None,
        "dummy parquet",
    )
    .unwrap();
    let index = make_v1_index(&[]);
    fs::write(ws.join("_releases.json"), serde_json::to_string_pretty(&index).unwrap()).unwrap();
    // Set active release symlink
    #[cfg(unix)]
    std::os::unix::fs::symlink("2026-07-31", releases_dir.join("current")).unwrap();

    // Active release has orgs.parquet but no provenance in it, and no TRUD archive package
    assert!(!rel_2026.join("trud").join(TRUD_ARCHIVE_PACKAGE_FILENAME).exists());

    // An input zip with XML
    let zip_path = create_mock_trud_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_20260731000001.zip");

    let output = ods_binary()
        .arg("trud")
        .arg("audit")
        .arg("-i")
        .arg(&zip_path)
        .arg("-w")
        .arg(&ws)
        .output()
        .unwrap();

    assert!(!output.status.success(), "trud audit must refuse release without provenance");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("carry no provenance: it was built from an archive ods couldn't match to a TRUD release, or by an older ods"), "stderr was: {stderr}");
    assert!(stderr.contains("To cite or publish it, get the archive through ods trud pull"), "stderr was: {stderr}");
}

#[test]
fn test_stale_terms_repair_with_force_pull_enables_make() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ws");
    fs::create_dir_all(&ws).unwrap();
    let zip_name = "hscorgrefdataxml_data_7.0.0_20260731000001.zip";
    let zip_path = create_mock_trud_zip(tmp.path(), zip_name);
    let sha256 = compute_file_sha256(&zip_path).unwrap();
    let file_size = fs::metadata(&zip_path).unwrap().len();

    let index = make_v1_index(&[(
        "2026-07-31",
        &sha256,
        file_size,
        &[("1.0.1", "sha256:0000000000000000000000000000000000000000000000000000000000000000")],
    )]);
    let index_file = tmp.path().join("index.json");
    fs::write(&index_file, serde_json::to_string_pretty(&index).unwrap()).unwrap();

    let trud_base = serve_mock_trud("testkey", "2026-07-31", zip_name, &sha256, file_size);

    // Pull into workspace
    let pull_out = ods_binary()
        .arg("trud")
        .arg("pull")
        .arg("--local-archive")
        .arg(&zip_path)
        .arg("-w")
        .arg(&ws)
        .arg("--index")
        .arg(&index_file)
        .output()
        .unwrap();
    assert!(pull_out.status.success(), "local-archive pull failed: {}", String::from_utf8_lossy(&pull_out.stderr));

    let release_dir = ws.join("releases/2026-07-31");
    let prov_file = release_dir.join("trud").join(TRUD_ARCHIVE_PACKAGE_FILENAME);

    // Tamper with terms to simulate stale terms
    let mut prov: ods::provenance::TrudArchivePackage = serde_json::from_str(&fs::read_to_string(&prov_file).unwrap()).unwrap();
    prov.licenses[0].name = "Old Stale Licence".to_string();
    fs::write(&prov_file, prov.to_json_string().unwrap()).unwrap();

    // ods make must refuse
    let make_fail = ods_binary()
        .current_dir(&ws)
        .arg("make")
        .arg("-i")
        .arg(&release_dir)
        .output()
        .unwrap();
    assert!(!make_fail.status.success(), "make must refuse stale terms");
    let stderr = String::from_utf8_lossy(&make_fail.stderr);
    assert!(stderr.contains("has older licence terms than this ods"), "stderr was: {stderr}");
    assert!(stderr.contains("Refresh it without downloading the archive: ods trud pull 2026-07-31 --force"), "stderr was: {stderr}");

    // Repair with ods trud pull 2026-07-31 --force
    let force_out = ods_binary()
        .arg("trud")
        .arg("pull")
        .arg("2026-07-31")
        .arg("--force")
        .arg("-w")
        .arg(&ws)
        .arg("--index")
        .arg(&index_file)
        .env("TRUD_API_KEY", "testkey")
        .env("ODS_TRUD_API_URL", &trud_base)
        .output()
        .unwrap();
    assert!(force_out.status.success(), "force pull must succeed: {}", String::from_utf8_lossy(&force_out.stderr));

    // Provenance now has current terms
    let refreshed_prov: ods::provenance::TrudArchivePackage = serde_json::from_str(&fs::read_to_string(&prov_file).unwrap()).unwrap();
    assert!(refreshed_prov.has_current_terms());

    // ods make now succeeds!
    let make_ok = ods_binary()
        .current_dir(&ws)
        .arg("make")
        .arg("-i")
        .arg(&release_dir)
        .output()
        .unwrap();
    assert!(make_ok.status.success(), "make must succeed after force refresh, stderr:\n{}", String::from_utf8_lossy(&make_ok.stderr));
}


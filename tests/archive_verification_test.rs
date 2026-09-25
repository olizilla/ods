mod common;

use anyhow::Result;
use common::{create_mock_trud_zip, make_v1_index, ods_binary};
use ods::commands::fetch::{
    run_local_archive_with_fetchers, Args as FetchArgs, TrudFetcher, TrudReleaseItem,
};
use ods::progress::{Progress, ProgressCaps};
use ods::provenance::{compute_file_sha256, PROVENANCE_FILENAME};
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
        .output()
        .expect("execute ods make");

    assert!(output.status.success(), "ods make must succeed, stderr:\n{}", String::from_utf8_lossy(&output.stderr));
    let stderr = String::from_utf8_lossy(&output.stderr);
    let expected_warning = format!("! No provenance info found for {}. Source is unverified.", zip_path.display());
    assert!(stderr.contains(&expected_warning), "stderr must report unverified: {}", stderr);

    let prov_file = out_dir.join(PROVENANCE_FILENAME);
    assert!(prov_file.exists());
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

    let prov_file = ws.join("releases").join("2026-07-31").join(PROVENANCE_FILENAME);
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

    let prov_file = out_dir.join(PROVENANCE_FILENAME);
    assert!(prov_file.exists());
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

    let prov_file = ws.join("releases").join("2026-07-31").join(PROVENANCE_FILENAME);
    assert!(prov_file.exists());
}

#[test]
fn test_trud_pull_local_archive_hash_mismatch_exits_1_and_leaves_workspace_untouched() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    let releases_dir = ws.join("releases");
    fs::create_dir_all(&releases_dir).unwrap();

    let zip_path = create_mock_trud_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_20260731000001.zip");
    let actual_sha = compute_file_sha256(&zip_path).unwrap();

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
    assert!(stderr.contains("✖ SHA-256 Checksum Failed!"), "stderr:\n{}", stderr);
    assert!(stderr.contains(&format!("Local SHA-256: {}", actual_sha)), "stderr:\n{}", stderr);
    assert!(stderr.contains(&format!("Index SHA-256: {}", bad_sha)), "stderr:\n{}", stderr);

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

    assert!(res.is_ok(), "must exit 0 on failing TRUD API during --local-archive");

    let output = String::from_utf8(buffer.lock().unwrap().clone())?;
    assert!(
        output.contains("! TRUD API check failed: connection refused (os error 111)"),
        "output was: {output}"
    );
    assert!(
        !output.contains("Set TRUD_API_KEY to check it against TRUD"),
        "output should not tell user to set key when key is set: {output}"
    );

    let prov_file = ws.join("releases").join("2026-07-31").join(PROVENANCE_FILENAME);
    assert!(prov_file.exists());

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


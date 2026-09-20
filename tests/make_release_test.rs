use anyhow::Result;
use ods::commands::make_release::{perform_all_release_checks, run, Args};
use ods::provenance::{compute_file_sha256, OdsProvenance, PROVENANCE_FILENAME};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

mod common;
use common::setup_synthetic_repo_and_release;

#[test]
fn test_make_release_success_appends_to_releases_json() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();

    run(Args {
        input: Some(rel_dir.clone()),
        repository: "ods-data".to_string(),
        output: None,
        doi: Some("10.5281/zenodo.12345".to_string()),
        tool_repo: Some(tmp.path().to_path_buf()),
        index: None,
    })?;

    let index_file = tmp.path().join("data").join("releases.json");
    let content = fs::read_to_string(&index_file)?;
    let index: ods::index::OdsReleaseIndex = serde_json::from_str(&content)?;

    let expected_ver = ods::datapackage::dataset_version();
    assert_eq!(index.releases.len(), 1);
    assert_eq!(index.releases[0].trud_release_date, "2026-07-31");
    assert_eq!(index.releases[0].datasets.len(), 1);
    assert_eq!(index.releases[0].datasets[0].dataset_version, expected_ver);
    assert_eq!(index.releases[0].datasets[0].dataset_doi, Some("10.5281/zenodo.12345".to_string()));
    assert!(index.releases[0].datasets[0].manifest_digest.starts_with("sha256:"));

    Ok(())
}

#[test]
fn test_make_release_fails_on_dirty_working_tree() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();

    let prov_path = rel_dir.join(PROVENANCE_FILENAME);
    let mut prov: OdsProvenance = serde_json::from_str(&fs::read_to_string(&prov_path)?)?;
    prov.tool_git_dirty = Some(true);
    fs::write(&prov_path, serde_json::to_string_pretty(&prov)?)?;

    let failures = perform_all_release_checks(
        &rel_dir,
        ods::datapackage::dataset_version(),
        Some(tmp.path()),
        None,
    )?;

    assert!(failures.iter().any(|f| f.contains("tool_git_dirty is true")));
    Ok(())
}

#[test]
fn test_make_release_fails_on_tool_git_sha_mismatch() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();

    let prov_path = rel_dir.join(PROVENANCE_FILENAME);
    let mut prov: OdsProvenance = serde_json::from_str(&fs::read_to_string(&prov_path)?)?;
    prov.tool_git_sha = Some("0000000000000000000000000000000000000000".to_string());
    fs::write(&prov_path, serde_json::to_string_pretty(&prov)?)?;

    let failures = perform_all_release_checks(
        &rel_dir,
        ods::datapackage::dataset_version(),
        Some(tmp.path()),
        None,
    )?;

    assert!(failures.iter().any(|f| f.contains("tool_git_sha")));
    Ok(())
}

#[test]
fn test_make_release_fails_on_duplicate_row_in_index() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();

    let ver = ods::datapackage::dataset_version();
    let mut index = ods::index::OdsReleaseIndex::default();
    index.releases.push(ods::index::Release {
        trud_release_date: "2026-07-31".to_string(),
        trud_release_sha256: "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string(),
        trud_release_filesize_bytes: 37983173,
        datasets: vec![ods::index::Dataset {
            dataset_version: ver.to_string(),
            manifest_digest: "sha256:0f2a000000000000000000000000000000000000000000000000000000000000".to_string(),
            dataset_doi: None,
            withdrawn: None,
        }],
    });

    let failures = perform_all_release_checks(
        &rel_dir,
        ver,
        Some(tmp.path()),
        Some(&index),
    )?;

    let expected_msg = format!("data/releases.json already has a dataset for 2026-07-31 {}", ver);
    assert!(failures.iter().any(|f| f.contains(&expected_msg)));
    Ok(())
}

#[test]
fn test_make_release_creates_dist_staging_tree_with_real_files() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();
    let dist_dir = tmp.path().join("dist");

    run(Args {
        input: Some(rel_dir.clone()),
        repository: "ods-data".to_string(),
        output: Some(dist_dir.clone()),
        doi: Some("10.5281/zenodo.12345".to_string()),
        tool_repo: Some(tmp.path().to_path_buf()),
        index: None,
    })?;

    assert!(dist_dir.exists(), "dist/ directory must exist");
    assert!(!dist_dir.join("releases.json").exists(), "dist/releases.json must NOT exist in dist/");

    let ver = ods::datapackage::dataset_version();
    let manifests_dir = dist_dir.join("v2").join("ods-data").join("manifests");
    assert!(manifests_dir.join(format!("2026-07-31_{}", ver)).exists());
    assert!(manifests_dir.join("2026-07-31").exists());
    assert!(manifests_dir.join("latest").exists());

    // Check that duplicate trees are NOT written to dist/
    assert!(!dist_dir.join("latest").exists(), "dist/latest must NOT exist in dist/");
    assert!(!dist_dir.join("2026-07-31").exists(), "dist/2026-07-31 must NOT exist in dist/");

    // Check blobs
    let blobs_dir = dist_dir.join("v2").join("ods-data").join("blobs").join("sha256");
    assert!(blobs_dir.exists());
    let mut blob_count = 0;
    for entry in fs::read_dir(&blobs_dir)? {
        let entry = entry?;
        assert!(!entry.path().is_symlink(), "dist blob {} must be a real file, not a symlink", entry.path().display());
        blob_count += 1;
    }
    assert_eq!(blob_count, 6, "Must contain 1 manifest blob + 5 layer blobs");

    // Two-sided contract check: assert dist/ exactly matches expected-keys.json
    let mut actual_dist_keys = Vec::new();
    fn walk_dist(base: &Path, current: &Path, paths: &mut Vec<String>) {
        if current.is_dir() {
            if let Ok(entries) = fs::read_dir(current) {
                for entry in entries.flatten() {
                    walk_dist(base, &entry.path(), paths);
                }
            }
        } else if current.is_file() {
            if let Ok(rel) = current.strip_prefix(base) {
                paths.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    walk_dist(&dist_dir, &dist_dir, &mut actual_dist_keys);
    actual_dist_keys.sort();

    let expected_keys_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("worker")
        .join("test")
        .join("fixtures")
        .join("expected-keys.json");

    if std::env::var("UPDATE_EXPECTED_KEYS").is_ok() || std::env::var("UPDATE_FIXTURES").is_ok() {
        let json = serde_json::to_string_pretty(&actual_dist_keys)?;
        fs::write(&expected_keys_path, format!("{}\n", json))?;
    }

    let expected_keys_content = fs::read_to_string(&expected_keys_path)
        .expect("worker/test/fixtures/expected-keys.json must exist");
    let mut expected_keys: Vec<String> = serde_json::from_str(&expected_keys_content)
        .expect("expected-keys.json must be valid JSON array of strings");
    expected_keys.sort();

    assert_eq!(
        actual_dist_keys,
        expected_keys,
        "dist/ keys generated by ods make release must exactly match worker/test/fixtures/expected-keys.json"
    );

    Ok(())
}

#[test]
fn test_make_release_refuses_when_no_repo_found() {
    let (_tmp, rel_dir) = setup_synthetic_repo_and_release();
    let not_repo_tmp = TempDir::new().unwrap();
    let not_a_repo = not_repo_tmp.path().join("not-a-repo");
    fs::create_dir_all(&not_a_repo).unwrap();

    // 1. Direct check in perform_all_release_checks
    let failures = perform_all_release_checks(
        &rel_dir,
        ods::datapackage::dataset_version(),
        None,
        None,
    ).unwrap();
    assert!(failures.iter().any(|f| f.contains("cannot locate the ods repository")));

    // 2. Full run refusal
    let res = run(Args {
        input: Some(rel_dir),
        repository: "ods-data".to_string(),
        output: None,
        doi: None,
        tool_repo: Some(not_a_repo),
        index: None,
    });

    assert!(res.is_err(), "Must refuse when tool_repo is not a repo");
    let err = format!("{:#}", res.unwrap_err());
    assert!(err.contains("cannot locate the ods repository"));
}

#[test]
fn test_make_release_refuses_unverified_provenance() -> Result<()> {
    let (_tmp, rel_dir) = setup_synthetic_repo_and_release();

    // Mutate provenance to Unverified
    let prov_path = rel_dir.join(PROVENANCE_FILENAME);
    let mut prov: OdsProvenance = serde_json::from_str(&fs::read_to_string(&prov_path)?)?;
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::Unverified);
    fs::write(&prov_path, serde_json::to_string_pretty(&prov)?)?;

    let failures = perform_all_release_checks(
        &rel_dir,
        ods::datapackage::dataset_version(),
        None,
        None,
    )?;

    assert!(
        failures.iter().any(|f| f.contains("trud_release_sha256_verified")),
        "Must refuse unverified provenance, got: {:?}",
        failures
    );

    Ok(())
}

#[test]
fn test_make_release_refuses_implausible_filesize() -> Result<()> {
    let (_tmp, rel_dir) = setup_synthetic_repo_and_release();

    // Mutate provenance to 16 bytes
    let prov_path = rel_dir.join(PROVENANCE_FILENAME);
    let mut prov: OdsProvenance = serde_json::from_str(&fs::read_to_string(&prov_path)?)?;
    prov.trud_release_filesize_bytes = Some(16);
    fs::write(&prov_path, serde_json::to_string_pretty(&prov)?)?;

    let failures = perform_all_release_checks(
        &rel_dir,
        ods::datapackage::dataset_version(),
        None,
        None,
    )?;

    assert!(
        failures.iter().any(|f| f.contains("trud_release_filesize_bytes is implausibly small")),
        "Must refuse implausible filesize, got: {:?}",
        failures
    );

    Ok(())
}

#[test]
fn test_make_release_fails_on_dataset_version_mismatch_with_tool() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();

    // Mutate datapackage.json to 1.0.0, differing from tool constant 0.1.0
    let dp_path = rel_dir.join("datapackage.json");
    let mut dp: serde_json::Value = serde_json::from_str(&fs::read_to_string(&dp_path)?)?;
    dp["version"] = serde_json::json!("1.0.0");
    fs::write(&dp_path, serde_json::to_string_pretty(&dp)?)?;

    let failures = perform_all_release_checks(
        &rel_dir,
        "1.0.0",
        Some(tmp.path()),
        None,
    )?;

    let expected_ver = ods::datapackage::dataset_version();
    let mismatch_failure = failures
        .iter()
        .find(|f| f.contains("datapackage.json version"))
        .expect("must have dataset_version mismatch failure");
    assert!(mismatch_failure.contains("1.0.0"));
    assert!(mismatch_failure.contains(expected_ver));
    assert!(mismatch_failure.contains("The release was compiled by an older tool. Re-run `ods make`"));

    // Full command execution fails
    let res = run(Args {
        input: Some(rel_dir),
        repository: "ods-data".to_string(),
        output: None,
        doi: None,
        tool_repo: Some(tmp.path().to_path_buf()),
        index: None,
    });
    assert!(res.is_err());
    let err = format!("{:#}", res.unwrap_err());
    assert!(err.contains("datapackage.json version (1.0.0) does not match this build of ods"));

    Ok(())
}

#[test]
fn test_make_release_refuses_missing_dataset_version() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();

    // Mutate datapackage.json to remove version
    let dp_path = rel_dir.join("datapackage.json");
    let mut dp: serde_json::Value = serde_json::from_str(&fs::read_to_string(&dp_path)?)?;
    dp.as_object_mut().unwrap().remove("version");
    fs::write(&dp_path, serde_json::to_string_pretty(&dp)?)?;

    let res = run(Args {
        input: Some(rel_dir),
        repository: "ods-data".to_string(),
        output: None,
        doi: None,
        tool_repo: Some(tmp.path().to_path_buf()),
        index: None,
    });
    assert!(res.is_err());
    let err = format!("{:#}", res.unwrap_err());
    assert!(err.contains("Missing version in datapackage.json"));
    assert!(err.contains("ods make"));

    Ok(())
}

#[test]
fn test_make_release_refuses_non_semver_dataset_version() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();

    let dp_path = rel_dir.join("datapackage.json");
    let mut dp: serde_json::Value = serde_json::from_str(&fs::read_to_string(&dp_path)?)?;
    dp["version"] = serde_json::json!("invalid-semver");
    fs::write(&dp_path, serde_json::to_string_pretty(&dp)?)?;

    let failures = perform_all_release_checks(
        &rel_dir,
        "invalid-semver",
        Some(tmp.path()),
        None,
    )?;

    assert!(failures.iter().any(|f| f.contains("is not valid SemVer")));
    Ok(())
}

#[test]
fn test_make_release_checks_before_writing_dirty_tree() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();
    // Remove oci directory to verify make_release doesn't write it if checks fail
    let oci_dir = rel_dir.join("oci");
    if oci_dir.exists() {
        fs::remove_dir_all(&oci_dir)?;
    }

    // Set tool_git_dirty to true
    let prov_path = rel_dir.join(PROVENANCE_FILENAME);
    let mut prov: OdsProvenance = serde_json::from_str(&fs::read_to_string(&prov_path)?)?;
    prov.tool_git_dirty = Some(true);
    fs::write(&prov_path, serde_json::to_string_pretty(&prov)?)?;

    // Record release directory entries and mtimes
    let before_files: Vec<(PathBuf, std::time::SystemTime)> = fs::read_dir(&rel_dir)?
        .filter_map(|e| e.ok())
        .map(|e| (e.path(), e.metadata().unwrap().modified().unwrap()))
        .collect();

    let dist_dir = tmp.path().join("dist");
    let res = run(Args {
        input: Some(rel_dir.clone()),
        repository: "ods-data".to_string(),
        output: Some(dist_dir.clone()),
        doi: None,
        tool_repo: Some(tmp.path().to_path_buf()),
        index: None,
    });
    assert!(res.is_err(), "make release must fail when working tree is dirty");
    let err = format!("{:#}", res.unwrap_err());
    assert!(err.contains("tool_git_dirty is true"));
    assert!(!oci_dir.exists(), "oci/ must not be written when checks fail");
    assert!(!dist_dir.exists(), "dist/ must not be written when checks fail");

    let after_files: Vec<(PathBuf, std::time::SystemTime)> = fs::read_dir(&rel_dir)?
        .filter_map(|e| e.ok())
        .map(|e| (e.path(), e.metadata().unwrap().modified().unwrap()))
        .collect();
    assert_eq!(before_files, after_files, "release files and mtimes must be untouched");

    Ok(())
}

#[test]
fn test_make_release_missing_index_writes_nothing() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();
    let oci_dir = rel_dir.join("oci");
    if oci_dir.exists() {
        fs::remove_dir_all(&oci_dir)?;
    }

    // Record release directory entries and mtimes before running
    let before_files: Vec<(PathBuf, std::time::SystemTime)> = fs::read_dir(&rel_dir)?
        .filter_map(|e| e.ok())
        .map(|e| (e.path(), e.metadata().unwrap().modified().unwrap()))
        .collect();

    let non_existent_index = tmp.path().join("does_not_exist_releases.json");
    let dist_dir = tmp.path().join("dist");

    // Test programmatic run()
    let res = run(Args {
        input: Some(rel_dir.clone()),
        repository: "ods-data".to_string(),
        output: Some(dist_dir.clone()),
        doi: None,
        tool_repo: Some(tmp.path().to_path_buf()),
        index: Some(non_existent_index.clone()),
    });
    assert!(res.is_err(), "make release must fail when index cannot be read");
    let err = res.unwrap_err();
    assert!(
        err.downcast_ref::<ods::commands::pull::AlreadyReported>().is_some(),
        "error must be AlreadyReported so main exits cleanly with code 1"
    );
    assert!(!oci_dir.exists(), "oci/ must not be written when index cannot be read");
    assert!(!dist_dir.exists(), "dist/ must not be written when index cannot be read");

    let after_files: Vec<(PathBuf, std::time::SystemTime)> = fs::read_dir(&rel_dir)?
        .filter_map(|e| e.ok())
        .map(|e| (e.path(), e.metadata().unwrap().modified().unwrap()))
        .collect();
    assert_eq!(before_files, after_files, "release files and mtimes must be untouched");

    // Also verify via CLI binary: exit 1, stderr names the path
    let cli_out = Command::new(env!("CARGO_BIN_EXE_ods"))
        .arg("make")
        .arg("release")
        .arg("--input")
        .arg(&rel_dir)
        .arg("--output")
        .arg(&dist_dir)
        .arg("--index")
        .arg(&non_existent_index)
        .output()?;
    assert_eq!(cli_out.status.code(), Some(1), "CLI exit status must be 1");
    let stderr = String::from_utf8_lossy(&cli_out.stderr);
    assert!(
        stderr.contains("Cannot read release index"),
        "stderr must contain 'Cannot read release index', got:\n{}",
        stderr
    );
    assert!(
        stderr.contains("does_not_exist_releases.json"),
        "stderr must contain index path, got:\n{}",
        stderr
    );
    assert!(!dist_dir.exists(), "dist/ must not exist after CLI run");
    assert!(!oci_dir.exists(), "oci/ must not exist after CLI run");

    Ok(())
}

#[test]
fn test_make_release_staging_with_relative_input() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();
    let dist_relative = tmp.path().join("dist-relative");
    let dist_absolute = tmp.path().join("dist-absolute");

    let index_src = tmp.path().join("data").join("releases.json");
    let index_a = tmp.path().join("releases-a.json");
    let index_b = tmp.path().join("releases-b.json");
    fs::copy(&index_src, &index_a)?;
    fs::copy(&index_src, &index_b)?;

    // 1. Run ods make release with relative --input and .current_dir(tmp.path())
    let ods_bin = env!("CARGO_BIN_EXE_ods");
    let rel_status = Command::new(ods_bin)
        .current_dir(tmp.path())
        .args([
            "make",
            "release",
            "--input",
            "releases/2026-07-31",
            "--tool-repo",
            tmp.path().to_str().unwrap(),
            "--index",
            index_a.to_str().unwrap(),
            "--output",
            dist_relative.to_str().unwrap(),
        ])
        .status()?;
    assert!(rel_status.success(), "make release with relative input failed");

    // 2. Run ods make release with absolute --input
    let abs_status = Command::new(ods_bin)
        .current_dir(tmp.path())
        .args([
            "make",
            "release",
            "--input",
            rel_dir.to_str().unwrap(),
            "--tool-repo",
            tmp.path().to_str().unwrap(),
            "--index",
            index_b.to_str().unwrap(),
            "--output",
            dist_absolute.to_str().unwrap(),
        ])
        .status()?;
    assert!(abs_status.success(), "make release with absolute input failed");

    // 3. Assert both staging trees hold the exact same relative file paths
    let collect_files = |base: &Path| -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = walkdir::WalkDir::new(base)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .map(|e| e.path().strip_prefix(base).unwrap().to_path_buf())
            .collect();
        files.sort();
        files
    };

    let rel_files = collect_files(&dist_relative);
    let abs_files = collect_files(&dist_absolute);
    assert!(!rel_files.is_empty(), "staging tree must not be empty");
    assert_eq!(rel_files, abs_files, "relative and absolute staging trees must hold identical files");

    // 4. Assert every blob's SHA-256 equals its filename
    let blobs_dir = dist_relative.join("v2").join("ods-data").join("blobs").join("sha256");
    for entry in fs::read_dir(&blobs_dir)? {
        let entry = entry?;
        let filename = entry.file_name().to_string_lossy().to_string();
        let computed = compute_file_sha256(&entry.path())?.to_lowercase();
        assert_eq!(filename, computed, "blob content must match its sha256 filename");
    }

    Ok(())
}


#[test]
fn test_make_release_staged_blobs_are_copies_not_hardlinks() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();
    let dist_dir = tmp.path().join("dist");

    run(Args {
        input: Some(rel_dir.clone()),
        repository: "ods-data".to_string(),
        output: Some(dist_dir.clone()),
        doi: None,
        tool_repo: Some(tmp.path().to_path_buf()),
        index: None,
    })?;

    // Read initial hash of _provenance.json in release directory
    let prov_path = rel_dir.join(PROVENANCE_FILENAME);
    let original_prov_hash = compute_file_sha256(&prov_path)?.to_lowercase();

    // Find the corresponding staged blob
    let staged_blob_path = dist_dir
        .join("v2")
        .join("ods-data")
        .join("blobs")
        .join("sha256")
        .join(&original_prov_hash);
    assert!(staged_blob_path.exists(), "staged blob for provenance must exist");

    // Mutate provenance file in place in release directory
    fs::write(&prov_path, b"mutated provenance content after release")?;

    // Verify the staged blob's hash still equals its original filename
    let current_staged_hash = compute_file_sha256(&staged_blob_path)?.to_lowercase();
    assert_eq!(
        current_staged_hash, original_prov_hash,
        "staged blob must not mutate when source file is modified (it must be a genuine copy, not hard link)"
    );

    Ok(())
}


#[test]
fn test_make_release_staging_failure_leaves_index_identical() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();
    let index_path = tmp.path().join("data").join("releases.json");
    let original_index_bytes = fs::read(&index_path)?;

    // Make output directory uncreatable / read-only
    let read_only_parent = tmp.path().join("readonly_dir");
    fs::create_dir_all(&read_only_parent)?;
    let dist_dir = read_only_parent.join("dist");

    // Set permissions of read_only_parent to 0o444
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&read_only_parent, fs::Permissions::from_mode(0o444))?;
    }

    let res = run(Args {
        input: Some(rel_dir),
        repository: "ods-data".to_string(),
        output: Some(dist_dir),
        doi: None,
        tool_repo: Some(tmp.path().to_path_buf()),
        index: None,
    });

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&read_only_parent, fs::Permissions::from_mode(0o755));
    }

    assert!(res.is_err(), "staging failure must return error");
    let current_index_bytes = fs::read(&index_path)?;
    assert_eq!(
        original_index_bytes, current_index_bytes,
        "index must be byte-identical if staging fails"
    );

    Ok(())
}

#[test]
fn test_make_release_response_listing_three_dates_adds_all_three() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();

    // Write trud/trud-releases-2026-07-31.json with 3 releases
    let trud_dir = rel_dir.join("trud");
    fs::create_dir_all(&trud_dir)?;
    let prov: OdsProvenance = serde_json::from_str(&fs::read_to_string(rel_dir.join(PROVENANCE_FILENAME))?)?;
    let zip_sha = prov.trud_release_sha256.unwrap();

    let response_json = serde_json::json!({
        "apiVersion": "1",
        "releases": [
            {
                "id": "item1.zip",
                "releaseDate": "2026-07-31",
                "archiveFileName": "hscorgrefdataxml_data_7.0.0_20260731000001.zip",
                "archiveFileSizeBytes": 37983173,
                "archiveFileSha256": zip_sha,
                "archiveFileUrl": "https://example.com/item1.zip"
            },
            {
                "id": "item2.zip",
                "releaseDate": "2026-06-26",
                "archiveFileName": "hscorgrefdataxml_data_6.0.0_20260626000001.zip",
                "archiveFileSizeBytes": 37957865,
                "archiveFileSha256": "712FE6C3810DC9C5BC6868F4F4038B0D8D8B92CC3EFD260A318811DEBFE04233",
                "archiveFileUrl": "https://example.com/item2.zip"
            },
            {
                "id": "item3.zip",
                "releaseDate": "2026-05-29",
                "archiveFileName": "hscorgrefdataxml_data_5.0.0_20260529000001.zip",
                "archiveFileSizeBytes": 37900000,
                "archiveFileSha256": "5555E6C3810DC9C5BC6868F4F4038B0D8D8B92CC3EFD260A318811DEBFE04233",
                "archiveFileUrl": "https://example.com/item3.zip"
            }
        ]
    });
    fs::write(trud_dir.join("trud-releases-2026-07-31.json"), serde_json::to_string_pretty(&response_json)?)?;

    run(Args {
        input: Some(rel_dir),
        repository: "ods-data".to_string(),
        output: None,
        doi: None,
        tool_repo: Some(tmp.path().to_path_buf()),
        index: None,
    })?;

    let index_file = tmp.path().join("data").join("releases.json");
    let content = fs::read_to_string(&index_file)?;
    let index: ods::index::OdsReleaseIndex = serde_json::from_str(&content)?;

    assert_eq!(index.releases.len(), 3);
    assert_eq!(index.releases[0].trud_release_date, "2026-07-31");
    assert_eq!(index.releases[0].datasets.len(), 1);
    assert_eq!(index.releases[1].trud_release_date, "2026-06-26");
    assert_eq!(index.releases[1].datasets.len(), 0);
    assert_eq!(index.releases[2].trud_release_date, "2026-05-29");
    assert_eq!(index.releases[2].datasets.len(), 0);

    Ok(())
}

#[test]
fn test_make_release_date_already_recorded_same_hash_left_byte_for_byte() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();

    let index_file = tmp.path().join("data").join("releases.json");
    let mut initial_index = ods::index::OdsReleaseIndex::default();
    initial_index.releases.push(ods::index::Release {
        trud_release_date: "2026-06-26".to_string(),
        trud_release_sha256: "712FE6C3810DC9C5BC6868F4F4038B0D8D8B92CC3EFD260A318811DEBFE04233".to_string(),
        trud_release_filesize_bytes: 37957865,
        datasets: vec![],
    });
    fs::write(&index_file, initial_index.to_json_pretty()?)?;

    let trud_dir = rel_dir.join("trud");
    fs::create_dir_all(&trud_dir)?;
    let prov: OdsProvenance = serde_json::from_str(&fs::read_to_string(rel_dir.join(PROVENANCE_FILENAME))?)?;
    let zip_sha = prov.trud_release_sha256.unwrap();

    let response_json = serde_json::json!({
        "apiVersion": "1",
        "releases": [
            {
                "id": "item1.zip",
                "releaseDate": "2026-07-31",
                "archiveFileName": "hscorgrefdataxml_data_7.0.0_20260731000001.zip",
                "archiveFileSizeBytes": 37983173,
                "archiveFileSha256": zip_sha,
                "archiveFileUrl": "https://example.com/item1.zip"
            },
            {
                "id": "item2.zip",
                "releaseDate": "2026-06-26",
                "archiveFileName": "hscorgrefdataxml_data_6.0.0_20260626000001.zip",
                "archiveFileSizeBytes": 37957865,
                "archiveFileSha256": "712FE6C3810DC9C5BC6868F4F4038B0D8D8B92CC3EFD260A318811DEBFE04233",
                "archiveFileUrl": "https://example.com/item2.zip"
            }
        ]
    });
    fs::write(trud_dir.join("trud-releases-2026-07-31.json"), serde_json::to_string_pretty(&response_json)?)?;

    run(Args {
        input: Some(rel_dir),
        repository: "ods-data".to_string(),
        output: None,
        doi: None,
        tool_repo: Some(tmp.path().to_path_buf()),
        index: None,
    })?;

    let content = fs::read_to_string(&index_file)?;
    let index: ods::index::OdsReleaseIndex = serde_json::from_str(&content)?;
    let rel_26 = index.releases.iter().find(|r| r.trud_release_date == "2026-06-26").unwrap();
    assert_eq!(rel_26.trud_release_sha256, "712FE6C3810DC9C5BC6868F4F4038B0D8D8B92CC3EFD260A318811DEBFE04233");
    assert_eq!(rel_26.trud_release_filesize_bytes, 37957865);
    assert!(rel_26.datasets.is_empty());

    Ok(())
}

#[test]
fn test_make_release_contradicting_hash_refuses_index_and_output_unchanged() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();

    let index_file = tmp.path().join("data").join("releases.json");
    let mut initial_index = ods::index::OdsReleaseIndex::default();
    initial_index.releases.push(ods::index::Release {
        trud_release_date: "2026-06-26".to_string(),
        trud_release_sha256: "1111111111111111111111111111111111111111111111111111111111111111".to_string(),
        trud_release_filesize_bytes: 37957865,
        datasets: vec![],
    });
    let original_bytes = initial_index.to_json_pretty()?;
    fs::write(&index_file, &original_bytes)?;

    let trud_dir = rel_dir.join("trud");
    fs::create_dir_all(&trud_dir)?;
    let prov: OdsProvenance = serde_json::from_str(&fs::read_to_string(rel_dir.join(PROVENANCE_FILENAME))?)?;
    let zip_sha = prov.trud_release_sha256.unwrap();

    let response_json = serde_json::json!({
        "apiVersion": "1",
        "releases": [
            {
                "id": "item1.zip",
                "releaseDate": "2026-07-31",
                "archiveFileName": "hscorgrefdataxml_data_7.0.0_20260731000001.zip",
                "archiveFileSizeBytes": 37983173,
                "archiveFileSha256": zip_sha,
                "archiveFileUrl": "https://example.com/item1.zip"
            },
            {
                "id": "item2.zip",
                "releaseDate": "2026-06-26",
                "archiveFileName": "hscorgrefdataxml_data_6.0.0_20260626000001.zip",
                "archiveFileSizeBytes": 37957865,
                "archiveFileSha256": "2222222222222222222222222222222222222222222222222222222222222222",
                "archiveFileUrl": "https://example.com/item2.zip"
            }
        ]
    });
    fs::write(trud_dir.join("trud-releases-2026-07-31.json"), serde_json::to_string_pretty(&response_json)?)?;

    let dist_dir = tmp.path().join("dist");
    let res = run(Args {
        input: Some(rel_dir),
        repository: "ods-data".to_string(),
        output: Some(dist_dir.clone()),
        doi: None,
        tool_repo: Some(tmp.path().to_path_buf()),
        index: None,
    });

    assert!(res.is_err());
    let err = res.unwrap_err().to_string();
    assert!(err.contains("The index records TRUD release 2026-06-26 with SHA-256 11111111…, but trud/trud-releases-2026-07-31.json says 22222222…"));
    assert!(err.contains("TRUD may have reissued it. Nothing was written."));

    // Index file and dist/ are unchanged
    assert_eq!(fs::read_to_string(&index_file)?, original_bytes);
    assert!(!dist_dir.exists(), "dist/ directory must not exist");

    Ok(())
}

#[test]
fn test_make_release_provenance_hash_differs_from_release_row_refuses() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();

    let index_file = tmp.path().join("data").join("releases.json");
    let mut initial_index = ods::index::OdsReleaseIndex::default();
    initial_index.releases.push(ods::index::Release {
        trud_release_date: "2026-07-31".to_string(),
        trud_release_sha256: "9999999999999999999999999999999999999999999999999999999999999999".to_string(),
        trud_release_filesize_bytes: 37983173,
        datasets: vec![],
    });
    fs::write(&index_file, initial_index.to_json_pretty()?)?;

    let failures = perform_all_release_checks(
        &rel_dir,
        ods::datapackage::dataset_version(),
        Some(tmp.path()),
        Some(&initial_index),
    )?;

    assert!(failures.iter().any(|f| f.contains("The release row's trud_release_sha256") && f.contains("does not match _provenance.json")));

    Ok(())
}

#[test]
fn test_make_release_second_dataset_version_on_same_date_added_beside_first() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();

    let index_file = tmp.path().join("data").join("releases.json");
    let prov: OdsProvenance = serde_json::from_str(&fs::read_to_string(rel_dir.join(PROVENANCE_FILENAME))?)?;
    let zip_sha = prov.trud_release_sha256.unwrap();

    let mut initial_index = ods::index::OdsReleaseIndex::default();
    initial_index.releases.push(ods::index::Release {
        trud_release_date: "2026-07-31".to_string(),
        trud_release_sha256: zip_sha,
        trud_release_filesize_bytes: 37983173,
        datasets: vec![ods::index::Dataset {
            dataset_version: "0.0.1".to_string(),
            manifest_digest: "sha256:0000000000000000000000000000000000000000000000000000000000000001".to_string(),
            dataset_doi: None,
            withdrawn: None,
        }],
    });
    fs::write(&index_file, initial_index.to_json_pretty()?)?;

    run(Args {
        input: Some(rel_dir),
        repository: "ods-data".to_string(),
        output: None,
        doi: None,
        tool_repo: Some(tmp.path().to_path_buf()),
        index: None,
    })?;

    let content = fs::read_to_string(&index_file)?;
    let index: ods::index::OdsReleaseIndex = serde_json::from_str(&content)?;

    assert_eq!(index.releases.len(), 1);
    assert_eq!(index.releases[0].trud_release_date, "2026-07-31");
    assert_eq!(index.releases[0].datasets.len(), 2);
    assert_eq!(index.releases[0].datasets[0].dataset_version, "0.0.1");
    assert_eq!(index.releases[0].datasets[1].dataset_version, ods::datapackage::dataset_version());

    Ok(())
}


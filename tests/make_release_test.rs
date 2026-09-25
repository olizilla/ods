use anyhow::Result;
use ods::commands::make_release::{perform_all_release_checks, run_as, Args, BuildIdentity};
use ods::provenance::{compute_file_sha256, OdsProvenance, PROVENANCE_FILENAME};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

mod common;
use common::{fixture_build_identity, setup_synthetic_repo_and_release};

// B6: the row records which ods built the dataset. `ods make release` reads no TRUD key and
// makes no network call: nothing here provides one.
#[test]
fn test_make_release_success_appends_to_releases_json() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();
    let build = fixture_build_identity(tmp.path());

    run_as(Args {
        input: Some(rel_dir.clone()),
        repository: "ods-data".to_string(),
        output: None,
        doi: Some("10.5281/zenodo.12345".to_string()),
        tool_repo: Some(tmp.path().to_path_buf()),
        index: None,
    }, &build)?;

    let index_file = tmp.path().join("data").join("releases.json");
    let content = fs::read_to_string(&index_file)?;
    let index: ods::index::OdsReleaseIndex = serde_json::from_str(&content)?;

    let expected_ver = ods::datapackage::DATASET_VERSION;
    assert_eq!(index.releases.len(), 1);
    assert_eq!(index.releases[0].trud_release_date, "2026-07-31");
    assert_eq!(index.releases[0].datasets.len(), 1);
    assert_eq!(index.releases[0].datasets[0].dataset_version, expected_ver);
    assert_eq!(index.releases[0].datasets[0].dataset_doi, Some("10.5281/zenodo.12345".to_string()));
    assert!(index.releases[0].datasets[0].manifest_digest.starts_with("sha256:"));
    assert_eq!(index.releases[0].datasets[0].tool_version, build.tool_version);
    assert_eq!(Some(index.releases[0].datasets[0].tool_git_sha.clone()), build.git_sha);

    Ok(())
}

// B6: each way an ods can be the wrong one to record a release, through the identity argument.
#[test]
fn test_make_release_refuses_an_ods_built_from_a_dirty_tree() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();
    let build = BuildIdentity { dirty: true, ..fixture_build_identity(tmp.path()) };

    let failures = perform_all_release_checks(
        &rel_dir,
        ods::datapackage::DATASET_VERSION,
        Some(tmp.path()),
        None,
        &build,
    )?;

    assert!(failures.iter().any(|f| f.contains("this ods was built from a dirty working tree")), "{:?}", failures);
    Ok(())
}

#[test]
fn test_make_release_refuses_an_ods_with_no_commit() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();
    let build = BuildIdentity { git_sha: None, ..fixture_build_identity(tmp.path()) };

    let failures = perform_all_release_checks(
        &rel_dir,
        ods::datapackage::DATASET_VERSION,
        Some(tmp.path()),
        None,
        &build,
    )?;

    assert!(failures.iter().any(|f| f.contains("this ods was built without a git commit")), "{:?}", failures);
    Ok(())
}

#[test]
fn test_make_release_refuses_an_ods_that_is_not_the_commit_its_tag_names() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();
    let other_commit = "0000000000000000000000000000000000000000".to_string();
    let build = BuildIdentity { git_sha: Some(other_commit.clone()), ..fixture_build_identity(tmp.path()) };

    let failures = perform_all_release_checks(
        &rel_dir,
        ods::datapackage::DATASET_VERSION,
        Some(tmp.path()),
        None,
        &build,
    )?;

    let tag = format!("v{}", build.tool_version);
    assert!(
        failures.iter().any(|f| f.contains(&format!("this ods is commit {}, not the commit {} points at", other_commit, tag))),
        "{:?}",
        failures
    );
    Ok(())
}

#[test]
fn test_make_release_refuses_an_ods_whose_tag_does_not_exist() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();
    let build = BuildIdentity { tool_version: "9.9.9".to_string(), ..fixture_build_identity(tmp.path()) };

    let failures = perform_all_release_checks(
        &rel_dir,
        ods::datapackage::DATASET_VERSION,
        Some(tmp.path()),
        None,
        &build,
    )?;

    assert!(
        failures.iter().any(|f| f.contains("not the commit v9.9.9 points at (tag missing)")),
        "{:?}",
        failures
    );
    Ok(())
}

#[test]
fn test_make_release_fails_on_duplicate_row_with_differing_manifest_digest() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();

    let ver = ods::datapackage::DATASET_VERSION;
    let prov: OdsProvenance = serde_json::from_str(&fs::read_to_string(rel_dir.join(PROVENANCE_FILENAME))?)?;
    let zip_sha = prov.trud_release_sha256.clone().unwrap();

    let (manifest, _) = ods::commands::make_oci::build_manifest_from_dir(&rel_dir, &prov, ver)?;
    let fixture_digest = manifest.digest()?;

    let mut index = ods::index::OdsReleaseIndex::baked().unwrap();
    index.releases.push(ods::index::Release {
        trud_release_date: "2026-07-31".to_string(),
        trud_release_sha256: zip_sha,
        trud_release_filesize_bytes: 37983173,
        datasets: vec![ods::index::Dataset {
            dataset_version: ver.to_string(),
            manifest_digest: "sha256:0f2a000000000000000000000000000000000000000000000000000000000000".to_string(),
            dataset_filesize_bytes: 29_700_000,
            tool_version: common::FIXTURE_TOOL_VERSION.to_string(),
            tool_git_sha: common::FIXTURE_TOOL_GIT_SHA.to_string(),
            dataset_doi: None,
            withdrawn: None,
        }],
    });

    let failures = perform_all_release_checks(
        &rel_dir,
        ver,
        Some(tmp.path()),
        Some(&index),
        &fixture_build_identity(tmp.path()),
    )?;

    assert!(failures.iter().any(|f| f.contains("sha256:0f2a000000000000000000000000000000000000000000000000000000000000")
        && f.contains(&fixture_digest)));
    Ok(())
}

#[test]
fn test_make_release_creates_dist_staging_tree_with_real_files() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();
    let dist_dir = tmp.path().join("dist");

    run_as(Args {
        input: Some(rel_dir.clone()),
        repository: "ods-data".to_string(),
        output: Some(dist_dir.clone()),
        doi: Some("10.5281/zenodo.12345".to_string()),
        tool_repo: Some(tmp.path().to_path_buf()),
        index: None,
    }, &fixture_build_identity(tmp.path()))?;

    assert!(dist_dir.exists(), "dist/ directory must exist");
    assert!(!dist_dir.join("releases.json").exists(), "dist/releases.json must NOT exist in dist/");

    let ver = ods::datapackage::DATASET_VERSION;
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
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();
    let not_repo_tmp = TempDir::new().unwrap();
    let not_a_repo = not_repo_tmp.path().join("not-a-repo");
    fs::create_dir_all(&not_a_repo).unwrap();

    // 1. Direct check in perform_all_release_checks
    let failures = perform_all_release_checks(
        &rel_dir,
        ods::datapackage::DATASET_VERSION,
        None,
        None,
        &fixture_build_identity(tmp.path()),
    ).unwrap();
    assert!(failures.iter().any(|f| f.contains("cannot locate the ods repository")));

    // 2. Full run refusal
    let res = run_as(Args {
        input: Some(rel_dir),
        repository: "ods-data".to_string(),
        output: None,
        doi: None,
        tool_repo: Some(not_a_repo),
        index: None,
    }, &fixture_build_identity(tmp.path()));

    assert!(res.is_err(), "Must refuse when tool_repo is not a repo");
    let err = format!("{:#}", res.unwrap_err());
    assert!(err.contains("cannot locate the ods repository"));
}

#[test]
fn test_make_release_refuses_implausible_filesize() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();

    // Mutate provenance to 16 bytes
    let prov_path = rel_dir.join(PROVENANCE_FILENAME);
    let mut prov: OdsProvenance = serde_json::from_str(&fs::read_to_string(&prov_path)?)?;
    prov.trud_release_filesize_bytes = Some(16);
    fs::write(&prov_path, serde_json::to_string_pretty(&prov)?)?;

    let failures = perform_all_release_checks(
        &rel_dir,
        ods::datapackage::DATASET_VERSION,
        None,
        None,
        &fixture_build_identity(tmp.path()),
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
        &fixture_build_identity(tmp.path()),
    )?;

    let expected_ver = ods::datapackage::DATASET_VERSION;
    let mismatch_failure = failures
        .iter()
        .find(|f| f.contains("datapackage.json version"))
        .expect("must have dataset_version mismatch failure");
    assert!(mismatch_failure.contains("1.0.0"));
    assert!(mismatch_failure.contains(expected_ver));
    assert!(mismatch_failure.contains("The release was compiled by an older tool. Re-run `ods make`"));

    // Full command execution fails
    let res = run_as(Args {
        input: Some(rel_dir),
        repository: "ods-data".to_string(),
        output: None,
        doi: None,
        tool_repo: Some(tmp.path().to_path_buf()),
        index: None,
    }, &fixture_build_identity(tmp.path()));
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

    let res = run_as(Args {
        input: Some(rel_dir),
        repository: "ods-data".to_string(),
        output: None,
        doi: None,
        tool_repo: Some(tmp.path().to_path_buf()),
        index: None,
    }, &fixture_build_identity(tmp.path()));
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
        &fixture_build_identity(tmp.path()),
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

    // Record release directory entries and mtimes
    let before_files: Vec<(PathBuf, std::time::SystemTime)> = fs::read_dir(&rel_dir)?
        .filter_map(|e| e.ok())
        .map(|e| (e.path(), e.metadata().unwrap().modified().unwrap()))
        .collect();

    let dist_dir = tmp.path().join("dist");
    let dirty = BuildIdentity { dirty: true, ..fixture_build_identity(tmp.path()) };
    let res = run_as(Args {
        input: Some(rel_dir.clone()),
        repository: "ods-data".to_string(),
        output: Some(dist_dir.clone()),
        doi: None,
        tool_repo: Some(tmp.path().to_path_buf()),
        index: None,
    }, &dirty);
    assert!(res.is_err(), "make release must fail when the recording ods was built from a dirty tree");
    let err = format!("{:#}", res.unwrap_err());
    assert!(err.contains("this ods was built from a dirty working tree"));
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
    let res = run_as(Args {
        input: Some(rel_dir.clone()),
        repository: "ods-data".to_string(),
        output: Some(dist_dir.clone()),
        doi: None,
        tool_repo: Some(tmp.path().to_path_buf()),
        index: Some(non_existent_index.clone()),
    }, &fixture_build_identity(tmp.path()));
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

    let cli_out = common::ods_cmd()
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

/// `target` spelled relative to this test's working directory, so a relative `--input` can be
/// exercised without changing the working directory, which every test in the binary shares.
fn relative_to_cwd(target: &Path) -> PathBuf {
    use std::path::Component;
    let cwd = std::env::current_dir().unwrap().canonicalize().unwrap();
    let target = target.canonicalize().unwrap();
    let mut path = PathBuf::new();
    for _ in cwd.components().filter(|c| matches!(c, Component::Normal(_))) {
        path.push("..");
    }
    for c in target.components().filter(|c| matches!(c, Component::Normal(_))) {
        path.push(c);
    }
    path
}

#[test]
fn test_make_release_staging_with_relative_input() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();
    let build = fixture_build_identity(tmp.path());
    let dist_relative = tmp.path().join("dist-relative");
    let dist_absolute = tmp.path().join("dist-absolute");

    let index_src = tmp.path().join("data").join("releases.json");
    let index_a = tmp.path().join("releases-a.json");
    let index_b = tmp.path().join("releases-b.json");
    fs::copy(&index_src, &index_a)?;
    fs::copy(&index_src, &index_b)?;

    // 1. Stage from a relative --input
    let relative_input = relative_to_cwd(&rel_dir);
    assert!(relative_input.is_relative());
    run_as(Args {
        input: Some(relative_input),
        repository: "ods-data".to_string(),
        output: Some(dist_relative.clone()),
        doi: None,
        tool_repo: Some(tmp.path().to_path_buf()),
        index: Some(index_a),
    }, &build)?;

    // 2. Stage from an absolute --input
    run_as(Args {
        input: Some(rel_dir.clone()),
        repository: "ods-data".to_string(),
        output: Some(dist_absolute.clone()),
        doi: None,
        tool_repo: Some(tmp.path().to_path_buf()),
        index: Some(index_b),
    }, &build)?;

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

    run_as(Args {
        input: Some(rel_dir.clone()),
        repository: "ods-data".to_string(),
        output: Some(dist_dir.clone()),
        doi: None,
        tool_repo: Some(tmp.path().to_path_buf()),
        index: None,
    }, &fixture_build_identity(tmp.path()))?;

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

    let res = run_as(Args {
        input: Some(rel_dir),
        repository: "ods-data".to_string(),
        output: Some(dist_dir),
        doi: None,
        tool_repo: Some(tmp.path().to_path_buf()),
        index: None,
    }, &fixture_build_identity(tmp.path()));

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

// R2: `ods make release` records the TRUD release it's recording, from `_provenance.json`, and
// reads no listing of other TRUD releases from disk. Every other date gets its row when a
// dataset built from it is recorded.
#[test]
fn test_make_release_records_only_the_release_it_records() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();
    let prov: OdsProvenance = serde_json::from_str(&fs::read_to_string(rel_dir.join(PROVENANCE_FILENAME))?)?;

    run_as(Args {
        input: Some(rel_dir),
        repository: "ods-data".to_string(),
        output: None,
        doi: None,
        tool_repo: Some(tmp.path().to_path_buf()),
        index: None,
    }, &fixture_build_identity(tmp.path()))?;

    let index_file = tmp.path().join("data").join("releases.json");
    let index: ods::index::OdsReleaseIndex = serde_json::from_str(&fs::read_to_string(&index_file)?)?;

    assert_eq!(index.releases.len(), 1, "the index gains exactly the release being recorded");
    let row = &index.releases[0];
    assert_eq!(row.trud_release_date, "2026-07-31");
    assert_eq!(Some(row.trud_release_sha256.clone()), prov.trud_release_sha256);
    assert_eq!(Some(row.trud_release_filesize_bytes), prov.trud_release_filesize_bytes);
    println!("{}", serde_json::to_string(row)?);
    assert_eq!(row.datasets.len(), 1);

    Ok(())
}

#[test]
fn test_make_release_leaves_another_recorded_date_byte_for_byte() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();

    let index_file = tmp.path().join("data").join("releases.json");
    let mut initial_index = ods::index::OdsReleaseIndex::baked().unwrap();
    initial_index.releases.push(ods::index::Release {
        trud_release_date: "2026-06-26".to_string(),
        trud_release_sha256: "712FE6C3810DC9C5BC6868F4F4038B0D8D8B92CC3EFD260A318811DEBFE04233".to_string(),
        trud_release_filesize_bytes: 37957865,
        datasets: vec![],
    });
    fs::write(&index_file, initial_index.to_json_pretty()?)?;

    run_as(Args {
        input: Some(rel_dir),
        repository: "ods-data".to_string(),
        output: None,
        doi: None,
        tool_repo: Some(tmp.path().to_path_buf()),
        index: None,
    }, &fixture_build_identity(tmp.path()))?;

    let content = fs::read_to_string(&index_file)?;
    let index: ods::index::OdsReleaseIndex = serde_json::from_str(&content)?;
    let rel_26 = index.releases.iter().find(|r| r.trud_release_date == "2026-06-26").unwrap();
    assert_eq!(rel_26.trud_release_sha256, "712FE6C3810DC9C5BC6868F4F4038B0D8D8B92CC3EFD260A318811DEBFE04233");
    assert_eq!(rel_26.trud_release_filesize_bytes, 37957865);
    assert!(rel_26.datasets.is_empty());

    Ok(())
}

#[test]
fn test_make_release_provenance_hash_differs_from_release_row_refuses() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();

    let index_file = tmp.path().join("data").join("releases.json");
    let mut initial_index = ods::index::OdsReleaseIndex::baked().unwrap();
    initial_index.releases.push(ods::index::Release {
        trud_release_date: "2026-07-31".to_string(),
        trud_release_sha256: "9999999999999999999999999999999999999999999999999999999999999999".to_string(),
        trud_release_filesize_bytes: 37983173,
        datasets: vec![],
    });
    fs::write(&index_file, initial_index.to_json_pretty()?)?;

    let failures = perform_all_release_checks(
        &rel_dir,
        ods::datapackage::DATASET_VERSION,
        Some(tmp.path()),
        Some(&initial_index),
        &fixture_build_identity(tmp.path()),
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

    let mut initial_index = ods::index::OdsReleaseIndex::baked().unwrap();
    initial_index.releases.push(ods::index::Release {
        trud_release_date: "2026-07-31".to_string(),
        trud_release_sha256: zip_sha,
        trud_release_filesize_bytes: 37983173,
        datasets: vec![ods::index::Dataset {
            dataset_version: "0.0.1".to_string(),
            manifest_digest: "sha256:0000000000000000000000000000000000000000000000000000000000000001".to_string(),
            dataset_filesize_bytes: 29_700_000,
            tool_version: common::FIXTURE_TOOL_VERSION.to_string(),
            tool_git_sha: common::FIXTURE_TOOL_GIT_SHA.to_string(),
            dataset_doi: None,
            withdrawn: None,
        }],
    });
    fs::write(&index_file, initial_index.to_json_pretty()?)?;

    run_as(Args {
        input: Some(rel_dir),
        repository: "ods-data".to_string(),
        output: None,
        doi: None,
        tool_repo: Some(tmp.path().to_path_buf()),
        index: None,
    }, &fixture_build_identity(tmp.path()))?;

    let content = fs::read_to_string(&index_file)?;
    let index: ods::index::OdsReleaseIndex = serde_json::from_str(&content)?;

    assert_eq!(index.releases.len(), 1);
    assert_eq!(index.releases[0].trud_release_date, "2026-07-31");
    assert_eq!(index.releases[0].datasets.len(), 2);
    assert_eq!(index.releases[0].datasets[0].dataset_version, "0.0.1");
    assert_eq!(index.releases[0].datasets[1].dataset_version, ods::datapackage::DATASET_VERSION);

    Ok(())
}

#[test]
fn test_make_release_identical_republish_is_noop_leaving_index_byte_identical() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();
    let index_file = tmp.path().join("data").join("releases.json");

    // First publish
    run_as(Args {
        input: Some(rel_dir.clone()),
        repository: "ods-data".to_string(),
        output: None,
        doi: None,
        tool_repo: Some(tmp.path().to_path_buf()),
        index: None,
    }, &fixture_build_identity(tmp.path()))?;

    let bytes_after_first = fs::read(&index_file)?;

    // Second publish: identical inputs
    run_as(Args {
        input: Some(rel_dir),
        repository: "ods-data".to_string(),
        output: None,
        doi: None,
        tool_repo: Some(tmp.path().to_path_buf()),
        index: None,
    }, &fixture_build_identity(tmp.path()))?;

    let bytes_after_second = fs::read(&index_file)?;

    // Must be byte-identical
    assert_eq!(
        bytes_after_first, bytes_after_second,
        "Identical re-publish must leave data/releases.json byte-identical"
    );

    // Verify row wasn't duplicated
    let index: ods::index::OdsReleaseIndex = serde_json::from_slice(&bytes_after_second)?;
    assert_eq!(index.releases.len(), 1);
    assert_eq!(index.releases[0].datasets.len(), 1);

    Ok(())
}

// R2: an identical re-publish leaves the row as the first ods recorded it, so the row keeps
// naming the ods that built the dataset first, and the index stays byte-identical.
#[test]
fn test_make_release_republish_by_another_ods_keeps_the_first_row() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();
    let index_file = tmp.path().join("data").join("releases.json");
    let first = fixture_build_identity(tmp.path());

    let args = |dir: &Path| Args {
        input: Some(dir.to_path_buf()),
        repository: "ods-data".to_string(),
        output: None,
        doi: None,
        tool_repo: Some(tmp.path().to_path_buf()),
        index: None,
    };
    run_as(args(&rel_dir), &first)?;
    let after_first = fs::read(&index_file)?;

    // A later commit on the same tag's line, at the same dataset version, rebuilds the same
    // bytes. It is a different ods, and the tag it claims still has to exist.
    let _ = common::git_cmd(tmp.path()).args(["commit", "--allow-empty", "-m", "later", "--no-gpg-sign"]).output();
    let later = fixture_build_identity(tmp.path());
    assert_ne!(first.git_sha, later.git_sha);
    let _ = common::git_cmd(tmp.path()).args(["tag", "--no-sign", "-f", &format!("v{}", later.tool_version)]).output();
    run_as(args(&rel_dir), &later)?;

    assert_eq!(after_first, fs::read(&index_file)?, "a re-publish by another ods must not rewrite the row");
    let index: ods::index::OdsReleaseIndex = serde_json::from_slice(&after_first)?;
    assert_eq!(Some(index.releases[0].datasets[0].tool_git_sha.clone()), first.git_sha);
    Ok(())
}

#[test]
fn test_make_release_succeeds_when_tool_repo_has_no_releases_json() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();

    // Remove data/releases.json entirely from the tool_repo
    let index_file = tmp.path().join("data").join("releases.json");
    if index_file.exists() {
        fs::remove_file(&index_file)?;
    }

    // Verify release checks pass without data/releases.json
    let failures = perform_all_release_checks(
        &rel_dir,
        ods::datapackage::DATASET_VERSION,
        Some(tmp.path()),
        None,
        &fixture_build_identity(tmp.path()),
    )?;
    assert!(
        failures.is_empty(),
        "Release checks must pass when tool repo has no data/releases.json: {:?}",
        failures
    );

    // Build the release
    run_as(Args {
        input: Some(rel_dir),
        repository: "ods-data".to_string(),
        output: None,
        doi: Some("10.5281/zenodo.12345".to_string()),
        tool_repo: Some(tmp.path().to_path_buf()),
        index: None,
    }, &fixture_build_identity(tmp.path()))?;

    assert!(
        index_file.exists(),
        "data/releases.json must be created by make release"
    );
    let content = fs::read_to_string(&index_file)?;
    let index: ods::index::OdsReleaseIndex = serde_json::from_str(&content)?;

    // Assert it validates and writes valid, non-empty fingerprints
    index.validate()?;
    assert!(
        !index.trud_signing_key_fingerprints.is_empty(),
        "Fingerprints must not be empty"
    );
    let baked = ods::index::OdsReleaseIndex::baked()?;
    assert_eq!(
        index.trud_signing_key_fingerprints,
        baked.trud_signing_key_fingerprints,
        "Fingerprints must match baked release index"
    );

    Ok(())
}

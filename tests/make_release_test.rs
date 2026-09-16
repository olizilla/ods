use anyhow::Result;
use ods::commands::make_release::{perform_all_release_checks, run, Args};
use ods::provenance::{compute_file_sha256, OdsProvenance, PROVENANCE_FILENAME};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn git_cmd(repo_dir: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(repo_dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z");
    cmd
}

fn setup_synthetic_repo_and_release() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let rel_dir = tmp.path().join("releases").join("2026-07-31");
    let trud_dir = rel_dir.join("trud");
    fs::create_dir_all(&trud_dir).unwrap();

    let outer_zip_path = trud_dir.join("hscorgrefdataxml_data_7.0.0_20260731000001.zip");
    {
        let outer_file = File::create(&outer_zip_path).unwrap();
        let mut outer_zip = zip::ZipWriter::new(outer_file);
        let options = zip::write::SimpleFileOptions::default()
            .last_modified_time(zip::DateTime::from_date_and_time(2026, 1, 1, 0, 0, 0).unwrap());
        outer_zip.start_file("dummy.txt", options).unwrap();
        outer_zip.write_all(b"dummy source zip").unwrap();
        outer_zip.finish().unwrap();
    }
    let zip_sha256 = compute_file_sha256(&outer_zip_path).unwrap();

    fs::write(rel_dir.join("orgs.parquet"), b"dummy orgs parquet content").unwrap();
    fs::write(rel_dir.join("roles.parquet"), b"dummy roles parquet content").unwrap();
    fs::write(rel_dir.join("datapackage.json"), b"{\"name\": \"test\", \"version\": \"0.1.0\"}").unwrap();
    fs::write(rel_dir.join("NOTES.md"), b"# Release Notes\nTest release.").unwrap();

    // Create Cargo.toml and git repo
    fs::write(
        tmp.path().join("Cargo.toml"),
        format!("[package]\nname = \"ods\"\nversion = \"{}\"\n", env!("CARGO_PKG_VERSION")),
    )
    .unwrap();
    fs::create_dir_all(tmp.path().join("src")).unwrap();
    fs::write(tmp.path().join("src").join("main.rs"), "fn main() {}\n").unwrap();

    let _ = git_cmd(tmp.path()).args(["init", "-b", "main"]).output();
    let _ = git_cmd(tmp.path()).args(["add", "."]).output();
    let _ = git_cmd(tmp.path())
        .args(["commit", "-m", "initial", "--no-gpg-sign"])
        .output();
    let head_out = git_cmd(tmp.path()).args(["rev-parse", "HEAD"]).output().unwrap();
    let git_sha = String::from_utf8_lossy(&head_out.stdout).trim().to_string();
    let tool_tag = format!("v{}", env!("CARGO_PKG_VERSION"));
    let _ = git_cmd(tmp.path()).args(["tag", "--no-sign", &tool_tag]).output();

    let mut prov = OdsProvenance::default();
    prov.trud_release_name = Some("Release 7.0.0".to_string());
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_file = Some("hscorgrefdataxml_data_7.0.0_20260731000001.zip".to_string());
    prov.trud_release_filesize_bytes = Some(37_983_173);
    prov.trud_release_sha256 = Some(zip_sha256.clone());
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);
    prov.publication_date = Some("2026-07-28".to_string());
    prov.publication_seq_num = Some("4700".to_string());
    prov.publication_type = Some("Full".to_string());
    prov.publication_record_count = Some(2);
    prov.tool_version = Some(env!("CARGO_PKG_VERSION").to_string());
    prov.tool_git_sha = Some(git_sha);
    prov.tool_git_dirty = Some(false);
    prov.dataset_version = Some(ods::datapackage::dataset_version().to_string());

    let prov_path = rel_dir.join(PROVENANCE_FILENAME);
    fs::write(&prov_path, serde_json::to_string_pretty(&prov).unwrap()).unwrap();

    // Create data/releases.json in repo
    fs::create_dir_all(tmp.path().join("data")).unwrap();
    let init_index = ods::index::OdsReleaseIndex {
        type_tag: "ods_release_index".to_string(),
        index_version: 2,
        concept_doi: None,
        mirrors: vec![
            ods::index::MirrorEntry {
                url: "https://ods.fyi/v2/ods-data".to_string(),
            },
            ods::index::MirrorEntry {
                url: "https://ghcr.io/v2/olizilla/ods-data".to_string(),
            },
        ],
        releases: vec![],
    };
    fs::write(
        tmp.path().join("data").join("releases.json"),
        serde_json::to_string_pretty(&init_index).unwrap() + "\n",
    )
    .unwrap();

    // Generate OCI layout first via make oci
    ods::commands::make_oci::run(ods::commands::make_oci::Args {
        input: Some(rel_dir.clone()),
        check: false,
    })
    .unwrap();

    (tmp, rel_dir)
}

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
    assert_eq!(index.releases[0].dataset_version, expected_ver);
    assert_eq!(index.releases[0].tag, format!("2026-07-31_{}", expected_ver));
    assert_eq!(index.releases[0].dataset_doi, Some("10.5281/zenodo.12345".to_string()));
    assert!(index.releases[0].manifest_digest.starts_with("sha256:"));

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
    let mut index = ods::index::OdsReleaseIndex::baked()?;
    index.releases.push(ods::index::ReleaseIndexEntry {
        trud_release_date: "2026-07-31".to_string(),
        dataset_version: ver.to_string(),
        tag: format!("2026-07-31_{}", ver),
        manifest_digest: "sha256:0f2a000000000000000000000000000000000000000000000000000000000000".to_string(),
        trud_release_sha256: "8151248D".to_string(),
        tool_version: "0.4.3".to_string(),
        dataset_doi: None,
        withdrawn: None,
    });

    let failures = perform_all_release_checks(
        &rel_dir,
        ver,
        Some(tmp.path()),
        Some(&index),
    )?;

    let expected_msg = format!("data/releases.json already has a row for 2026-07-31 {}", ver);
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

    // Mutate provenance to 1.0.0, differing from tool constant 0.1.0
    let prov_path = rel_dir.join(PROVENANCE_FILENAME);
    let mut prov: OdsProvenance = serde_json::from_str(&fs::read_to_string(&prov_path)?)?;
    prov.dataset_version = Some("1.0.0".to_string());
    fs::write(&prov_path, serde_json::to_string_pretty(&prov)?)?;

    let failures = perform_all_release_checks(
        &rel_dir,
        "1.0.0",
        Some(tmp.path()),
        None,
    )?;

    let expected_ver = ods::datapackage::dataset_version();
    let mismatch_failure = failures
        .iter()
        .find(|f| f.contains("Provenance dataset_version"))
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
    assert!(err.contains("Provenance dataset_version (1.0.0) does not match this build of ods"));

    Ok(())
}

#[test]
fn test_make_release_refuses_missing_dataset_version() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();

    // Mutate provenance to None
    let prov_path = rel_dir.join(PROVENANCE_FILENAME);
    let mut prov: OdsProvenance = serde_json::from_str(&fs::read_to_string(&prov_path)?)?;
    prov.dataset_version = None;
    fs::write(&prov_path, serde_json::to_string_pretty(&prov)?)?;

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
    assert!(err.contains("Missing dataset_version in _provenance.json"));
    assert!(err.contains("ods make"));

    Ok(())
}

#[test]
fn test_make_release_refuses_non_semver_dataset_version() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();

    let prov_path = rel_dir.join(PROVENANCE_FILENAME);
    let mut prov: OdsProvenance = serde_json::from_str(&fs::read_to_string(&prov_path)?)?;
    prov.dataset_version = Some("invalid-semver".to_string());
    fs::write(&prov_path, serde_json::to_string_pretty(&prov)?)?;

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
    let tmp = TempDir::new()?;
    let dist_dir = tmp.path().join("dist");
    let index_file = tmp.path().join("releases.json");
    let init_index = ods::index::OdsReleaseIndex {
        type_tag: "ods_release_index".to_string(),
        index_version: 2,
        concept_doi: None,
        mirrors: vec![],
        releases: vec![],
    };
    fs::write(&index_file, serde_json::to_string_pretty(&init_index)? + "\n")?;

    // Copy the real fixture release into temp directory
    let fixture_release = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("ods_data")
        .join("releases")
        .join("2026-07-31");

    let temp_ws = tmp.path().join("ods_data").join("releases").join("2026-07-31");
    fs::create_dir_all(&temp_ws)?;
    for entry in fs::read_dir(&fixture_release)? {
        let entry = entry?;
        let path = entry.path();
        let file_name = entry.file_name();
        if path.is_file() {
            fs::copy(&path, temp_ws.join(file_name))?;
        }
    }

    // Pass relative path from tmp.path()
    let rel_input = PathBuf::from("ods_data").join("releases").join("2026-07-31");

    let orig_dir = std::env::current_dir()?;
    std::env::set_current_dir(tmp.path())?;

    let res = run(Args {
        input: Some(rel_input),
        repository: "ods-data".to_string(),
        output: Some(dist_dir.clone()),
        doi: None,
        tool_repo: Some(PathBuf::from(env!("CARGO_MANIFEST_DIR"))),
        index: Some(index_file),
    });

    let _ = std::env::set_current_dir(orig_dir);
    res?;

    // Must have 11 objects: 8 blobs (7 layer blobs + 1 config blob) + 3 manifests (versioned, date, latest)
    let count = walkdir::WalkDir::new(&dist_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .count();
    assert_eq!(count, 11, "staging tree must contain exactly 11 objects");

    // Verify every blob's SHA-256 matches its filename
    let blobs_dir = dist_dir.join("v2").join("ods-data").join("blobs").join("sha256");
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


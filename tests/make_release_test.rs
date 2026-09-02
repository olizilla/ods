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
    prov.dataset_version = Some("1.0.1".to_string());

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
        version: Some("1.0.1".to_string()),
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
        version: Some("1.0.1".to_string()),
        repository: "ods-data".to_string(),
        output: None,
        doi: Some("10.5281/zenodo.12345".to_string()),
        tool_repo: Some(tmp.path().to_path_buf()),
        index: None,
        offline: true,
    })?;

    let index_file = tmp.path().join("data").join("releases.json");
    let content = fs::read_to_string(&index_file)?;
    let index: ods::index::OdsReleaseIndex = serde_json::from_str(&content)?;

    assert_eq!(index.releases.len(), 1);
    assert_eq!(index.releases[0].trud_release_date, "2026-07-31");
    assert_eq!(index.releases[0].dataset_version, "1.0.1");
    assert_eq!(index.releases[0].tag, "2026-07-31_1.0.1");
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
        "1.0.1",
        Some(tmp.path()),
        None,
        true,
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
        "1.0.1",
        Some(tmp.path()),
        None,
        true,
    )?;

    assert!(failures.iter().any(|f| f.contains("tool_git_sha")));
    Ok(())
}

#[test]
fn test_make_release_fails_on_existing_git_tag() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();

    // Create the tag in git
    git_cmd(tmp.path())
        .args(["tag", "--no-sign", "data/2026-07-31_1.0.1"])
        .output()?;

    let failures = perform_all_release_checks(
        &rel_dir,
        "1.0.1",
        Some(tmp.path()),
        None,
        true,
    )?;

    assert!(failures.iter().any(|f| f.contains("already exists locally")));
    Ok(())
}

#[test]
fn test_make_release_fails_on_duplicate_row_in_index() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();

    let mut index = ods::index::OdsReleaseIndex::baked()?;
    index.releases.push(ods::index::ReleaseIndexEntry {
        trud_release_date: "2026-07-31".to_string(),
        dataset_version: "1.0.1".to_string(),
        tag: "2026-07-31_1.0.1".to_string(),
        manifest_digest: "sha256:0f2a000000000000000000000000000000000000000000000000000000000000".to_string(),
        trud_release_sha256: "8151248D".to_string(),
        tool_version: "0.4.3".to_string(),
        dataset_doi: None,
        withdrawn: None,
    });

    let failures = perform_all_release_checks(
        &rel_dir,
        "1.0.1",
        Some(tmp.path()),
        Some(&index),
        true,
    )?;

    assert!(failures.iter().any(|f| f.contains("data/releases.json already has a row for 2026-07-31 1.0.1")));
    Ok(())
}

#[test]
fn test_make_release_creates_dist_staging_tree_with_real_files() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();
    let dist_dir = tmp.path().join("dist");

    run(Args {
        input: Some(rel_dir.clone()),
        version: None, // Test reading version from _provenance.json
        repository: "ods-data".to_string(),
        output: Some(dist_dir.clone()),
        doi: Some("10.5281/zenodo.12345".to_string()),
        tool_repo: Some(tmp.path().to_path_buf()),
        index: None,
        offline: true,
    })?;

    assert!(dist_dir.exists(), "dist/ directory must exist");
    assert!(!dist_dir.join("releases.json").exists(), "dist/releases.json must NOT exist in dist/");

    let manifests_dir = dist_dir.join("v2").join("ods-data").join("manifests");
    assert!(manifests_dir.join("2026-07-31_1.0.1").exists());
    assert!(manifests_dir.join("2026-07-31").exists());
    assert!(manifests_dir.join("latest").exists());

    // Check versioned and latest layer files
    let versioned_orgs = dist_dir.join("2026-07-31").join("1.0.1").join("orgs.parquet");
    let latest_orgs = dist_dir.join("latest").join("orgs.parquet");
    assert!(versioned_orgs.exists());
    assert!(latest_orgs.exists());
    assert!(!versioned_orgs.is_symlink(), "dist files must be real files, not symlinks");
    assert!(!latest_orgs.is_symlink(), "dist files must be real files, not symlinks");

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
        "1.0.1",
        None,
        None,
        true,
    ).unwrap();
    assert!(failures.iter().any(|f| f.contains("cannot locate the ods repository")));

    // 2. Full run refusal
    let res = run(Args {
        input: Some(rel_dir),
        version: Some("1.0.1".to_string()),
        repository: "ods-data".to_string(),
        output: None,
        doi: None,
        tool_repo: Some(not_a_repo),
        index: None,
        offline: true,
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
        "1.0.1",
        None,
        None,
        true,
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
        "1.0.1",
        None,
        None,
        true,
    )?;

    assert!(
        failures.iter().any(|f| f.contains("trud_release_filesize_bytes is implausibly small")),
        "Must refuse implausible filesize, got: {:?}",
        failures
    );

    Ok(())
}


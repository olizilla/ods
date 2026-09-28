use anyhow::Result;
use ods::commands::make_release::{perform_all_release_checks, run_as, Args, BuildIdentity};
use ods::provenance::{Embedded, ReleaseFacts};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

use crate::common;
use common::{fixture_build_identity, setup_synthetic_repo_and_release};

/// What the fixture release's Parquet files say it is.
fn facts(rel_dir: &Path) -> ReleaseFacts {
    ods::provenance::read_release(rel_dir).unwrap().facts().cloned().expect("the fixture carries provenance")
}

/// Rewrites the fixture's two Parquet files, same content, carrying the object `change` makes
/// of the one they carry now: a release built with other provenance.
fn rewrite_embedded(rel_dir: &Path, change: impl Fn(&mut Embedded)) {
    let mut embedded = facts(rel_dir).embedded;
    change(&mut embedded);
    for (name, content) in [("orgs.parquet", "dummy orgs parquet content"), ("roles.parquet", "dummy roles parquet content")] {
        ods::commands::parquet::write_stub_parquet(&rel_dir.join(name), Some(&embedded), content).unwrap();
    }
}

// B6: the row records which ods built the dataset. `ods make release` reads no TRUD key and
// makes no network call: nothing here provides one.
#[test]
fn test_make_release_success_appends_to_releases_json() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();
    let build = fixture_build_identity(tmp.path());

    run_as(Args {
        input: Some(rel_dir.clone()),
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
    let zip_sha = facts(&rel_dir).source_sha256_upper();

    let (manifest, _) = ods::commands::make_oci::build_manifest_from_dir(&rel_dir)?;
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

    // Files built from a 16-byte archive: the check reads the source's size from the files
    rewrite_embedded(&rel_dir, |e| e.sources[0].bytes = 16);

    let failures = perform_all_release_checks(
        &rel_dir,
        ods::datapackage::DATASET_VERSION,
        None,
        None,
        &fixture_build_identity(tmp.path()),
    )?;

    assert!(
        failures.iter().any(|f| f.contains("the source archive's size is implausibly small (16 bytes")),
        "Must refuse implausible filesize, got: {:?}",
        failures
    );

    Ok(())
}

/// Rewrites `rel_dir`'s files carrying dataset `version`: what the version checks below read.
fn repack_with_version(rel_dir: &Path, version: &str) -> Result<()> {
    rewrite_embedded(rel_dir, |e| e.version = format!("2026-07-31_{version}"));
    Ok(())
}

#[test]
fn test_make_release_fails_on_dataset_version_mismatch_with_tool() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();

    // Repack with a version differing from this build's compiled constant
    repack_with_version(&rel_dir, "1.0.0")?;

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
        .find(|f| f.contains("the Parquet files' dataset version"))
        .expect("must have dataset_version mismatch failure");
    assert!(mismatch_failure.contains("1.0.0"));
    assert!(mismatch_failure.contains(expected_ver));
    assert!(mismatch_failure.contains("The release was compiled by an older tool. Re-run `ods make`"));

    // Full command execution fails
    let res = run_as(Args {
        input: Some(rel_dir),
        doi: None,
        tool_repo: Some(tmp.path().to_path_buf()),
        index: None,
    }, &fixture_build_identity(tmp.path()));
    assert!(res.is_err());
    let err = format!("{:#}", res.unwrap_err());
    assert!(err.contains("the Parquet files' dataset version (1.0.0) does not match this build of ods"));

    Ok(())
}

#[test]
fn test_make_release_refuses_missing_dataset_version() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();

    // Files whose `version` names no dataset version after the source release's
    rewrite_embedded(&rel_dir, |e| e.version = "2026-07-31".to_string());

    let res = run_as(Args {
        input: Some(rel_dir),
        doi: None,
        tool_repo: Some(tmp.path().to_path_buf()),
        index: None,
    }, &fixture_build_identity(tmp.path()));
    assert!(res.is_err());
    let err = format!("{:#}", res.unwrap_err());
    // `run_as` reads the files' embedded object first, and refuses one it can't read.
    assert!(err.contains("carry provenance this ods can't read"), "got error: {}", err);
    assert!(err.contains("isn't <source release>_<dataset version>"), "got error: {}", err);

    Ok(())
}

#[test]
fn test_make_release_refuses_non_semver_dataset_version() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();

    repack_with_version(&rel_dir, "invalid-semver")?;

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
    // `setup_synthetic_repo_and_release` already packed `oci/`: a failed check must leave it be.
    let oci_dir = rel_dir.join("oci");
    let mut before_oci_files: Vec<PathBuf> = fs::read_dir(oci_dir.join("blobs").join("sha256"))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    before_oci_files.sort();

    // Record release directory entries and mtimes
    let before_files: Vec<(PathBuf, std::time::SystemTime)> = fs::read_dir(&rel_dir)?
        .filter_map(|e| e.ok())
        .map(|e| (e.path(), e.metadata().unwrap().modified().unwrap()))
        .collect();

    let dirty = BuildIdentity { dirty: true, ..fixture_build_identity(tmp.path()) };
    let res = run_as(Args {
        input: Some(rel_dir.clone()),
        doi: None,
        tool_repo: Some(tmp.path().to_path_buf()),
        index: None,
    }, &dirty);
    assert!(res.is_err(), "make release must fail when the recording ods was built from a dirty tree");
    let err = format!("{:#}", res.unwrap_err());
    assert!(err.contains("this ods was built from a dirty working tree"));

    let mut after_oci_files: Vec<PathBuf> = fs::read_dir(oci_dir.join("blobs").join("sha256"))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    after_oci_files.sort();
    assert_eq!(before_oci_files, after_oci_files, "oci/ must not be rewritten when checks fail");

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
    // `setup_synthetic_repo_and_release` already packed a stored manifest — `ods make release`
    // only ever reads it, never writes it (`.agents/briefs/manifest-only.md`: packing is a
    // separate, earlier step), so this test only needs to isolate the index-file failure, not
    // an unpacked release.
    let oci_dir = rel_dir.join("oci");
    let before_oci_files: Vec<PathBuf> = fs::read_dir(oci_dir.join("blobs").join("sha256"))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();

    // Record release directory entries and mtimes before running
    let before_files: Vec<(PathBuf, std::time::SystemTime)> = fs::read_dir(&rel_dir)?
        .filter_map(|e| e.ok())
        .map(|e| (e.path(), e.metadata().unwrap().modified().unwrap()))
        .collect();

    let non_existent_index = tmp.path().join("does_not_exist_releases.json");

    // Test programmatic run()
    let res = run_as(Args {
        input: Some(rel_dir.clone()),
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
    let mut after_oci_files: Vec<PathBuf> = fs::read_dir(oci_dir.join("blobs").join("sha256"))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    let mut before_oci_files = before_oci_files;
    before_oci_files.sort();
    after_oci_files.sort();
    assert_eq!(before_oci_files, after_oci_files, "oci/ must not be rewritten when index cannot be read");

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
    let mut cli_after_oci_files: Vec<PathBuf> = fs::read_dir(oci_dir.join("blobs").join("sha256"))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    cli_after_oci_files.sort();
    assert_eq!(before_oci_files, cli_after_oci_files, "oci/ must not be rewritten after the CLI run either");

    Ok(())
}

// R2: `ods make release` records the TRUD release it's recording, from the Parquet files, and
// reads no listing of other TRUD releases from disk. Every other date gets its row when a
// dataset built from it is recorded.
#[test]
fn test_make_release_records_only_the_release_it_records() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();
    let prov = facts(&rel_dir);

    run_as(Args {
        input: Some(rel_dir),
        doi: None,
        tool_repo: Some(tmp.path().to_path_buf()),
        index: None,
    }, &fixture_build_identity(tmp.path()))?;

    let index_file = tmp.path().join("data").join("releases.json");
    let index: ods::index::OdsReleaseIndex = serde_json::from_str(&fs::read_to_string(&index_file)?)?;

    assert_eq!(index.releases.len(), 1, "the index gains exactly the release being recorded");
    let row = &index.releases[0];
    assert_eq!(row.trud_release_date, "2026-07-31");
    assert_eq!(row.trud_release_sha256, prov.source_sha256_upper());
    assert_eq!(row.trud_release_filesize_bytes, prov.source.bytes);
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

    assert!(failures.iter().any(|f| f.contains("The release row's trud_release_sha256") && f.contains("does not match the Parquet files' source hash")));

    Ok(())
}

#[test]
fn test_make_release_second_dataset_version_on_same_date_added_beside_first() -> Result<()> {
    let (tmp, rel_dir) = setup_synthetic_repo_and_release();

    let index_file = tmp.path().join("data").join("releases.json");
    let zip_sha = facts(&rel_dir).source_sha256_upper();

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
        doi: None,
        tool_repo: Some(tmp.path().to_path_buf()),
        index: None,
    }, &fixture_build_identity(tmp.path()))?;

    let bytes_after_first = fs::read(&index_file)?;

    // Second publish: identical inputs
    run_as(Args {
        input: Some(rel_dir),
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

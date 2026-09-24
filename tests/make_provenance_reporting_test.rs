//! Acceptance tests for `.agents/briefs/make-names-its-provenance.md`.
//!
//! Verifies:
//! - Task 1: Report an unverified source when building from an archive file without
//!   `_provenance.json` above it: a `!` line after the report block (`make-output.md` holds
//!   warnings until the block has settled). Directory input keeps existing hard error.
//!   `ods make release` refuses unverified output.
//! - Task 2: Name the archive read (`* Source: <path>`) first, as `ods info` names its source,
//!   and the `_provenance.json` used (`* Provenance: <path>`) only when it isn't the release
//!   directory's own. Everything `ods make` prints goes to stderr; stdout stays empty.

mod common;

use common::{create_mock_trud_zip as create_mock_zip, ods_binary};
use std::fs;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Task 1: Report an unverified source at the start
// ---------------------------------------------------------------------------

#[test]
fn test_task1_unverified_archive_prints_warning_and_builds_successfully() {
    let tmp = TempDir::new().unwrap();
    let archive_dir = tmp.path().join("bare_archive");
    fs::create_dir_all(&archive_dir).unwrap();
    let zip_path = create_mock_zip(&archive_dir, "hscorgrefdataxml_data_8.0.0_20260828000001.zip");

    let out_dir = tmp.path().join("out");
    fs::create_dir_all(&out_dir).unwrap();

    let output = ods_binary()
        .current_dir(tmp.path())
        .arg("make")
        .arg("-i")
        .arg(&zip_path)
        .arg("-o")
        .arg(&out_dir)
        .output()
        .expect("execute ods make");

    assert!(output.status.success(), "ods make must succeed on bare archive");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    // 1. Warning line on stderr naming the input path
    let expected_warning = format!("! No provenance info found for {}. Source is unverified.", zip_path.display());
    assert!(
        stderr.contains(&expected_warning),
        "stderr must contain unverified warning, got:\n{}",
        stderr
    );

    // 2. The warning waits until the report block has settled
    let warn_pos = stderr.find(&expected_warning).unwrap();
    let last_row_pos = stderr.find("  orgs          ").expect("the block's last row must appear in stderr");
    assert!(
        warn_pos > last_row_pos,
        "the unverified warning must come after the block, got:\n{}",
        stderr
    );

    // 3. stdout has no warning or provenance line
    assert!(!stdout.contains("! No provenance info found"), "stdout must not contain warning");
    assert!(!stdout.contains("* Provenance:"), "stdout must not claim provenance file exists");

    // 4. Resulting _provenance.json records unverified
    let prov_content = fs::read_to_string(out_dir.join(ods::provenance::PROVENANCE_FILENAME)).unwrap();
    assert!(
        prov_content.contains(r#""trud_release_sha256_verified": "unverified""#),
        "_provenance.json must record trud_release_sha256_verified as unverified, got:\n{}",
        prov_content
    );

    // 5. ods make release refuses the unverified build
    let release_output = ods_binary()
        .current_dir(tmp.path())
        .env("CARGO_MANIFEST_DIR", env!("CARGO_MANIFEST_DIR"))
        .arg("make")
        .arg("release")
        .arg("-i")
        .arg(&out_dir)
        .output()
        .expect("execute ods make release");

    assert!(!release_output.status.success(), "ods make release must refuse unverified build");
    let release_stderr = String::from_utf8_lossy(&release_output.stderr);
    assert!(
        release_stderr.contains("source was never verified against TRUD") || release_stderr.contains("unverified"),
        "rejection must name unverified reason, got:\n{}",
        release_stderr
    );
}

#[test]
fn test_directory_input_without_provenance_fails_with_hard_error() {
    let tmp = TempDir::new().unwrap();
    let input_dir = tmp.path().join("dir_without_prov");
    fs::create_dir_all(&input_dir).unwrap();
    create_mock_zip(&input_dir, "hscorgrefdataxml_data_8.0.0_20260828000001.zip");

    let out_dir = tmp.path().join("out");
    fs::create_dir_all(&out_dir).unwrap();

    let output = ods_binary()
        .current_dir(tmp.path())
        .arg("make")
        .arg("-i")
        .arg(&input_dir)
        .arg("-o")
        .arg(&out_dir)
        .output()
        .expect("execute ods make");

    assert!(!output.status.success(), "directory input without provenance must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Missing _provenance.json in input directory") && stderr.contains("Did you run 'ods trud pull' first?"),
        "must bail with existing directory error, got:\n{}",
        stderr
    );
    assert!(
        !stderr.contains("! No provenance info found"),
        "must not print unverified warning for directory input"
    );
}

// ---------------------------------------------------------------------------
// Task 2: Name the file it used
// ---------------------------------------------------------------------------

#[test]
fn test_make_names_its_source_and_says_nothing_of_the_release_dirs_own_provenance() {
    let tmp = TempDir::new().unwrap();
    let ws_root = tmp.path().join("workspace");
    fs::create_dir_all(&ws_root).unwrap();

    // Setup releases/2026-08-28 directory with trud zip and _provenance.json
    let rel_dir = ws_root.join("releases").join("2026-08-28");
    let trud_dir = rel_dir.join("trud");
    fs::create_dir_all(&trud_dir).unwrap();
    let zip_path = create_mock_zip(&trud_dir, "hscorgrefdataxml_data_8.0.0_20260828000001.zip");
    let zip_sha256 = ods::provenance::compute_file_sha256(&zip_path).unwrap();

    let prov = ods::provenance::OdsProvenance {
        trud_release_date: Some("2026-08-28".to_string()),
        trud_release_sha256: Some(zip_sha256),
        trud_release_sha256_verified: Some(ods::provenance::TrudVerificationSource::TrudApi),
        trud_release_filesize_bytes: Some(37_000_000),
        ..Default::default()
    };

    let prov_file = rel_dir.join(ods::provenance::PROVENANCE_FILENAME);
    fs::write(&prov_file, serde_json::to_string_pretty(&prov).unwrap()).unwrap();

    // Open/create workspace and pin active release to 2026-08-28
    let ws = ods::workspace::Workspace::open_or_create(Some(&ws_root)).unwrap();
    ws.set_active("2026-08-28").unwrap();

    // Run `ods make` from workspace root with no arguments
    let output = ods_binary()
        .current_dir(&ws_root)
        .arg("make")
        .output()
        .expect("execute ods make");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "ods make must succeed, stderr:\n{}", stderr);

    // 1. The first line names the source, relative to the workspace, as `ods info` names its own
    assert_eq!(
        stderr.lines().next(),
        Some("* Source: releases/2026-08-28/trud/hscorgrefdataxml_data_8.0.0_20260828000001.zip"),
        "the first line names the archive read, got:\n{}",
        stderr
    );
    assert!(!stderr.contains("→"), "no '<date> (current) → <dir>' line, got:\n{}", stderr);

    // 2. The release directory's own _provenance.json is the expected one, so it goes unnamed
    assert!(
        ws_root.join("releases/2026-08-28/_provenance.json").exists(),
        "the release's own provenance exists on disk"
    );
    assert!(!stderr.contains("* Provenance:"), "the release's own provenance isn't named, got:\n{}", stderr);

    // 3. The report block follows the source line
    assert!(stderr.contains("  reading xml   "), "stderr must contain the report block, got:\n{}", stderr);

    // 4. Stdout is empty
    assert!(
        stdout.trim().is_empty(),
        "stdout must be empty, got:\n{}",
        stdout
    );

    // 5. Stderr does not contain ! warning
    assert!(
        !stderr.contains("! No provenance info found"),
        "stderr must not contain ! warning when provenance exists"
    );
}

#[test]
fn test_make_after_trud_pull_prints_no_warning() {
    let tmp = TempDir::new().unwrap();
    let zip_dir = tmp.path().join("prov_dir");
    fs::create_dir_all(&zip_dir).unwrap();
    let zip_path = create_mock_zip(&zip_dir, "hscorgrefdataxml_data_8.0.0_20260828000001.zip");
    let zip_sha256 = ods::provenance::compute_file_sha256(&zip_path).unwrap();

    let prov = ods::provenance::OdsProvenance {
        trud_release_date: Some("2026-08-28".to_string()),
        trud_release_sha256: Some(zip_sha256),
        trud_release_sha256_verified: Some(ods::provenance::TrudVerificationSource::TrudApi),
        trud_release_filesize_bytes: Some(37_000_000),
        ..Default::default()
    };
    fs::write(zip_dir.join(ods::provenance::PROVENANCE_FILENAME), serde_json::to_string_pretty(&prov).unwrap()).unwrap();

    let out_dir = tmp.path().join("out");
    fs::create_dir_all(&out_dir).unwrap();

    let output = ods_binary()
        .current_dir(tmp.path())
        .arg("make")
        .arg("-i")
        .arg(&zip_path)
        .arg("-o")
        .arg(&out_dir)
        .output()
        .expect("execute ods make");

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    // A provenance file from outside the release being built is named, after the source.
    assert!(stderr.contains("* Provenance:"), "a provenance from elsewhere is named:\n{stderr}");
    assert!(
        stderr.find("* Source:").unwrap() < stderr.find("* Provenance:").unwrap(),
        "the source comes first:\n{stderr}"
    );
    assert!(stdout.trim().is_empty());
    assert!(!stderr.contains("! No provenance info found"));
}

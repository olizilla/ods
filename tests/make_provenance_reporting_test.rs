//! Acceptance tests for `.agents/briefs/make-names-its-provenance.md`.
//!
//! Verifies:
//! - Task 1: Report an unverified source at the start when building from an archive file
//!   without `_provenance.json` above it. Directory input keeps existing hard error.
//!   `ods make release` refuses unverified output.
//! - Task 2: Name the `_provenance.json` file used on stdout (`* Provenance: <path>`)
//!   before `Generating dataset target projections (Parquet)...`.
//!   Streams are segregated (stdout gets `* Provenance:`, stderr gets `Generating...`).

mod common;

use common::create_mock_trud_zip as create_mock_zip;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn ods_binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ods"))
}

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

    // 2. Generating notice appears after the warning on stderr
    let warn_pos = stderr.find(&expected_warning).unwrap();
    let gen_pos = stderr.find("Generating dataset target projections (Parquet)...")
        .expect("Generating notice must appear in stderr");
    assert!(
        gen_pos > warn_pos,
        "Generating notice must appear after warning on stderr"
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
fn test_make_reports_provenance_file_on_stderr() {
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

    // 1. Stderr has * Provenance: releases/2026-08-28/_provenance.json
    assert!(
        stderr.contains("* Provenance: releases/2026-08-28/_provenance.json"),
        "stderr must name provenance file relative to workspace, got:\n{}",
        stderr
    );

    // 2. The named file exists on disk
    assert!(
        ws_root.join("releases/2026-08-28/_provenance.json").exists(),
        "named provenance file must exist on disk"
    );

    // 3. Stderr has Generating notice
    assert!(
        stderr.contains("Generating dataset target projections (Parquet)..."),
        "stderr must contain Generating notice, got:\n{}",
        stderr
    );

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
    assert!(stderr.contains("* Provenance:"));
    assert!(stdout.trim().is_empty());
    assert!(!stderr.contains("! No provenance info found"));
}

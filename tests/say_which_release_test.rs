//! Tests for `say-which-release.md` brief.
//!
//! Verifies:
//! - Task 1: Report release on find, info, role, cite; stdout untouched
//! - Task 2: cwd never selects release; current always selects release; non-ods_data workspace discovery
//! - Task 3: Name disagreement when cwd is another release dir; offer ods use <cwd_date>
//! - Task 4: Suppress release reporting on machine-readable formats (json, csv, tsv)
//! - Task 5: ods make release requires --input; ods make oci reports inferred release

mod common;

use common::setup_find_test_workspace;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

fn ods_binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ods"))
}

/// Sets up a test workspace at a custom (non-default) path with two releases:
/// - 2026-07-31 (current)
/// - 2026-06-26 (inactive)
fn setup_two_release_workspace() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().expect("create tempdir");
    let ws_root = tmp.path().join("custom_workspace");
    fs::create_dir_all(&ws_root).unwrap();

    let (_find_tmp, sample_parquet_dir) = setup_find_test_workspace();

    // 1. Create release 2026-07-31
    let rel_active = ws_root.join("releases").join("2026-07-31");
    fs::create_dir_all(&rel_active).unwrap();
    for entry in fs::read_dir(&sample_parquet_dir).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            fs::copy(entry.path(), rel_active.join(entry.file_name())).unwrap();
        }
    }
    let mut active_prov = ods::provenance::OdsProvenance::load_from_dir(&rel_active).unwrap_or_default();
    active_prov.trud_release_date = Some("2026-07-31".to_string());
    active_prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());
    active_prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);
    fs::write(
        rel_active.join(ods::provenance::PROVENANCE_FILENAME),
        serde_json::to_string_pretty(&active_prov).unwrap(),
    )
    .unwrap();
    fs::write(
        rel_active.join("datapackage.json"),
        b"{\"name\": \"ods\", \"version\": \"1.0.1\"}",
    )
    .unwrap();

    // 2. Create release 2026-06-26 with slightly different provenance
    let rel_older = ws_root.join("releases").join("2026-06-26");
    fs::create_dir_all(&rel_older).unwrap();
    for entry in fs::read_dir(&sample_parquet_dir).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            fs::copy(entry.path(), rel_older.join(entry.file_name())).unwrap();
        }
    }
    let mut older_prov = ods::provenance::OdsProvenance::load_from_dir(&rel_older).unwrap_or_default();
    older_prov.trud_release_date = Some("2026-06-26".to_string());
    older_prov.trud_release_sha256 = Some("7151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());
    older_prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);
    fs::write(
        rel_older.join(ods::provenance::PROVENANCE_FILENAME),
        serde_json::to_string_pretty(&older_prov).unwrap(),
    )
    .unwrap();
    fs::write(
        rel_older.join("datapackage.json"),
        b"{\"name\": \"ods\", \"version\": \"1.0.0\"}",
    )
    .unwrap();

    // 3. Setup workspace marker and pin current -> 2026-07-31
    let ws = ods::workspace::Workspace::open_or_create(Some(&ws_root)).unwrap();
    ws.set_active("2026-07-31").unwrap();

    (tmp, ws_root)
}

// ---------------------------------------------------------------------------
// Task 1: Report the release on every command that resolves one
// ---------------------------------------------------------------------------

#[test]
fn test_find_reports_release_in_header_source_line() {
    let (_tmp, ws_root) = setup_two_release_workspace();

    let output = ods_binary()
        .current_dir(&ws_root)
        .arg("find")
        .arg("sedbergh")
        .output()
        .expect("run ods find");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Stdout contains Source line naming active release
    assert!(
        stdout.contains("* Source: releases/2026-07-31/orgs.parquet"),
        "stdout should report active release in * Source: line, got:\n{}",
        stdout
    );
    assert!(stdout.contains("SEDBERGH MEDICAL PRACTICE"));
}

#[test]
fn test_info_reports_release_in_header_source_line() {
    let (_tmp, ws_root) = setup_two_release_workspace();

    let output = ods_binary()
        .current_dir(&ws_root)
        .arg("info")
        .arg("A82608")
        .output()
        .expect("run ods info");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("* Source: releases/2026-07-31/orgs.parquet"),
        "stdout should report active release in * Source: line, got:\n{}",
        stdout
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("(current)"),
        "stderr should not report release footer on info, got:\n{}",
        stderr
    );
}

#[test]
fn test_role_reports_release_in_header_source_line() {
    let (_tmp, ws_root) = setup_two_release_workspace();

    let output = ods_binary()
        .current_dir(&ws_root)
        .arg("role")
        .output()
        .expect("run ods role");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("* Source: releases/2026-07-31/orgs.parquet"),
        "stdout should report active release in * Source: line, got:\n{}",
        stdout
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("(current)"),
        "stderr should not report release footer on role, got:\n{}",
        stderr
    );
}

#[test]
fn test_cite_reports_release_in_header_source_line_on_stdout() {
    let (_tmp, ws_root) = setup_two_release_workspace();

    let output = ods_binary()
        .current_dir(&ws_root)
        .arg("cite")
        .output()
        .expect("run ods cite");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "ods cite failed with status {:?}:\nSTDOUT:\n{}\nSTDERR:\n{}",
        output.status.code(),
        stdout,
        stderr
    );
    assert!(
        stdout.contains("* Source: releases/2026-07-31 (1.0.1)"),
        "stdout should report active release in * Source: line, got:\n{}",
        stdout
    );
    assert!(
        !stderr.contains("(current)"),
        "stderr should not contain release resolution, got:\n{}",
        stderr
    );
}

// ---------------------------------------------------------------------------
// Task 2: The working directory never selects the release
// ---------------------------------------------------------------------------

#[test]
fn test_running_from_inside_older_release_reads_current_and_resolves_workspace() {
    let (_tmp, ws_root) = setup_two_release_workspace();
    let older_rel_dir = ws_root.join("releases").join("2026-06-26");

    // Run ods find from inside releases/2026-06-26/
    let output = ods_binary()
        .current_dir(&older_rel_dir)
        .arg("find")
        .arg("sedbergh")
        .output()
        .expect("run ods find");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Results must be read from current (2026-07-31), NOT 2026-06-26
    assert!(
        stdout.contains("* Source: releases/2026-07-31/orgs.parquet"),
        "must read from current release, got:\n{}",
        stdout
    );
}

// ---------------------------------------------------------------------------
// Task 3: Name the disagreement
// ---------------------------------------------------------------------------

#[test]
fn test_task3_disagreement_lines_appear_only_when_in_different_release_dir() {
    let (_tmp, ws_root) = setup_two_release_workspace();
    let older_rel_dir = ws_root.join("releases").join("2026-06-26");

    // 1. Run from inside older release: must show ! disagreement line directly under Source
    let output_disagree = ods_binary()
        .current_dir(&older_rel_dir)
        .arg("find")
        .arg("sedbergh")
        .output()
        .expect("run ods find");

    assert!(output_disagree.status.success());
    let stdout_disagree = String::from_utf8_lossy(&output_disagree.stdout);

    let expected_disagree = "! Run from releases/2026-06-26. Change source with: ods use 2026-06-26";
    assert!(
        stdout_disagree.contains(expected_disagree),
        "find must name the disagreement with ! sigil, got:\n{}",
        stdout_disagree
    );

    let info_disagree = ods_binary()
        .current_dir(&older_rel_dir)
        .arg("info")
        .arg("A82608")
        .output()
        .expect("run ods info");
    assert!(info_disagree.status.success());
    let stdout_info_disagree = String::from_utf8_lossy(&info_disagree.stdout);
    assert!(
        stdout_info_disagree.contains(expected_disagree),
        "info must name the disagreement with ! sigil, got:\n{}",
        stdout_info_disagree
    );

    let role_disagree = ods_binary()
        .current_dir(&older_rel_dir)
        .arg("role")
        .output()
        .expect("run ods role");
    assert!(role_disagree.status.success());
    let stdout_role_disagree = String::from_utf8_lossy(&role_disagree.stdout);
    assert!(
        stdout_role_disagree.contains(expected_disagree),
        "role must name the disagreement with ! sigil, got:\n{}",
        stdout_role_disagree
    );

    let cite_disagree = ods_binary()
        .current_dir(&older_rel_dir)
        .arg("cite")
        .output()
        .expect("run ods cite");
    assert!(cite_disagree.status.success());
    let stdout_cite_disagree = String::from_utf8_lossy(&cite_disagree.stdout);
    let stderr_cite_disagree = String::from_utf8_lossy(&cite_disagree.stderr);
    assert!(
        stdout_cite_disagree.contains(expected_disagree),
        "cite must name the disagreement with ! sigil, got:\n{}",
        stdout_cite_disagree
    );
    assert!(
        !stderr_cite_disagree.contains("! Run from releases/2026-06-26"),
        "cite stderr must not contain disagreement line, got:\n{}",
        stderr_cite_disagree
    );
    assert!(
        !stderr_cite_disagree.contains("(current)"),
        "cite stderr must not contain release resolution, got:\n{}",
        stderr_cite_disagree
    );

    // 2. Run from workspace root: normal run, NO disagreement line
    let output_normal = ods_binary()
        .current_dir(&ws_root)
        .arg("find")
        .arg("sedbergh")
        .output()
        .expect("run ods find");

    assert!(output_normal.status.success());
    let stdout_normal = String::from_utf8_lossy(&output_normal.stdout);
    let stderr_normal = String::from_utf8_lossy(&output_normal.stderr);
    assert!(
        !stdout_normal.contains("! Run from releases/"),
        "normal run must not contain disagreement line, got:\n{}",
        stdout_normal
    );
    assert!(
        !stderr_normal.contains("not the"),
        "normal run must not contain disagreement"
    );
    assert!(
        !stderr_normal.contains("Switch with:"),
        "normal run must not contain switch suggestion"
    );

    // 3. Run from current release dir: matches current, NO disagreement
    let current_rel_dir = ws_root.join("releases").join("2026-07-31");
    let output_current = ods_binary()
        .current_dir(&current_rel_dir)
        .arg("find")
        .arg("sedbergh")
        .output()
        .expect("run ods find");

    assert!(output_current.status.success());
    let stderr_current = String::from_utf8_lossy(&output_current.stderr);
    assert!(
        !stderr_current.contains("not the"),
        "running from current release dir must not report disagreement"
    );
}

// ---------------------------------------------------------------------------
// Task 4: Never on machine-readable output
// ---------------------------------------------------------------------------

#[test]
fn test_machine_readable_formats_suppress_release_and_disagreement_reporting() {
    let (_tmp, ws_root) = setup_two_release_workspace();
    let older_rel_dir = ws_root.join("releases").join("2026-06-26");

    for format in ["json", "csv", "tsv"] {
        // Run from workspace root (no disagreement)
        let out_root = ods_binary()
            .current_dir(&ws_root)
            .args(["find", "sedbergh", "--format", format])
            .output()
            .unwrap();

        // Run from older release dir (disagreement)
        let out_disagree = ods_binary()
            .current_dir(&older_rel_dir)
            .args(["find", "sedbergh", "--format", format])
            .output()
            .unwrap();

        assert!(out_root.status.success());
        assert!(out_disagree.status.success());

        // Both stdout and stderr must be byte-identical
        assert_eq!(
            out_root.stdout, out_disagree.stdout,
            "stdout for format {} must be byte-identical with and without disagreement",
            format
        );
        assert_eq!(
            out_root.stderr, out_disagree.stderr,
            "stderr for format {} must be byte-identical with and without disagreement",
            format
        );

        // Stderr must not contain the release report or switch suggestion
        let stderr_str = String::from_utf8_lossy(&out_disagree.stderr);
        assert!(
            !stderr_str.contains("(current)"),
            "stderr for format {} must not contain release report",
            format
        );
        assert!(
            !stderr_str.contains("Switch with:"),
            "stderr for format {} must not contain switch suggestion",
            format
        );
    }

    // Also verify `role` (csv, json) and `info` (json)
    for (cmd, args) in [
        ("role", vec!["role", "--format", "csv"]),
        ("role", vec!["role", "--format", "json"]),
        ("info", vec!["info", "A82608", "--format", "json"]),
    ] {
        let out_root = ods_binary()
            .current_dir(&ws_root)
            .args(&args)
            .output()
            .unwrap();

        let out_disagree = ods_binary()
            .current_dir(&older_rel_dir)
            .args(&args)
            .output()
            .unwrap();

        assert!(out_root.status.success(), "{} failed from root", cmd);
        assert!(out_disagree.status.success(), "{} failed from disagree", cmd);

        assert_eq!(
            out_root.stdout, out_disagree.stdout,
            "stdout for {:?} must be byte-identical with and without disagreement",
            args
        );
        assert_eq!(
            out_root.stderr, out_disagree.stderr,
            "stderr for {:?} must be byte-identical with and without disagreement",
            args
        );
        let stderr_str = String::from_utf8_lossy(&out_disagree.stderr);
        assert!(
            !stderr_str.contains("(current)"),
            "stderr for {:?} must not contain release report",
            args
        );
        assert!(
            !stderr_str.contains("Switch with:"),
            "stderr for {:?} must not contain switch suggestion",
            args
        );
    }
}

#[test]
fn test_explicit_input_to_non_current_release_prints_bare_date_without_current() {
    let (_tmp, ws_root) = setup_two_release_workspace();
    let older_rel_dir = ws_root.join("releases").join("2026-06-26");

    // ods cite -i <older_rel_dir>
    let cite_output = ods_binary()
        .current_dir(&ws_root)
        .args(["cite", "-i", older_rel_dir.to_str().unwrap()])
        .output()
        .expect("run ods cite with explicit -i");

    assert!(cite_output.status.success());
    let cite_stdout = String::from_utf8_lossy(&cite_output.stdout);
    assert!(
        cite_stdout.contains("* Source: releases/2026-06-26 (1.0.0)"),
        "stdout should report release in * Source: line, got:\n{}",
        cite_stdout
    );
    assert!(
        !cite_stdout.contains("(current)"),
        "non-current release must not be labelled (current), got:\n{}",
        cite_stdout
    );
    let cite_stderr = String::from_utf8_lossy(&cite_output.stderr);
    assert!(
        !cite_stderr.contains("2026-06-26"),
        "stderr should not report release date, got:\n{}",
        cite_stderr
    );

    // ods find -i <older_rel_dir> sedbergh
    let find_output = ods_binary()
        .current_dir(&ws_root)
        .args(["find", "-i", older_rel_dir.to_str().unwrap(), "sedbergh"])
        .output()
        .expect("run ods find with explicit -i");

    assert!(find_output.status.success());
    let find_stdout = String::from_utf8_lossy(&find_output.stdout);
    assert!(
        find_stdout.contains("* Source: releases/2026-06-26/orgs.parquet"),
        "stdout should report release in * Source: line, got:\n{}",
        find_stdout
    );
    assert!(
        !find_stdout.contains("(current)"),
        "non-current release must not be labelled (current), got:\n{}",
        find_stdout
    );
}

// ---------------------------------------------------------------------------
// Task 5: Scale inference to consequence
// ---------------------------------------------------------------------------

#[test]
fn test_make_release_refuses_without_input() {
    let (_tmp, ws_root) = setup_two_release_workspace();

    let output = ods_binary()
        .current_dir(&ws_root)
        .arg("make")
        .arg("release")
        .output()
        .expect("run ods make release");

    assert!(!output.status.success(), "ods make release must fail without --input");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--input"),
        "rejection must name the --input flag, got:\n{}",
        stderr
    );
    assert!(
        stderr.contains("✖ --input is required for 'ods make release'"),
        "rejection must follow house error style, got:\n{}",
        stderr
    );
}

#[test]
fn test_make_oci_reports_inferred_release_and_directory_written() {
    let (_tmp, ws_root) = setup_two_release_workspace();

    let output = ods_binary()
        .current_dir(&ws_root)
        .arg("make")
        .arg("oci")
        .output()
        .expect("run ods make oci");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("  2026-07-31 (current) → "),
        "ods make oci must report inferred release and written directory, got:\n{}",
        stderr
    );
}

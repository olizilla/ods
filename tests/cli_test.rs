//! End-to-end CLI integration tests verifying binary execution and stdout/stderr output formatting.

use std::process::Command;

fn ods_binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ods"))
}

#[test]
fn test_cli_help_displays_subcommands() {
    let output = ods_binary()
        .arg("--help")
        .output()
        .expect("Failed to execute binary");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("find"));
    assert!(stdout.contains("pull"));
    assert!(stdout.contains("cite"));
    assert!(stdout.contains("trud"));
    assert!(stdout.contains("make"));
}

#[test]
fn test_cli_pull_list_output_formatting() {
    let output = ods_binary()
        .arg("pull")
        .arg("--list")
        .output()
        .expect("Failed to execute pull --list");

    // This exercises the real binary, so whether the release index is reachable
    // depends on the network and on whether any release has been published yet.
    // That is not what this test is about: it checks output formatting, and the
    // formatting must hold either way. An unreachable index is a deliberate
    // non-zero exit (see `AlreadyReported`), so accept 0 or 1 and reject
    // anything else, which would mean a crash or signal.
    let code = output.status.code();
    assert!(
        matches!(code, Some(0) | Some(1)),
        "expected a clean exit or a reported failure, got {code:?}"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(stderr.contains("Querying available ODS dataset releases"));
    assert!(stderr.contains("Legend:"));
    assert!(!stdout.is_empty() || !stderr.is_empty());
}

#[test]
fn test_cli_cite_output_formatting() {
    let output = ods_binary()
        .arg("cite")
        .output()
        .expect("Failed to execute cite");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}{}", stdout, stderr);
    assert!(combined.contains("How to Cite") || combined.contains("Source") || combined.contains("ODS"));
}

#[test]
fn test_cli_make_help() {
    let output = ods_binary()
        .arg("make")
        .arg("--help")
        .output()
        .expect("Failed to execute make --help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("parquet"));
    assert!(stdout.contains("markdown"));
}

#[test]
fn test_cli_trud_help() {
    let output = ods_binary()
        .arg("trud")
        .arg("--help")
        .output()
        .expect("Failed to execute trud --help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("pull"));
    assert!(stdout.contains("diff"));
}

#[test]
fn test_cli_pull_help_has_no_api_key() {
    let output = ods_binary()
        .arg("pull")
        .arg("--help")
        .output()
        .expect("Failed to execute pull --help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("[RELEASE_DATE]"),
        "pull --help must have positional [RELEASE_DATE], got:\n{}",
        stdout
    );
    assert!(
        !stdout.contains("--api-key"),
        "pull --help must NOT have --api-key flag, got:\n{}",
        stdout
    );
}

#[test]
fn test_cli_trud_pull_help_has_positional_release_and_no_release_flag() {
    let output = ods_binary()
        .arg("trud")
        .arg("pull")
        .arg("--help")
        .output()
        .expect("Failed to execute trud pull --help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("[RELEASE_DATE]"),
        "trud pull --help must have positional [RELEASE_DATE], got:\n{}",
        stdout
    );
    assert!(
        !stdout.contains("--release <"),
        "trud pull --help must NOT have --release flag, got:\n{}",
        stdout
    );
}

#[test]
fn test_cli_pull_local_release_output() {
    let output = ods_binary()
        .arg("pull")
        .arg("2026-05-29")
        .output()
        .expect("Failed to execute pull 2026-05-29");

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("* Release 2026-05-29 already local, verified"),
        "stderr must contain '* Release 2026-05-29 already local, verified', got:\n{}",
        stderr
    );
    assert!(
        stderr.contains("✓ current updated to releases/2026-05-29"),
        "stderr must contain '✓ current updated to releases/2026-05-29', got:\n{}",
        stderr
    );

    // Restore 2026-07-31 active pointer
    let _ = ods_binary()
        .arg("pull")
        .arg("2026-07-31")
        .output();
}


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

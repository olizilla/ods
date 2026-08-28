//! End-to-end CLI integration tests verifying binary execution and stdout/stderr output formatting.

use sha2::Digest;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

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
    assert!(stdout.contains("info"));
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

    if output.status.success() {
        assert!(!stdout.is_empty(), "pull --list must output rows to stdout when successful");
        assert!(!stderr.contains("Legend:"), "piped --list must not output TTY legend to stderr");
    } else {
        assert!(!stderr.is_empty(), "reported failure must write diagnostics to stderr");
    }
}

#[test]
fn test_cli_cite_output_formatting() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    let rel_dir = ws.join("releases").join("2026-07-31");
    fs::create_dir_all(&rel_dir).unwrap();

    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_name = Some("Release 7.0.0".to_string());
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_file = Some("hscorgrefdataxml_data_7.0.0_20260731000001.zip".to_string());
    prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);
    prov.publication_date = Some("2026-07-28".to_string());
    prov.publication_seq_num = Some("4700".to_string());
    prov.publication_type = Some("Full".to_string());
    prov.publication_record_count = Some(305541);
    fs::write(rel_dir.join("orgs.parquet"), b"dummy").unwrap();
    fs::write(
        rel_dir.join(ods::provenance::PROVENANCE_FILENAME),
        serde_json::to_string_pretty(&prov).unwrap(),
    )
    .unwrap();

    ods::workspace::set_active_release(&ws, "2026-07-31").unwrap();

    let output = ods_binary()
        .current_dir(tmp.path())
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
    assert!(!stdout.contains("markdown"));
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
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    fs::create_dir_all(&ws).unwrap();

    let rel1 = ws.join("releases").join("2026-05-29");
    fs::create_dir_all(&rel1).unwrap();
    let p1 = rel1.join("orgs.parquet");
    fs::write(&p1, b"dummy parquet 1").unwrap();
    let h1 = format!("{:x}", sha2::Sha256::digest(b"dummy parquet 1"));
    fs::write(rel1.join("SHA256SUMS"), format!("{} *orgs.parquet\n", h1)).unwrap();

    let rel2 = ws.join("releases").join("2026-06-26");
    fs::create_dir_all(&rel2).unwrap();
    let p2 = rel2.join("orgs.parquet");
    fs::write(&p2, b"dummy parquet 2").unwrap();
    let h2 = format!("{:x}", sha2::Sha256::digest(b"dummy parquet 2"));
    fs::write(rel2.join("SHA256SUMS"), format!("{} *orgs.parquet\n", h2)).unwrap();

    // Ensure starting pin is 2026-06-26 so switching to 2026-05-29 moves the pin
    ods::workspace::set_active_release(&ws, "2026-06-26").unwrap();

    let output = ods_binary()
        .current_dir(tmp.path())
        .arg("pull")
        .arg("2026-05-29")
        .output()
        .expect("Failed to execute pull 2026-05-29");

    assert!(output.status.success(), "stderr was: {}", String::from_utf8_lossy(&output.stderr));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("* Release 2026-05-29 already local, verified"),
        "stderr must contain '* Release 2026-05-29 already local, verified', got:\n{}",
        stderr
    );

    assert!(
        stderr.contains("current → releases/2026-05-29"),
        "stderr must contain 'current → releases/2026-05-29', got:\n{}",
        stderr
    );
}

#[test]
fn test_cli_find_empty_workspace_message() {
    let tmp = TempDir::new().unwrap();

    let output = ods_binary()
        .current_dir(tmp.path())
        .arg("find")
        .arg("sedbergh")
        .output()
        .expect("Failed to execute find");

    assert!(!output.status.success(), "find must exit non-zero on empty workspace");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("✖ No dataset found in ods_data/current"),
        "stderr must contain '✖ No dataset found in ods_data/current', got:\n{}",
        stderr
    );
    assert!(
        stderr.contains("Run `ods pull` to download a release, then `ods use <date>` to pin it"),
        "stderr must advise running ods pull and ods use, got:\n{}",
        stderr
    );
}

#[test]
fn test_cli_cite_empty_workspace_message() {
    let tmp = TempDir::new().unwrap();

    let output = ods_binary()
        .current_dir(tmp.path())
        .arg("cite")
        .output()
        .expect("Failed to execute cite");

    assert!(!output.status.success(), "cite must exit non-zero on empty workspace");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("✖ No dataset found in ods_data/current"),
        "stderr must contain '✖ No dataset found in ods_data/current', got:\n{}",
        stderr
    );
    assert!(
        stderr.contains("Run `ods pull` to download a release, then `ods use <date>` to pin it"),
        "stderr must advise running ods pull and ods use, got:\n{}",
        stderr
    );
}

#[test]
fn test_cli_find_unpinned_workspace_message() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    let trud_dir = ws.join("releases").join("2026-07-31").join("trud");
    fs::create_dir_all(&trud_dir).unwrap();
    fs::write(trud_dir.join("hscorgrefdataxml_data_7.0.0_20260731000001.zip"), b"dummy").unwrap();

    let output = ods_binary()
        .current_dir(tmp.path())
        .arg("find")
        .arg("sedbergh")
        .output()
        .expect("Failed to execute find");

    assert!(!output.status.success(), "find must exit non-zero on unpinned workspace");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("✖ No active release pinned"),
        "stderr must contain '✖ No active release pinned', got:\n{}",
        stderr
    );
    assert!(
        stderr.contains("1 release in ods_data/releases/, none active."),
        "stderr must contain '1 release in ods_data/releases/, none active.', got:\n{}",
        stderr
    );
    assert!(
        stderr.contains("Pin one:  ods use 2026-07-31"),
        "stderr must contain 'Pin one:  ods use 2026-07-31', got:\n{}",
        stderr
    );
}

#[test]
fn test_cli_cite_unpinned_workspace_message() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    let trud_dir = ws.join("releases").join("2026-07-31").join("trud");
    fs::create_dir_all(&trud_dir).unwrap();
    fs::write(trud_dir.join("hscorgrefdataxml_data_7.0.0_20260731000001.zip"), b"dummy").unwrap();

    let output = ods_binary()
        .current_dir(tmp.path())
        .arg("cite")
        .output()
        .expect("Failed to execute cite");

    assert!(!output.status.success(), "cite must exit non-zero on unpinned workspace");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("✖ No active release pinned"),
        "stderr must contain '✖ No active release pinned', got:\n{}",
        stderr
    );
    assert!(
        stderr.contains("1 release in ods_data/releases/, none active."),
        "stderr must contain '1 release in ods_data/releases/, none active.', got:\n{}",
        stderr
    );
    assert!(
        stderr.contains("Pin one:  ods use 2026-07-31"),
        "stderr must contain 'Pin one:  ods use 2026-07-31', got:\n{}",
        stderr
    );
}

#[test]
fn test_cli_unpinned_workspace_multiple_releases_names_newest() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    fs::create_dir_all(ws.join("releases").join("2026-05-29").join("trud")).unwrap();
    fs::create_dir_all(ws.join("releases").join("2026-07-31").join("trud")).unwrap();

    let output = ods_binary()
        .current_dir(tmp.path())
        .arg("find")
        .arg("sedbergh")
        .output()
        .expect("Failed to execute find");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("2 releases in ods_data/releases/, none active."),
        "stderr must contain '2 releases in ods_data/releases/, none active.', got:\n{}",
        stderr
    );
    assert!(
        stderr.contains("Pin one:  ods use 2026-07-31"),
        "stderr must contain 'Pin one:  ods use 2026-07-31', got:\n{}",
        stderr
    );
}


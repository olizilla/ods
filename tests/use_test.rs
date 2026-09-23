mod common;

use anyhow::Result;
use common::make_v1_index;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn ods_cmd() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ods"))
}

#[test]
fn test_use_refuses_unknown_workspace_flag() {
    let tmp = TempDir::new().unwrap();
    let output = ods_cmd()
        .current_dir(tmp.path())
        .args(["use", "--workspace", "x", "2026-07-31"])
        .output()
        .expect("run ods use with --workspace");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unexpected argument '--workspace'")
            || stderr.contains("unexpected argument '--workspace' found")
            || stderr.contains("unknown argument '--workspace'"),
        "stderr must report unknown argument, got:\n{}",
        stderr
    );
}

#[test]
fn test_use_refuses_when_release_does_not_exist() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");
    fs::create_dir_all(&workspace).unwrap();
    ods::workspace::ensure_workspace_marker(&workspace).unwrap();

    let output = ods_cmd()
        .current_dir(tmp.path())
        .args(["use", "2026-07-31"])
        .output()
        .expect("run ods use");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Release 2026-07-31 not found"), "got:\n{}", stderr);
    assert!(stderr.contains("Run 'ods pull 2026-07-31' to download it"), "got:\n{}", stderr);
}

#[test]
fn test_use_pins_mismatched_release_and_warns() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");
    let rel_dir = workspace.join("releases").join("2026-07-31");
    fs::create_dir_all(&rel_dir).unwrap();

    let prov = ods::provenance::OdsProvenance {
        trud_release_date: Some("2026-07-31".to_string()),
        ..Default::default()
    };
    fs::write(
        rel_dir.join(ods::provenance::PROVENANCE_FILENAME),
        serde_json::to_string_pretty(&prov).unwrap(),
    )
    .unwrap();
    fs::write(rel_dir.join("datapackage.json"), b"{\"name\": \"ods\", \"version\": \"1.0.1\"}").unwrap();
    fs::write(rel_dir.join("orgs.parquet"), b"dummy content").unwrap();

    // Cache an index with a different manifest digest
    let index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("1.0.1", "sha256:0000000000000000000000000000000000000000000000000000000000000000")],
    )]);
    let index_bytes = serde_json::to_vec_pretty(&index).unwrap();
    ods::index::OdsReleaseIndex::save_to_workspace_bytes(&index_bytes, &workspace).unwrap();

    let output = ods_cmd()
        .current_dir(tmp.path())
        .args(["use", "2026-07-31"])
        .output()
        .expect("run ods use");

    assert!(output.status.success(), "use must exit 0 on mismatch, got: {:?}", output);

    let ws = ods::workspace::Workspace::open(Some(&workspace)).unwrap();
    let (active_date, path) = ws.active_release().unwrap();
    assert_eq!(active_date, "2026-07-31");
    assert_eq!(path, fs::canonicalize(&rel_dir).unwrap());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("✓ Active release set to 2026-07-31"), "got:\n{}", stderr);
    assert!(stderr.contains("  current → releases/2026-07-31"), "got:\n{}", stderr);
    assert!(stderr.contains("! releases/2026-07-31 does not match the published 2026-07-31 (1.0.1)"), "got:\n{}", stderr);
    assert!(stderr.contains("  expected manifest sha256:0000000000000000000000000000000000000000000000000000000000000000"), "got:\n{}", stderr);
    assert!(stderr.contains("  got      sha256:"), "got:\n{}", stderr);
    assert!(stderr.contains("  Repair it: ods pull --force 2026-07-31"), "got:\n{}", stderr);
}

#[test]
fn test_use_pins_corrupted_release_and_warns() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");
    let rel_dir = workspace.join("releases").join("2026-07-31");
    fs::create_dir_all(&rel_dir).unwrap();
    ods::workspace::ensure_workspace_marker(&workspace).unwrap();

    // No _provenance.json -> Corrupted("Missing or unreadable _provenance.json")
    let output = ods_cmd()
        .current_dir(tmp.path())
        .args(["use", "2026-07-31"])
        .output()
        .expect("run ods use");

    assert!(output.status.success(), "use must exit 0 on corrupted release, got: {:?}", output);

    let ws = ods::workspace::Workspace::open(Some(&workspace)).unwrap();
    let (active_date, path) = ws.active_release().unwrap();
    assert_eq!(active_date, "2026-07-31");
    assert_eq!(path, fs::canonicalize(&rel_dir).unwrap());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("✓ Active release set to 2026-07-31"), "got:\n{}", stderr);
    assert!(stderr.contains("  current → releases/2026-07-31"), "got:\n{}", stderr);
    assert!(stderr.contains("! releases/2026-07-31 can't be checked: Missing or unreadable _provenance.json"), "got:\n{}", stderr);
    assert!(stderr.contains("  Repair it: ods pull --force 2026-07-31"), "got:\n{}", stderr);
}

#[test]
fn test_use_pins_verified_release_and_creates_current_link() -> Result<()> {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");
    let rel_dir = workspace.join("releases").join("2026-07-31");
    fs::create_dir_all(&rel_dir).unwrap();

    let prov = ods::provenance::OdsProvenance {
        trud_release_date: Some("2026-07-31".to_string()),
        ..Default::default()
    };
    fs::write(
        rel_dir.join(ods::provenance::PROVENANCE_FILENAME),
        serde_json::to_string_pretty(&prov)?,
    )?;
    fs::write(rel_dir.join("datapackage.json"), b"{\"name\": \"ods\", \"version\": \"1.0.1\"}")?;

    let orgs_bytes = b"sample orgs parquet bytes";
    fs::write(rel_dir.join("orgs.parquet"), orgs_bytes)?;

    // 1. Run with unpublished local release: outputs "* reconstructed manifest ... verified (unpublished local release)"
    let output1 = ods_cmd()
        .current_dir(tmp.path())
        .args(["use", "2026-07-31"])
        .output()
        .expect("run ods use");
    assert!(output1.status.success());

    let current = workspace.join("current");
    assert!(current.exists());
    let (active_date, path) = ods::workspace::Workspace::open(Some(&workspace))?.active_release()?;
    assert_eq!(active_date, "2026-07-31");
    assert_eq!(path, fs::canonicalize(&rel_dir)?);

    let stderr1 = String::from_utf8_lossy(&output1.stderr);
    assert!(stderr1.contains("✓ Active release set to 2026-07-31"), "got:\n{}", stderr1);
    assert!(stderr1.contains("  current → releases/2026-07-31"), "got:\n{}", stderr1);
    assert!(stderr1.contains("* reconstructed manifest") && stderr1.contains("verified (unpublished local release)"), "got:\n{}", stderr1);

    // 2. Compute reconstructed manifest digest, add matching index entry, run again:
    // outputs "✓ reconstructed manifest ... matches the index for 2026-07-31 (1.0.1)"
    let (m, _) = ods::commands::make_oci::build_manifest_from_dir(&rel_dir, &prov, "1.0.1")?;
    let digest = m.digest()?;
    let index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("1.0.1", &digest)],
    )]);
    ods::index::OdsReleaseIndex::save_to_workspace_bytes(
        &serde_json::to_vec_pretty(&index)?,
        &workspace,
    )?;

    let output2 = ods_cmd()
        .current_dir(tmp.path())
        .args(["use", "2026-07-31"])
        .output()
        .expect("run ods use second time");
    assert!(output2.status.success());

    let stderr2 = String::from_utf8_lossy(&output2.stderr);
    assert!(stderr2.contains("* Release 2026-07-31 already active"), "got:\n{}", stderr2);
    assert!(stderr2.contains("  current → releases/2026-07-31"), "got:\n{}", stderr2);
    assert!(stderr2.contains(&format!("✓ reconstructed manifest {} matches the index for 2026-07-31 (1.0.1)", digest)), "got:\n{}", stderr2);

    // Verify workspace README was generated
    let readme = workspace.join("README.md");
    assert!(readme.exists());
    let readme_content = fs::read_to_string(readme)?;
    assert!(readme_content.contains("2026-07-31"));

    Ok(())
}

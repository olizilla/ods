use crate::common;

use anyhow::Result;
use common::{make_v1_index, ods_cmd};
use std::fs;
use tempfile::TempDir;

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

    common::write_fixture_parquet(&rel_dir.join("orgs.parquet"), "2026-07-31", "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933", "1.0.1", "dummy content");

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

    // No Parquet files at all: pinned, and said so
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
    assert!(
        stderr.contains("! releases/2026-07-31 can't be checked: it holds no Parquet files"),
        "got:\n{}",
        stderr
    );
    assert!(
        stderr.contains("  Repair it: ods pull --force 2026-07-31"),
        "got:\n{}",
        stderr
    );
}

#[test]
fn test_use_pins_verified_release_and_creates_current_link() -> Result<()> {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");
    let rel_dir = workspace.join("releases").join("2026-07-31");
    fs::create_dir_all(&rel_dir).unwrap();

    common::write_fixture_parquet(&rel_dir.join("orgs.parquet"), "2026-07-31", "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933", "1.0.1", "sample orgs parquet bytes");

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
    let (m, _) = ods::commands::make_oci::build_manifest_from_dir(&rel_dir)?;
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

#[test]
fn test_use_different_archive_warns_when_version_published_and_verifies_when_not() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");
    let rel_dir = workspace.join("releases").join("2026-07-31");
    fs::create_dir_all(&rel_dir).unwrap();

    let other_sha = "1111111111111111111111111111111111111111111111111111111111111111";

    // Cache an index with a published version 1.0.1 for 2026-07-31
    let index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("1.0.1", "sha256:0000000000000000000000000000000000000000000000000000000000000000")],
    )]);
    let index_bytes = serde_json::to_vec_pretty(&index).unwrap();
    ods::index::OdsReleaseIndex::save_to_workspace_bytes(&index_bytes, &workspace).unwrap();

    // 1. Version 1.0.1 is published: ods use must warn mismatch
    common::write_fixture_parquet(&rel_dir.join("orgs.parquet"), "2026-07-31", other_sha, "1.0.1", "dummy content");
    let output1 = ods_cmd()
        .current_dir(tmp.path())
        .args(["use", "2026-07-31"])
        .output()
        .expect("run ods use");
    assert!(output1.status.success());
    let stderr1 = String::from_utf8_lossy(&output1.stderr);
    assert!(stderr1.contains("! releases/2026-07-31 does not match the published 2026-07-31 (1.0.1)"), "got:\n{}", stderr1);
    assert!(stderr1.contains("  expected manifest sha256:0000000000000000000000000000000000000000000000000000000000000000"), "got:\n{}", stderr1);
    assert!(stderr1.contains("  Repair it: ods pull --force 2026-07-31"), "got:\n{}", stderr1);

    // 2. Version 0.9.0 is NOT published: ods use reports verified unpublished local release without warning
    common::write_fixture_parquet(&rel_dir.join("orgs.parquet"), "2026-07-31", other_sha, "0.9.0", "dummy content");
    let output2 = ods_cmd()
        .current_dir(tmp.path())
        .args(["use", "2026-07-31"])
        .output()
        .expect("run ods use with unpublished version");
    assert!(output2.status.success());
    let stderr2 = String::from_utf8_lossy(&output2.stderr);
    assert!(stderr2.contains("* reconstructed manifest") && stderr2.contains("verified (unpublished local release)"), "got:\n{}", stderr2);
    assert!(!stderr2.contains("! releases/2026-07-31 does not match"), "got:\n{}", stderr2);
}

#[test]
fn test_use_latest_pins_newest_local_release() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");
    ods::workspace::ensure_workspace_marker(&workspace).unwrap();

    for date in ["2026-07-31", "2026-08-28"] {
        let rel_dir = workspace.join("releases").join(date);
        fs::create_dir_all(&rel_dir).unwrap();
        common::write_fixture_parquet(&rel_dir.join("orgs.parquet"), date, "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933", "1.0.1", "dummy content");
    }
    // A non-date directory under releases/ must be ignored when picking "latest".
    fs::create_dir_all(workspace.join("releases").join("scratch")).unwrap();
    fs::write(workspace.join("releases").join("scratch").join("orgs.parquet"), b"dummy").unwrap();

    let output = ods_cmd()
        .current_dir(tmp.path())
        .args(["use", "latest"])
        .output()
        .expect("run ods use latest");

    assert!(output.status.success(), "got: {:?}", output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("✓ Active release set to 2026-08-28"), "got:\n{}", stderr);
    assert!(stderr.contains("  current → releases/2026-08-28"), "got:\n{}", stderr);

    let ws = ods::workspace::Workspace::open(Some(&workspace)).unwrap();
    let (active_date, _) = ws.active_release().unwrap();
    assert_eq!(active_date, "2026-08-28");
}

#[test]
fn test_use_refuses_non_date_argument_and_leaves_current_unchanged() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");
    let rel_dir = workspace.join("releases").join("2026-07-31");
    fs::create_dir_all(&rel_dir).unwrap();
    common::write_fixture_parquet(&rel_dir.join("orgs.parquet"), "2026-07-31", "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933", "1.0.1", "dummy content");
    ods::workspace::ensure_workspace_marker(&workspace).unwrap();

    // Pin a release first, so we can assert "yesterday" leaves it unchanged.
    let setup = ods_cmd()
        .current_dir(tmp.path())
        .args(["use", "2026-07-31"])
        .output()
        .expect("run ods use to set up an active release");
    assert!(setup.status.success(), "got: {:?}", setup);

    let output = ods_cmd()
        .current_dir(tmp.path())
        .args(["use", "yesterday"])
        .output()
        .expect("run ods use yesterday");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("✖ yesterday isn't a release date"), "got:\n{}", stderr);
    assert!(
        stderr.contains("  Name a release as YYYY-MM-DD, or pin the newest one here: ods use latest"),
        "got:\n{}",
        stderr
    );

    let ws = ods::workspace::Workspace::open(Some(&workspace)).unwrap();
    let (active_date, _) = ws.active_release().unwrap();
    assert_eq!(active_date, "2026-07-31", "a refused argument must not move the pin");
}

#[test]
fn test_use_latest_in_empty_workspace() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");
    ods::workspace::ensure_workspace_marker(&workspace).unwrap();

    let output = ods_cmd()
        .current_dir(tmp.path())
        .args(["use", "latest"])
        .output()
        .expect("run ods use latest in an empty workspace");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("✖ No releases in "), "got:\n{}", stderr);
    assert!(stderr.contains("ods_data"), "got:\n{}", stderr);
    assert!(stderr.contains("  Run: ods pull"), "got:\n{}", stderr);
}

/// Files that don't carry the same provenance can't say what release they are: `ods use`
/// refuses, naming them, and leaves the pin where it was.
#[test]
fn test_use_refuses_files_that_disagree_and_leaves_current_unchanged() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");
    ods::workspace::ensure_workspace_marker(&workspace).unwrap();
    for date in ["2026-06-26", "2026-07-31"] {
        let rel_dir = workspace.join("releases").join(date);
        fs::create_dir_all(&rel_dir).unwrap();
        common::write_fixture_parquet(&rel_dir.join("orgs.parquet"), date, "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933", "1.0.1", "orgs");
        common::write_fixture_parquet(&rel_dir.join("roles.parquet"), date, "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933", "1.0.1", "roles");
    }
    let setup = ods_cmd().current_dir(tmp.path()).args(["use", "2026-06-26"]).output().unwrap();
    assert!(setup.status.success(), "got: {:?}", setup);

    // orgs.parquet rewritten carrying another dataset version
    let rel_dir = workspace.join("releases").join("2026-07-31");
    common::write_fixture_parquet(&rel_dir.join("orgs.parquet"), "2026-07-31", "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933", "9.9.9", "orgs");

    let output = ods_cmd().current_dir(tmp.path()).args(["use", "2026-07-31"]).output().unwrap();
    assert!(!output.status.success(), "use must refuse, got: {:?}", output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("don't carry the same provenance"), "got:\n{}", stderr);
    assert!(stderr.contains("orgs.parquet   2026-07-31_9.9.9"), "got:\n{}", stderr);
    assert!(stderr.contains("roles.parquet  2026-07-31_1.0.1"), "got:\n{}", stderr);

    let ws = ods::workspace::Workspace::open(Some(&workspace)).unwrap();
    assert_eq!(ws.active_release().unwrap().0, "2026-06-26", "a refused release must not move the pin");
}

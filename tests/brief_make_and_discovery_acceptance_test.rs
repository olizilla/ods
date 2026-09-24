//! Per-task acceptance tests for `.agents/briefs/make-commands-and-workspace-discovery.md`.

mod common;

use common::{create_mock_trud_zip as create_mock_zip, ods_binary};
use std::fs;
use tempfile::TempDir;



/// Task 3 Acceptance: Workspace not named `ods_data` (e.g. `nhs-archive/`).
/// Discovers from inside workspace, from release dir, and when unpinned outside refuses with error.
#[test]
fn test_custom_named_workspace_discovery_from_inside_and_release_subdir() {
    let tmp = TempDir::new().unwrap();
    // Isolate from ascending past tmp
    fs::create_dir_all(tmp.path().join(".git")).unwrap();

    let ws = tmp.path().join("nhs-archive");
    let rel_dir = ws.join("releases").join("2026-07-31");
    let trud_dir = rel_dir.join("trud");
    fs::create_dir_all(&trud_dir).unwrap();

    let zip_filename = "hscorgrefdataxml_data_7.0.0_20260731000001.zip";
    let zip_path = create_mock_zip(&trud_dir, zip_filename);

    // Compile parquet in release dir
    let res = ods_binary()
        .arg("make")
        .arg("-i")
        .arg(&zip_path)
        .arg("-o")
        .arg(&rel_dir)
        .output()
        .expect("execute ods make");
    assert!(res.status.success());

    // Write valid _releases.json and pin current
    let mut prov = ods::provenance::OdsProvenance::load_from_dir(&rel_dir).unwrap_or_default();
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);
    fs::write(
        rel_dir.join(ods::provenance::PROVENANCE_FILENAME),
        serde_json::to_string_pretty(&prov).unwrap(),
    ).unwrap();

    fs::write(ws.join("_releases.json"), ods::index::BAKED_RELEASES_JSON_BYTES).unwrap();
    ods::workspace::Workspace::open_or_create(Some(&ws)).unwrap().set_active("2026-07-31").unwrap();

    // 1. Run `ods find` from inside nhs-archive/
    let out_find = ods_binary()
        .current_dir(&ws)
        .arg("find")
        .arg("RAE")
        .output()
        .expect("execute ods find from inside custom workspace");
    assert!(out_find.status.success(), "find must succeed from inside custom workspace");
    let stdout_find = String::from_utf8_lossy(&out_find.stdout);
    assert!(stdout_find.contains("RAE") || stdout_find.contains("ALDER HEY"), "find output must contain match");

    // 2. Run `ods cite` from inside nhs-archive/
    let out_cite = ods_binary()
        .current_dir(&ws)
        .arg("cite")
        .output()
        .expect("execute ods cite from inside custom workspace");
    let stdout_cite = String::from_utf8_lossy(&out_cite.stdout);
    let stderr_cite = String::from_utf8_lossy(&out_cite.stderr);
    assert!(
        out_cite.status.success(),
        "cite must succeed from inside custom workspace, got exit {:?}:\nstdout:\n{}\nstderr:\n{}",
        out_cite.status.code(),
        stdout_cite,
        stderr_cite
    );

    // 3. Run `ods cite` from inside the release subdirectory
    let out_cite_rel = ods_binary()
        .current_dir(&rel_dir)
        .arg("cite")
        .output()
        .expect("execute ods cite from release directory");
    assert!(out_cite_rel.status.success(), "cite must succeed from release dir");

    // 4. Run `ods make` in a directory with no workspace anywhere above it
    let empty_dir = tmp.path().join("empty_standalone");
    fs::create_dir_all(&empty_dir).unwrap();
    let out_empty = ods_binary()
        .current_dir(&empty_dir)
        .arg("make")
        .output()
        .expect("execute ods make in empty dir");

    assert!(!out_empty.status.success(), "make must fail when no workspace found");
    let stderr_empty = String::from_utf8_lossy(&out_empty.stderr);
    assert!(
        stderr_empty.contains("✖ no ods workspace found here"),
        "stderr must contain '✖ no ods workspace found here', got:\n{}",
        stderr_empty
    );
    assert!(
        stderr_empty.contains("Pass -i <trud.zip> -o <dir>, or run `ods pull` or `ods trud pull` to create a workspace."),
        "stderr must contain usage hint, got:\n{}",
        stderr_empty
    );
    // Ensure it did not create any parquet files in the empty dir
    assert!(!empty_dir.join("orgs.parquet").exists(), "make must not write to . when no workspace is found");
}

/// Task 4 Acceptance: `make` does not move `current` pointer.
#[test]
fn test_make_into_new_release_does_not_move_active_current_pointer() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    let rel_a = ws.join("releases").join("2026-05-29");
    let rel_b = ws.join("releases").join("2026-07-31");
    fs::create_dir_all(&rel_a).unwrap();
    fs::create_dir_all(&rel_b).unwrap();

    let zip_a = create_mock_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_20260529000001.zip");
    let zip_b = create_mock_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_20260731000001.zip");

    // Compile release A and pin it
    ods_binary()
        .arg("make")
        .arg("-i")
        .arg(&zip_a)
        .arg("-o")
        .arg(&rel_a)
        .output()
        .expect("make A");

    fs::write(ws.join("_releases.json"), ods::index::BAKED_RELEASES_JSON_BYTES).unwrap();
    ods::workspace::Workspace::open_or_create(Some(&ws)).unwrap().set_active("2026-05-29").unwrap();

    // Verify current is pinned to 2026-05-29
    let (active_date, _) = ods::workspace::Workspace::open(Some(&ws)).unwrap().active_release().unwrap();
    assert_eq!(active_date, "2026-05-29");

    // Now run `ods make` into release B
    let res_make_b = ods_binary()
        .arg("make")
        .arg("-i")
        .arg(&zip_b)
        .arg("-o")
        .arg(&rel_b)
        .output()
        .expect("make B");
    assert!(res_make_b.status.success(), "make B must succeed");
    assert!(rel_b.join("orgs.parquet").exists(), "release B parquet must be generated");

    // Assert that `current` STILL points to 2026-05-29
    let (current_after, _) = ods::workspace::Workspace::open(Some(&ws)).unwrap().active_release().unwrap();
    assert_eq!(
        current_after,
        "2026-05-29",
        "`current` pointer must remain pinned to release A and not moved to release B by `make`"
    );
}

/// Task 6 Acceptance: `ods audit --workspace <path> --full` against an unpinned external workspace succeeds.
#[test]
fn test_audit_workspace_flag_authoritative_on_unpinned_workspace() {
    let tmp = TempDir::new().unwrap();
    let external_ws = tmp.path().join("external_archive");
    let rel_dir = external_ws.join("releases").join("2026-07-31");
    let trud_dir = rel_dir.join("trud");
    fs::create_dir_all(&trud_dir).unwrap();

    let zip_filename = "hscorgrefdataxml_data_7.0.0_20260731000001.zip";
    let zip_path = create_mock_zip(&trud_dir, zip_filename);

    // Make parquet in release dir (which generates verified parquet + provenance)
    let res_make = ods_binary()
        .arg("make")
        .arg("-i")
        .arg(&zip_path)
        .arg("-o")
        .arg(&rel_dir)
        .output()
        .expect("make into external workspace");
    assert!(res_make.status.success());

    fs::write(external_ws.join("_releases.json"), ods::index::BAKED_RELEASES_JSON_BYTES).unwrap();

    // Verify that the workspace has NO active release pinned
    assert!(ods::workspace::Workspace::open(Some(&external_ws)).unwrap().active_release().is_err(), "workspace should be unpinned");

    // From a completely separate isolated working directory
    let isolated_cwd = tmp.path().join("isolated_cwd");
    fs::create_dir_all(&isolated_cwd).unwrap();

    let out_audit = ods_binary()
        .current_dir(&isolated_cwd)
        .arg("audit")
        .arg("--workspace")
        .arg(&external_ws)
        .arg("--all")
        .arg("--full")
        .output()
        .expect("execute ods audit --workspace <path> --all --full");

    let stdout_audit = String::from_utf8_lossy(&out_audit.stdout);
    let stderr_audit = String::from_utf8_lossy(&out_audit.stderr);
    let combined = format!("{}{}", stdout_audit, stderr_audit);

    assert!(
        out_audit.status.success(),
        "audit against external unpinned workspace must succeed, got exit status {:?}:\n{}",
        out_audit.status.code(),
        combined
    );
    assert!(
        combined.contains("audited") || combined.contains("1 audited"),
        "audit output must confirm release audited, got:\n{}",
        combined
    );
}

//! Per-task acceptance tests for `.agents/briefs/make-commands-and-workspace-discovery.md`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

const FIXTURE_XML: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/mock_hscorgrefdata.xml"
);

fn ods_binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ods"))
}

fn create_mock_zip(dir: &Path, filename: &str) -> PathBuf {
    let zip_path = dir.join(filename);
    let zip_file = fs::File::create(&zip_path).unwrap();
    let mut zip_writer = zip::ZipWriter::new(zip_file);
    let options = zip::write::SimpleFileOptions::default();
    zip_writer.start_file("HSCOrgRefData_Full_mock.xml", options).unwrap();
    let xml_content = fs::read_to_string(FIXTURE_XML).unwrap();
    std::io::Write::write_all(&mut zip_writer, xml_content.as_bytes()).unwrap();
    zip_writer.finish().unwrap();
    zip_path
}

/// Task 1 Acceptance: `ods make ndjson` is gone and returns unknown subcommand error.
#[test]
fn test_task_1_acceptance_make_ndjson_subcommand_is_gone() {
    let output = ods_binary()
        .arg("make")
        .arg("ndjson")
        .output()
        .expect("execute ods make ndjson");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unrecognized subcommand") || stderr.contains("invalid subcommand") || stderr.contains("error:"),
        "stderr should indicate unrecognized subcommand, got:\n{}",
        stderr
    );
}

/// Task 2 Acceptance: `ods make` ≡ `ods make parquet`, with identical output and unaffected by stray `ods.ndjson`.
#[test]
fn test_task_2_acceptance_make_identical_to_make_parquet_and_ignores_stray_ndjson() {
    let tmp = TempDir::new().unwrap();
    let zip_path = create_mock_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_20260731000001.zip");

    let out_make = tmp.path().join("out_make");
    let out_make_parquet = tmp.path().join("out_make_parquet");
    fs::create_dir_all(&out_make).unwrap();
    fs::create_dir_all(&out_make_parquet).unwrap();

    // 1. Run `ods make`
    let res_make = ods_binary()
        .arg("make")
        .arg("-i")
        .arg(&zip_path)
        .arg("-o")
        .arg(&out_make)
        .output()
        .expect("execute ods make");
    assert!(res_make.status.success(), "ods make must succeed");

    // 2. Run `ods make parquet`
    let res_make_parquet = ods_binary()
        .arg("make")
        .arg("parquet")
        .arg("-i")
        .arg(&zip_path)
        .arg("-o")
        .arg(&out_make_parquet)
        .output()
        .expect("execute ods make parquet");
    assert!(res_make_parquet.status.success(), "ods make parquet must succeed");

    // Assert all generated files are byte-for-byte identical
    let expected_files = [
        "orgs.parquet",
        "orgs_all.parquet",
        "roles.parquet",
        "relationships.parquet",
        "successions.parquet",
        "datapackage.json",
        ods::provenance::PROVENANCE_FILENAME,
    ];

    for file in &expected_files {
        let path1 = out_make.join(file);
        let path2 = out_make_parquet.join(file);
        assert!(path1.exists(), "file {} should exist in out_make", file);
        assert!(path2.exists(), "file {} should exist in out_make_parquet", file);

        let bytes1 = fs::read(&path1).unwrap();
        let bytes2 = fs::read(&path2).unwrap();
        assert_eq!(bytes1, bytes2, "file {} must be byte-for-byte identical between `make` and `make parquet`", file);
    }

    // 3. Create a stray ods.ndjson in cwd and rerun
    let stray_ndjson = tmp.path().join("ods.ndjson");
    fs::write(&stray_ndjson, b"{\"stray\": true}\n").unwrap();

    let out_make_stray = tmp.path().join("out_make_stray");
    let res_stray = ods_binary()
        .current_dir(tmp.path())
        .arg("make")
        .arg("-i")
        .arg(&zip_path)
        .arg("-o")
        .arg(&out_make_stray)
        .output()
        .expect("execute ods make with stray ndjson");
    assert!(res_stray.status.success(), "ods make with stray ndjson must succeed");

    for file in &expected_files {
        let path1 = out_make.join(file);
        let path2 = out_make_stray.join(file);
        let bytes1 = fs::read(&path1).unwrap();
        let bytes2 = fs::read(&path2).unwrap();
        assert_eq!(bytes1, bytes2, "stray ndjson must have no effect on output of {}", file);
    }
}

/// Task 3 Acceptance: Workspace not named `ods_data` (e.g. `nhs-archive/`).
/// Discovers from inside workspace, from release dir, and when unpinned outside refuses with error.
#[test]
fn test_task_3_acceptance_non_default_workspace_discovery() {
    let tmp = TempDir::new().unwrap();
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

    let cached = ods::index::CachedReleaseIndex {
        fetched_at: "2026-07-31T00:00:00Z".to_string(),
        index: ods::index::OdsReleaseIndex::baked().unwrap_or_default(),
    };
    cached.save_to_workspace(&ws).unwrap();
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
fn test_task_4_acceptance_make_does_not_move_current_pointer() {
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

    let cached = ods::index::CachedReleaseIndex {
        fetched_at: "2026-05-29T00:00:00Z".to_string(),
        index: ods::index::OdsReleaseIndex::baked().unwrap_or_default(),
    };
    cached.save_to_workspace(&ws).unwrap();
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
fn test_task_6_acceptance_audit_workspace_authoritative_on_unpinned_workspace() {
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

    let cached = ods::index::CachedReleaseIndex {
        fetched_at: "2026-07-31T00:00:00Z".to_string(),
        index: ods::index::OdsReleaseIndex::baked().unwrap_or_default(),
    };
    cached.save_to_workspace(&external_ws).unwrap();

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

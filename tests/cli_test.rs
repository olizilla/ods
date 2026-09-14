//! End-to-end CLI integration tests verifying binary execution and stdout/stderr output formatting.

use std::fs;
use std::process::Command;
use tempfile::TempDir;

mod common;

use common::create_mock_trud_zip;

fn ods_binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ods"))
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
    prov.dataset_version = Some("1.0.1".to_string());
    fs::write(rel_dir.join("orgs.parquet"), b"dummy").unwrap();
    fs::write(
        rel_dir.join(ods::provenance::PROVENANCE_FILENAME),
        serde_json::to_string_pretty(&prov).unwrap(),
    )
    .unwrap();

    ods::workspace::Workspace::open_or_create(Some(&ws)).unwrap().set_active("2026-07-31").unwrap();
    fs::write(ws.join("_releases.json"), ods::index::BAKED_RELEASES_JSON_BYTES).unwrap();

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
fn test_cli_pull_local_release_output() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    fs::create_dir_all(&ws).unwrap();

    let mut prov1 = ods::provenance::OdsProvenance::default();
    prov1.trud_release_date = Some("2026-05-29".to_string());
    prov1.dataset_version = Some("1.0.1".to_string());
    let rel1 = ws.join("releases").join("2026-05-29");
    fs::create_dir_all(&rel1).unwrap();
    fs::write(rel1.join(ods::provenance::PROVENANCE_FILENAME), serde_json::to_string_pretty(&prov1).unwrap()).unwrap();
    fs::write(rel1.join("orgs.parquet"), b"dummy parquet 1").unwrap();

    let (m1, _) = ods::commands::make_oci::build_manifest_from_dir(&rel1, &prov1, "1.0.1").unwrap();
    let d1 = m1.digest().unwrap();

    let mut prov2 = ods::provenance::OdsProvenance::default();
    prov2.trud_release_date = Some("2026-06-26".to_string());
    prov2.dataset_version = Some("1.0.1".to_string());
    let rel2 = ws.join("releases").join("2026-06-26");
    fs::create_dir_all(&rel2).unwrap();
    fs::write(rel2.join(ods::provenance::PROVENANCE_FILENAME), serde_json::to_string_pretty(&prov2).unwrap()).unwrap();
    fs::write(rel2.join("orgs.parquet"), b"dummy parquet 2").unwrap();

    let (m2, _) = ods::commands::make_oci::build_manifest_from_dir(&rel2, &prov2, "1.0.1").unwrap();
    let d2 = m2.digest().unwrap();

    let index = ods::index::OdsReleaseIndex {
        type_tag: "ods_release_index".to_string(),
        index_version: 2,
        concept_doi: None,
        mirrors: vec![],
        releases: vec![
            ods::index::ReleaseIndexEntry {
                trud_release_date: "2026-05-29".to_string(),
                dataset_version: "1.0.1".to_string(),
                tag: "2026-05-29_1.0.1".to_string(),
                manifest_digest: d1,
                trud_release_sha256: "AAAA".to_string(),
                tool_version: "0.4.3".to_string(),
                dataset_doi: None,
                withdrawn: None,
            },
            ods::index::ReleaseIndexEntry {
                trud_release_date: "2026-06-26".to_string(),
                dataset_version: "1.0.1".to_string(),
                tag: "2026-06-26_1.0.1".to_string(),
                manifest_digest: d2,
                trud_release_sha256: "BBBB".to_string(),
                tool_version: "0.4.3".to_string(),
                dataset_doi: None,
                withdrawn: None,
            },
        ],
    };
    let bytes = serde_json::to_vec_pretty(&index).unwrap();
    ods::index::OdsReleaseIndex::save_to_workspace_bytes(&bytes, &ws).unwrap();

    // Ensure starting pin is 2026-06-26 so switching to 2026-05-29 moves the pin
    ods::workspace::Workspace::open_or_create(Some(&ws)).unwrap().set_active("2026-06-26").unwrap();

    let output = ods_binary()
        .current_dir(tmp.path())
        .env("ODS_RELEASE_INDEX_URL", "http://127.0.0.1:9999/offline")
        .arg("pull")
        .arg("2026-05-29")
        .output()
        .expect("Failed to execute pull 2026-05-29");

    assert!(output.status.success(), "stderr was: {}", String::from_utf8_lossy(&output.stderr));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("verified (cache hit)"),
        "stderr must contain 'verified (cache hit)', got:\n{}",
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
        stderr.contains("✖ no ods workspace found here"),
        "stderr must contain '✖ no ods workspace found here', got:\n{}",
        stderr
    );
    assert!(
        stderr.contains("Pass -i <trud.zip> -o <dir>, or run `ods pull` or `ods trud pull` to create a workspace."),
        "stderr must advise passing -i/-o or running ods pull or ods trud pull, got:\n{}",
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
        stderr.contains("✖ no ods workspace found here"),
        "stderr must contain '✖ no ods workspace found here', got:\n{}",
        stderr
    );
    assert!(
        stderr.contains("Pass -i <trud.zip> -o <dir>, or run `ods pull` or `ods trud pull` to create a workspace."),
        "stderr must advise passing -i/-o or running ods pull or ods trud pull, got:\n{}",
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

    fs::write(ws.join("_releases.json"), ods::index::BAKED_RELEASES_JSON_BYTES).unwrap();

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

    fs::write(ws.join("_releases.json"), ods::index::BAKED_RELEASES_JSON_BYTES).unwrap();

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

    fs::write(ws.join("_releases.json"), ods::index::BAKED_RELEASES_JSON_BYTES).unwrap();

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



#[test]
fn test_read_commands_refuse_when_no_workspace_and_write_nothing() {
    let tmp = TempDir::new().unwrap();
    // Isolate from repository root so discovery doesn't ascend past tmp
    fs::create_dir_all(tmp.path().join(".git")).unwrap();

    let empty_dir = tmp.path().join("empty_project");
    fs::create_dir_all(&empty_dir).unwrap();

    let read_commands: Vec<&[&str]> = vec![
        &["cite"],
        &["find", "sedbergh"],
        &["info", "RAE01"],
        &["role", "RO177"],
        &["audit"],
        &["diff"],
    ];

    for cmd_args in read_commands {
        let mut cmd = ods_binary();
        cmd.current_dir(&empty_dir);
        for arg in cmd_args {
            cmd.arg(arg);
        }

        let output = cmd.output().expect("Failed to execute read command");
        assert!(
            !output.status.success(),
            "Command {:?} should fail when no workspace exists",
            cmd_args
        );

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("ods pull"),
            "Command {:?} stderr must name 'ods pull', got:\n{}",
            cmd_args,
            stderr
        );
        assert!(
            stderr.contains("ods trud pull"),
            "Command {:?} stderr must name 'ods trud pull', got:\n{}",
            cmd_args,
            stderr
        );

        assert_eq!(
            fs::read_dir(&empty_dir).unwrap().count(),
            0,
            "Command {:?} must not write anything on refusal path",
            cmd_args
        );
    }
}

#[test]
fn test_trud_pull_furniture_and_ods_make_succeeds() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join(".git")).unwrap();

    let project_dir = tmp.path().join("project");
    fs::create_dir_all(&project_dir).unwrap();

    let mock_zip = create_mock_trud_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_20260731000001.zip");

    // 1. Run `ods trud pull --local-archive <zip>`
    let trud_pull_output = ods_binary()
        .current_dir(&project_dir)
        .arg("trud")
        .arg("pull")
        .arg("--local-archive")
        .arg(&mock_zip)
        .output()
        .expect("execute ods trud pull");

    assert!(
        trud_pull_output.status.success(),
        "ods trud pull failed: stderr was:\n{}",
        String::from_utf8_lossy(&trud_pull_output.stderr)
    );

    let ws_dir = project_dir.join("ods_data");
    assert!(ws_dir.join(".gitignore").is_file(), ".gitignore must exist in workspace root after trud pull");
    assert!(ws_dir.join("README.md").is_file(), "README.md must exist in workspace root after trud pull");
    assert!(ws_dir.join("_releases.json").is_file(), "_releases.json must exist in workspace root after trud pull");
    assert!(ws_dir.join("current").exists(), "current symlink must exist after trud pull");

    // 2. Run bare `ods make` in the project directory
    let make_output = ods_binary()
        .current_dir(&project_dir)
        .arg("make")
        .output()
        .expect("execute bare ods make");

    assert!(
        make_output.status.success(),
        "bare ods make failed: stderr was:\n{}",
        String::from_utf8_lossy(&make_output.stderr)
    );

    assert!(
        ws_dir.join("current").join("orgs.parquet").is_file(),
        "orgs.parquet must exist in current release after bare ods make"
    );
}

#[test]
fn test_use_cmd_repairs_missing_furniture() {
    let tmp = TempDir::new().unwrap();
    let ws_dir = tmp.path().join("unfurnished_ws");
    let rel_dir = ws_dir.join("releases").join("2026-07-31");
    fs::create_dir_all(&rel_dir).unwrap();

    // Export fixture XML directly into release dir so verify_release_dir passes
    let mock_zip = create_mock_trud_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_20260731000001.zip");
    let make_output = ods_binary()
        .arg("make")
        .arg("-i")
        .arg(&mock_zip)
        .arg("-o")
        .arg(&rel_dir)
        .output()
        .expect("execute ods make into release dir");
    assert!(make_output.status.success());

    // Ensure workspace furniture is missing before `ods use`
    assert!(!ws_dir.join(".gitignore").exists());
    assert!(!ws_dir.join("README.md").exists());

    // Run `ods use 2026-07-31 --workspace <dir>`
    let use_output = ods_binary()
        .arg("use")
        .arg("2026-07-31")
        .arg("--workspace")
        .arg(&ws_dir)
        .output()
        .expect("execute ods use");

    assert!(
        use_output.status.success(),
        "ods use failed: stderr was:\n{}",
        String::from_utf8_lossy(&use_output.stderr)
    );

    assert!(ws_dir.join(".gitignore").is_file(), "ods use must repair .gitignore");
    assert!(ws_dir.join("README.md").is_file(), "ods use must repair README.md");
    assert!(ws_dir.join("current").exists(), "ods use must pin current");
}

#[test]
fn test_bare_ods_make_in_empty_dir_fails_and_leaves_dir_empty() {
    let tmp = TempDir::new().unwrap();
    // Boundary to stop discovery from ascending into repository root
    fs::create_dir_all(tmp.path().join(".git")).unwrap();

    let empty_dir = tmp.path().join("empty_project");
    fs::create_dir_all(&empty_dir).unwrap();

    let output = ods_binary()
        .current_dir(&empty_dir)
        .arg("make")
        .output()
        .expect("execute bare ods make in empty dir");

    assert!(!output.status.success(), "bare ods make must fail in empty dir");
    assert_eq!(
        fs::read_dir(&empty_dir).unwrap().count(),
        0,
        "bare ods make in empty dir must leave directory completely empty (no ods_data litter)"
    );
}

#[test]
fn test_find_with_loose_parquet_dir() {
    let tmp = TempDir::new().unwrap();
    let mock_zip = create_mock_trud_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_20260731000001.zip");

    // Make into a staging dir to get orgs.parquet
    let staging = tmp.path().join("staging");
    fs::create_dir_all(&staging).unwrap();
    let make_output = ods_binary()
        .arg("make")
        .arg("-i")
        .arg(&mock_zip)
        .arg("-o")
        .arg(&staging)
        .output()
        .expect("execute ods make");
    assert!(make_output.status.success());

    // Copy ONLY orgs.parquet to a loose directory with NO provenance, releases/, current, etc.
    let loose_dir = tmp.path().join("loose_parquet");
    fs::create_dir_all(&loose_dir).unwrap();
    fs::copy(staging.join("orgs.parquet"), loose_dir.join("orgs.parquet")).unwrap();

    // Verify it is not a workspace
    assert!(!loose_dir.join("_provenance.json").exists());
    assert!(!loose_dir.join("releases").exists());
    assert!(!loose_dir.join("current").exists());

    // Run `ods find Mock -i <loose_dir>`
    let find_output = ods_binary()
        .arg("find")
        .arg("Mock")
        .arg("-i")
        .arg(&loose_dir)
        .output()
        .expect("execute ods find");

    assert!(
        find_output.status.success(),
        "ods find -i <loose_dir> must succeed, stderr: {}",
        String::from_utf8_lossy(&find_output.stderr)
    );
    let stdout = String::from_utf8_lossy(&find_output.stdout);
    assert!(stdout.contains("Mock"), "stdout must contain found record");
}

#[test]
fn test_find_with_non_active_release_honours_explicit_release_dir() {
    let tmp = TempDir::new().unwrap();
    let ws_dir = tmp.path().join("ods_data");
    let rel1_dir = ws_dir.join("releases").join("2026-05-01");
    let rel2_dir = ws_dir.join("releases").join("2026-07-31");
    fs::create_dir_all(&rel1_dir).unwrap();
    fs::create_dir_all(&rel2_dir).unwrap();

    let mock_zip = create_mock_trud_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_20260731000001.zip");

    // Build release 1
    let make1 = ods_binary()
        .arg("make")
        .arg("-i")
        .arg(&mock_zip)
        .arg("-o")
        .arg(&rel1_dir)
        .output()
        .expect("make rel1");
    assert!(make1.status.success());

    // Build release 2
    let make2 = ods_binary()
        .arg("make")
        .arg("-i")
        .arg(&mock_zip)
        .arg("-o")
        .arg(&rel2_dir)
        .output()
        .expect("make rel2");
    assert!(make2.status.success());

    // Pin active release to release 2 (2026-07-31)
    let use_output = ods_binary()
        .arg("use")
        .arg("2026-07-31")
        .arg("--workspace")
        .arg(&ws_dir)
        .output()
        .expect("ods use");
    assert!(use_output.status.success());

    // 1. Explicitly querying release 1 should succeed
    let find_output = ods_binary()
        .arg("find")
        .arg("Mock")
        .arg("-i")
        .arg(&rel1_dir)
        .output()
        .expect("ods find -i rel1");
    assert!(
        find_output.status.success(),
        "ods find -i <non-active-release> must succeed, stderr: {}",
        String::from_utf8_lossy(&find_output.stderr)
    );

    // 2. Delete orgs.parquet from release 1: querying it must fail rather than silently falling back to current
    fs::remove_file(rel1_dir.join("orgs.parquet")).unwrap();
    let find_deleted = ods_binary()
        .arg("find")
        .arg("Mock")
        .arg("-i")
        .arg(&rel1_dir)
        .output()
        .expect("ods find -i rel1 with deleted orgs.parquet");
    assert!(
        !find_deleted.status.success(),
        "ods find -i <non-active-release> with deleted parquet must fail, NOT fall back to active release"
    );
    let stderr = String::from_utf8_lossy(&find_deleted.stderr);
    assert!(
        stderr.contains("not found in") || stderr.contains("not found"),
        "stderr should state parquet file not found, got: {}",
        stderr
    );
}

#[test]
fn test_cite_with_non_active_release_honours_explicit_release_dir() {
    let tmp = TempDir::new().unwrap();
    let ws_dir = tmp.path().join("ods_data");
    let rel1_dir = ws_dir.join("releases").join("2026-05-01");
    let rel2_dir = ws_dir.join("releases").join("2026-07-31");
    fs::create_dir_all(&rel1_dir).unwrap();
    fs::create_dir_all(&rel2_dir).unwrap();

    let mock_zip = create_mock_trud_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_20260731000001.zip");

    let make1 = ods_binary()
        .arg("make")
        .arg("-i")
        .arg(&mock_zip)
        .arg("-o")
        .arg(&rel1_dir)
        .output()
        .expect("make rel1");
    assert!(make1.status.success());

    let make2 = ods_binary()
        .arg("make")
        .arg("-i")
        .arg(&mock_zip)
        .arg("-o")
        .arg(&rel2_dir)
        .output()
        .expect("make rel2");
    assert!(make2.status.success());

    // Pin active release to release 2 (2026-07-31)
    let use_output = ods_binary()
        .arg("use")
        .arg("2026-07-31")
        .arg("--workspace")
        .arg(&ws_dir)
        .output()
        .expect("ods use");
    assert!(use_output.status.success());

    // Mark rel1 provenance as verified so ods cite allows it
    let prov_path = rel1_dir.join(ods::provenance::PROVENANCE_FILENAME);
    let mut prov = ods::provenance::OdsProvenance::load_from_dir(&rel1_dir).unwrap();
    prov.trud_release_date = Some("2026-05-01".to_string());
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);
    fs::write(&prov_path, serde_json::to_string_pretty(&prov).unwrap()).unwrap();

    // Query cite on release 1 explicitly: output should cite 2026-05-01, NOT 2026-07-31
    let cite_output = ods_binary()
        .arg("cite")
        .arg("-i")
        .arg(&rel1_dir)
        .output()
        .expect("ods cite -i rel1");
    assert!(
        cite_output.status.success(),
        "ods cite -i <archived-release> failed, stderr: {}",
        String::from_utf8_lossy(&cite_output.stderr)
    );
    let stdout = String::from_utf8_lossy(&cite_output.stdout);
    assert!(
        stdout.contains("2026-05-01"),
        "ods cite -i <archived-release> must cite the requested release date 2026-05-01, got:\n{}",
        stdout
    );
}

#[test]
fn test_ods_make_explicit_output_seeds_releases_json() {
    let tmp = TempDir::new().unwrap();
    let mock_zip = create_mock_trud_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_20260731000001.zip");
    let custom_ws = tmp.path().join("custom_root");
    let rel_dir = custom_ws.join("releases").join("2026-07-31");

    let make_output = ods_binary()
        .arg("make")
        .arg("-i")
        .arg(&mock_zip)
        .arg("-o")
        .arg(&rel_dir)
        .output()
        .expect("ods make -o custom_ws/releases/2026-07-31");

    assert!(make_output.status.success());
    let seeded_marker = custom_ws.join("_releases.json");
    assert!(seeded_marker.is_file(), "_releases.json must be seeded in custom root");
    let bytes = fs::read(&seeded_marker).unwrap();
    assert_eq!(bytes, ods::index::BAKED_RELEASES_JSON_BYTES, "_releases.json must be byte-identical to baked index");
}

#[test]
fn test_ods_use_seeds_missing_marker() {
    let tmp = TempDir::new().unwrap();
    let ws_dir = tmp.path().join("ods_data");
    let rel_dir = ws_dir.join("releases").join("2026-07-31");
    fs::create_dir_all(&rel_dir).unwrap();

    let mock_zip = create_mock_trud_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_20260731000001.zip");
    let make_output = ods_binary()
        .arg("make")
        .arg("-i")
        .arg(&mock_zip)
        .arg("-o")
        .arg(&rel_dir)
        .output()
        .expect("ods make");
    assert!(make_output.status.success());

    // Explicitly remove the marker if created, to test that `ods use` seeds it if missing
    let marker = ws_dir.join("_releases.json");
    if marker.exists() {
        fs::remove_file(&marker).unwrap();
    }
    assert!(!marker.exists());

    let use_output = ods_binary()
        .arg("use")
        .arg("2026-07-31")
        .arg("--workspace")
        .arg(&ws_dir)
        .output()
        .expect("ods use");

    assert!(use_output.status.success());
    assert!(marker.is_file(), "ods use must seed missing _releases.json");
    assert_eq!(fs::read(&marker).unwrap(), ods::index::BAKED_RELEASES_JSON_BYTES);
}

#[test]
fn test_staleness_nudge_emitted_on_table_and_suppressed_on_json() {
    let tmp = TempDir::new().unwrap();
    let ws_dir = tmp.path().join("ods_data");
    let rel_dir = ws_dir.join("releases").join("2026-07-31");
    fs::create_dir_all(&rel_dir).unwrap();

    let mock_zip = create_mock_trud_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_20260731000001.zip");
    let make_output = ods_binary()
        .arg("make")
        .arg("-i")
        .arg(&mock_zip)
        .arg("-o")
        .arg(&rel_dir)
        .output()
        .expect("ods make");
    assert!(make_output.status.success());

    let use_output = ods_binary()
        .arg("use")
        .arg("2026-07-31")
        .arg("--workspace")
        .arg(&ws_dir)
        .output()
        .expect("ods use");
    assert!(use_output.status.success());

    // Create an old release index (e.g. from 2020-01-01) so it is definitely stale (>45 days)
    let stale_index = ods::index::OdsReleaseIndex {
        type_tag: "ods_release_index".to_string(),
        index_version: 2,
        concept_doi: None,
        mirrors: vec![],
        releases: vec![
            ods::index::ReleaseIndexEntry {
                trud_release_date: "2020-01-01".to_string(),
                dataset_version: "0.1.0".to_string(),
                tag: "2020-01-01_0.1.0".to_string(),
                manifest_digest: "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_string(),
                trud_release_sha256: "0000".to_string(),
                tool_version: "0.1.0".to_string(),
                dataset_doi: None,
                withdrawn: None,
            },
        ],
    };
    let stale_bytes = serde_json::to_vec_pretty(&stale_index).unwrap();
    fs::write(ws_dir.join("_releases.json"), &stale_bytes).unwrap();

    // 1. Table format (human): nudge MUST be emitted to stderr
    let find_table = ods_binary()
        .current_dir(tmp.path())
        .arg("find")
        .arg("Sedbergh")
        .output()
        .expect("ods find");
    assert!(find_table.status.success());
    let stderr_table = String::from_utf8_lossy(&find_table.stderr);
    assert!(
        stderr_table.contains("2020-01-01 is") && stderr_table.contains("days old. TRUD ships roughly every 4 weeks"),
        "stderr must contain staleness nudge, got:\n{}",
        stderr_table
    );
    assert!(
        stderr_table.contains("Check with: ods pull"),
        "stderr must contain 'Check with: ods pull', got:\n{}",
        stderr_table
    );

    // 2. JSON format (machine-readable): nudge MUST be suppressed
    let find_json = ods_binary()
        .current_dir(tmp.path())
        .arg("find")
        .arg("Sedbergh")
        .arg("--format")
        .arg("json")
        .output()
        .expect("ods find --format json");
    assert!(find_json.status.success());
    let stderr_json = String::from_utf8_lossy(&find_json.stderr);
    assert!(
        !stderr_json.contains("days old. TRUD ships roughly every 4 weeks"),
        "machine-readable JSON format must NOT output staleness nudge, got:\n{}",
        stderr_json
    );
}







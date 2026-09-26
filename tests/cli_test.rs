//! End-to-end CLI integration tests verifying binary execution and stdout/stderr output formatting.

use std::fs;
use tempfile::TempDir;

mod common;

use common::{create_mock_trud_zip, make_v1_index, ods_binary};

#[test]
fn test_cli_version_output_formatting() {
    let output = ods_binary()
        .arg("--version")
        .output()
        .expect("Failed to execute ods --version");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();

    let output_short = ods_binary()
        .arg("-V")
        .output()
        .expect("Failed to execute ods -V");
    assert!(output_short.status.success());
    let stdout_short = String::from_utf8_lossy(&output_short.stdout).trim().to_string();
    assert_eq!(stdout, stdout_short, "-V and --version must match identically");

    let pkg_ver = env!("CARGO_PKG_VERSION");
    let git_sha = option_env!("ODS_GIT_SHA");
    let git_dirty = option_env!("ODS_GIT_DIRTY");

    match (git_sha, git_dirty) {
        (Some(sha), Some("true")) => {
            let short = if sha.len() >= 7 { &sha[..7] } else { sha };
            let expected = format!("ods {} ({}-dirty)", pkg_ver, short);
            assert_eq!(stdout, expected);
        }
        (Some(sha), _) => {
            let short = if sha.len() >= 7 { &sha[..7] } else { sha };
            let expected = format!("ods {} ({})", pkg_ver, short);
            assert_eq!(stdout, expected);
        }
        (None, _) => {
            let expected = format!("ods {}", pkg_ver);
            assert_eq!(stdout, expected);
        }
    }
}

#[test]
fn test_cli_pull_list_output_formatting() {
    let tmp = TempDir::new().unwrap();
    let idx_file = tmp.path().join("releases.json");
    fs::write(&idx_file, ods::index::BAKED_RELEASES_JSON_BYTES).unwrap();

    let output = ods_binary()
        .current_dir(tmp.path())
        .arg("pull")
        .arg("--list")
        .arg("--index")
        .arg(&idx_file)
        .output()
        .expect("Failed to execute pull --list");

    // With a fixed `--index`, this doesn't reach the network, so its result is
    // deterministic. It checks output formatting either way, so both branches stay:
    // an unreachable index would otherwise be a deliberate non-zero exit (see
    // `AlreadyReported`), so accept 0 or 1 and reject anything else, which would
    // mean a crash or signal.
    let code = output.status.code();
    assert!(
        matches!(code, Some(0) | Some(1)),
        "expected a clean exit or a reported failure, got {code:?}"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if output.status.success() {
        assert!(!stdout.is_empty(), "pull --list must output rows to stdout when successful");
        assert!(
            stdout.contains("Available ODS dataset releases:\n"),
            "stdout should contain heading, got:\n{}",
            stdout
        );
        assert!(
            !stderr.contains("Available ODS dataset releases:"),
            "stderr should not contain heading, got:\n{}",
            stderr
        );
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
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());
    fs::write(rel_dir.join("orgs.parquet"), b"dummy").unwrap();
    fs::write(rel_dir.join("datapackage.json"), b"{\"name\": \"ods\", \"version\": \"1.0.1\"}").unwrap();
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
    let rel1 = ws.join("releases").join("2026-05-29");
    fs::create_dir_all(&rel1).unwrap();
    fs::write(rel1.join(ods::provenance::PROVENANCE_FILENAME), serde_json::to_string_pretty(&prov1).unwrap()).unwrap();
    fs::write(rel1.join("orgs.parquet"), b"dummy parquet 1").unwrap();
    fs::write(rel1.join("datapackage.json"), b"{\"name\": \"ods\", \"version\": \"1.0.1\"}").unwrap();

    let (m1, _) = ods::commands::make_oci::build_manifest_from_dir(&rel1, &prov1, "1.0.1").unwrap();
    let d1 = m1.digest().unwrap();

    let mut prov2 = ods::provenance::OdsProvenance::default();
    prov2.trud_release_date = Some("2026-06-26".to_string());
    let rel2 = ws.join("releases").join("2026-06-26");
    fs::create_dir_all(&rel2).unwrap();
    fs::write(rel2.join(ods::provenance::PROVENANCE_FILENAME), serde_json::to_string_pretty(&prov2).unwrap()).unwrap();
    fs::write(rel2.join("orgs.parquet"), b"dummy parquet 2").unwrap();
    fs::write(rel2.join("datapackage.json"), b"{\"name\": \"ods\", \"version\": \"1.0.1\"}").unwrap();

    let (m2, _) = ods::commands::make_oci::build_manifest_from_dir(&rel2, &prov2, "1.0.1").unwrap();
    let d2 = m2.digest().unwrap();

    let index = make_v1_index(&[
        (
            "2026-06-26",
            "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
            37983173,
            &[("1.0.1", &d2)],
        ),
        (
            "2026-05-29",
            "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
            37983173,
            &[("1.0.1", &d1)],
        ),
    ]);
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
        stderr.contains("2026-05-29") && stderr.contains("cached"),
        "stderr must report the cache hit as a cached block, got:\n{}",
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
        &["trud", "audit"],
        &["trud", "diff"],
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

    let mock_zip =
        create_mock_trud_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_20260731000001.zip");
    let index_file = common::write_index_for_zip(tmp.path(), "2026-07-31", &mock_zip);

    // 1. Run `ods trud pull --local-archive <zip>`
    let trud_pull_output = ods_binary()
        .current_dir(&project_dir)
        .arg("trud")
        .arg("pull")
        .arg("--local-archive")
        .arg(&mock_zip)
        .arg("--index")
        .arg(&index_file)
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
    let mock_zip =
        create_mock_trud_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_20260731000001.zip");
    let index_file = common::write_index_for_zip(tmp.path(), "2026-07-31", &mock_zip);
    let make_output = ods_binary()
        .arg("make")
        .arg("-i")
        .arg(&mock_zip)
        .arg("--index")
        .arg(&index_file)
        .arg("-o")
        .arg(&rel_dir)
        .output()
        .expect("execute ods make into release dir");
    assert!(make_output.status.success());

    // Ensure workspace furniture is missing before `ods use`
    assert!(!ws_dir.join(".gitignore").exists());
    assert!(!ws_dir.join("README.md").exists());

    // Run `ods use 2026-07-31` from within the workspace directory
    let use_output = ods_binary()
        .current_dir(&ws_dir)
        .arg("use")
        .arg("2026-07-31")
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
        .arg("--force")
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

    let mock_zip =
        create_mock_trud_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_20260731000001.zip");
    let index_file = common::write_index_for_zip(tmp.path(), "2026-07-31", &mock_zip);

    // Build release 1
    let make1 = ods_binary()
        .arg("make")
        .arg("-i")
        .arg(&mock_zip)
        .arg("--index")
        .arg(&index_file)
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
        .arg("--index")
        .arg(&index_file)
        .arg("-o")
        .arg(&rel2_dir)
        .output()
        .expect("make rel2");
    assert!(make2.status.success());

    // Pin active release to release 2 (2026-07-31)
    let use_output = ods_binary()
        .current_dir(tmp.path())
        .arg("use")
        .arg("2026-07-31")
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

    let mock_zip =
        create_mock_trud_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_20260731000001.zip");
    let index_file = common::write_index_for_zip(tmp.path(), "2026-07-31", &mock_zip);

    let make1 = ods_binary()
        .arg("make")
        .arg("-i")
        .arg(&mock_zip)
        .arg("--index")
        .arg(&index_file)
        .arg("-o")
        .arg(&rel1_dir)
        .output()
        .expect("make rel1");
    assert!(make1.status.success());

    let make2 = ods_binary()
        .arg("make")
        .arg("-i")
        .arg(&mock_zip)
        .arg("--index")
        .arg(&index_file)
        .arg("-o")
        .arg(&rel2_dir)
        .output()
        .expect("make rel2");
    assert!(make2.status.success());

    // Pin active release to release 2 (2026-07-31)
    let use_output = ods_binary()
        .current_dir(tmp.path())
        .arg("use")
        .arg("2026-07-31")
        .output()
        .expect("ods use");
    assert!(use_output.status.success());

    // Mark rel1 provenance as verified so ods cite allows it
    let prov_path = rel1_dir.join(ods::provenance::PROVENANCE_FILENAME);
    let mut prov = ods::provenance::OdsProvenance::load_from_dir(&rel1_dir).unwrap();
    prov.trud_release_date = Some("2026-05-01".to_string());
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
fn test_ods_make_explicit_output_writes_no_releases_json() {
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
        .arg("--force")
        .output()
        .expect("ods make -o custom_ws/releases/2026-07-31");

    assert!(make_output.status.success());
    let seeded_marker = custom_ws.join("_releases.json");
    assert!(!seeded_marker.exists(), "_releases.json must NOT be seeded by ods make");
}

#[test]
fn test_ods_use_in_workspace_writes_no_releases_json() {
    let tmp = TempDir::new().unwrap();
    let ws_dir = tmp.path().join("ods_data");
    let rel_dir = ws_dir.join("releases").join("2026-07-31");
    fs::create_dir_all(&rel_dir).unwrap();

    let mock_zip =
        create_mock_trud_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_20260731000001.zip");
    let index_file = common::write_index_for_zip(tmp.path(), "2026-07-31", &mock_zip);
    let make_output = ods_binary()
        .arg("make")
        .arg("-i")
        .arg(&mock_zip)
        .arg("--index")
        .arg(&index_file)
        .arg("-o")
        .arg(&rel_dir)
        .output()
        .expect("ods make");
    assert!(make_output.status.success());

    // Explicitly remove the marker if created, to test that `ods use` does not write it
    let marker = ws_dir.join("_releases.json");
    if marker.exists() {
        fs::remove_file(&marker).unwrap();
    }
    assert!(!marker.exists());

    let use_output = ods_binary()
        .current_dir(tmp.path())
        .arg("use")
        .arg("2026-07-31")
        .output()
        .expect("ods use");

    assert!(use_output.status.success());
    assert!(!marker.exists(), "ods use must NOT write _releases.json");
}

#[test]
fn test_staleness_nudge_emitted_on_table_and_suppressed_on_json() {
    let tmp = TempDir::new().unwrap();
    let ws_dir = tmp.path().join("ods_data");
    let rel_dir = ws_dir.join("releases").join("2020-01-01");
    fs::create_dir_all(&rel_dir).unwrap();

    let mock_zip =
        create_mock_trud_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_20200101000001.zip");
    let index_file = common::write_index_for_zip(tmp.path(), "2020-01-01", &mock_zip);
    let make_output = ods_binary()
        .arg("make")
        .arg("-i")
        .arg(&mock_zip)
        .arg("--index")
        .arg(&index_file)
        .arg("-o")
        .arg(&rel_dir)
        .output()
        .expect("ods make");
    assert!(make_output.status.success());

    let use_output = ods_binary()
        .current_dir(tmp.path())
        .arg("use")
        .arg("2020-01-01")
        .output()
        .expect("ods use");
    assert!(use_output.status.success());

    // 1. Table format (human): notice MUST be emitted to stderr as the last line
    let find_table = ods_binary()
        .current_dir(tmp.path())
        .arg("find")
        .arg("Sedbergh")
        .output()
        .expect("ods find");
    assert!(find_table.status.success());
    let stderr_table = String::from_utf8_lossy(&find_table.stderr);
    let expected_start = "* 2020-01-01 release is ";
    let expected_end = " days old. Run `ods pull` to check for a newer one.";
    assert!(
        stderr_table.contains(expected_start) && stderr_table.contains(expected_end),
        "stderr must contain staleness notice, got:\n{}",
        stderr_table
    );
    let last_line = stderr_table.trim_end().lines().last().unwrap_or("");
    assert!(
        last_line.starts_with(expected_start) && last_line.ends_with(expected_end),
        "staleness notice must be the last line on stderr, got:\n{}",
        last_line
    );

    // 2. JSON format (machine-readable): notice MUST be suppressed
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
        !stderr_json.contains("release is") && !stderr_json.contains("ods pull"),
        "machine-readable JSON format must NOT output staleness notice, got:\n{}",
        stderr_json
    );
}

#[test]
fn test_error_without_cross_sigil_is_prefixed_with_cross_and_has_no_error_prefix() {
    let tmp = tempfile::tempdir().unwrap();
    let idx_file = tmp.path().join("releases.json");
    std::fs::write(&idx_file, ods::index::BAKED_RELEASES_JSON_BYTES).unwrap();
    let output = ods_binary()
        .current_dir(tmp.path())
        .args(["pull", "--index", idx_file.to_str().unwrap(), "1999-01-01"])
        .output()
        .expect("run ods pull 1999-01-01");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("✖ "),
        "stderr must start with '✖ ', got:\n{}",
        stderr
    );
    assert!(
        !stderr.contains("Error:"),
        "stderr must contain no 'Error:', got:\n{}",
        stderr
    );
}

#[test]
fn test_unreadable_provenance_reading_commands_warn_and_continue() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    let rel_dir = ws.join("releases").join("2026-08-28");
    fs::create_dir_all(&rel_dir).unwrap();
    fs::write(ws.join("_releases.json"), ods::index::BAKED_RELEASES_JSON_BYTES).unwrap();

    let (_find_tmp, find_dir) = common::setup_find_test_workspace();
    for entry in fs::read_dir(&find_dir).unwrap() {
        let entry = entry.unwrap();
        if entry.path().extension().is_some_and(|ext| ext == "parquet") {
            fs::copy(entry.path(), rel_dir.join(entry.file_name())).unwrap();
        }
    }
    let dp = ods::datapackage::generate_release_datapackage(&rel_dir, None, None);
    fs::write(rel_dir.join("datapackage.json"), serde_json::to_string_pretty(&dp).unwrap()).unwrap();

    // 1. Unreadable provenance with no $schema
    let bad_prov = r#"{
  "trud_release_date": "2026-08-28",
  "unrecognised_field": "some_value"
}"#;
    fs::write(rel_dir.join(ods::provenance::PROVENANCE_FILENAME), bad_prov).unwrap();

    // ods find should return rows and print the '!' warning once on stderr
    let find_out = ods_binary()
        .current_dir(&ws)
        .args(["find", "sedbergh", "-i", "releases/2026-08-28", "--plain"])
        .output()
        .expect("ods find");
    assert!(find_out.status.success(), "find must succeed even with unreadable provenance");
    let stdout = String::from_utf8_lossy(&find_out.stdout);
    let stderr = String::from_utf8_lossy(&find_out.stderr);
    assert!(stdout.contains("SEDBERGH"), "find output must contain rows");
    assert!(
        stderr.contains("! releases/2026-08-28/_provenance.json isn't provenance this ods can read. Rebuild the release with `ods make`, or pull it again."),
        "stderr must contain '!' warning, got:\n{}",
        stderr
    );
    // Ensure the warning is printed exactly once
    assert_eq!(
        stderr.matches("isn't provenance this ods can read").count(),
        1,
        "warning should be printed exactly once, got:\n{}",
        stderr
    );

    // ods make oci should fail with exit 1, print the '✖' block, and leave oci/ absent
    let make_oci_out = ods_binary()
        .current_dir(&ws)
        .args(["make", "oci", "-i", "releases/2026-08-28"])
        .output()
        .expect("ods make oci");
    assert!(!make_oci_out.status.success(), "make oci must fail on unreadable provenance");
    assert_eq!(make_oci_out.status.code(), Some(1));
    let oci_stderr = String::from_utf8_lossy(&make_oci_out.stderr);
    assert!(
        oci_stderr.contains("✖ releases/2026-08-28/_provenance.json isn't provenance this ods can read"),
        "stderr must contain '✖' error block, got:\n{}",
        oci_stderr
    );
    assert!(
        oci_stderr.contains("Expected $schema https://ods.fyi/schema/provenance.v1.json"),
        "stderr must explain expected $schema, got:\n{}",
        oci_stderr
    );
    assert!(
        oci_stderr.contains("Pull the archive again with `ods trud pull 2026-08-28 --force`, then run `ods make`."),
        "stderr must provide remediation hint, got:\n{}",
        oci_stderr
    );
    assert!(
        !rel_dir.join("oci").exists(),
        "oci/ must remain absent on unreadable provenance failure"
    );

    // 2. Absent provenance: both commands behave as expected
    fs::remove_file(rel_dir.join(ods::provenance::PROVENANCE_FILENAME)).unwrap();

    let find_absent = ods_binary()
        .current_dir(&ws)
        .args(["find", "sedbergh", "-i", "releases/2026-08-28", "--plain"])
        .output()
        .expect("ods find absent");
    assert!(find_absent.status.success());
    let absent_stdout = String::from_utf8_lossy(&find_absent.stdout);
    let absent_stderr = String::from_utf8_lossy(&find_absent.stderr);
    assert!(absent_stdout.contains("SEDBERGH"));
    assert!(
        !absent_stderr.contains("isn't provenance this ods can read"),
        "absent provenance must NOT emit unreadable warning"
    );

    let make_oci_absent = ods_binary()
        .current_dir(&ws)
        .args(["make", "oci", "-i", "releases/2026-08-28"])
        .output()
        .expect("ods make oci absent");
    let make_oci_stderr = String::from_utf8_lossy(&make_oci_absent.stderr);
    assert!(!make_oci_absent.status.success(), "make oci must fail when provenance is absent");
    assert_eq!(make_oci_absent.status.code(), Some(1));
    assert!(
        make_oci_stderr.contains(
            "has no provenance: it was built from an archive ods couldn't match to a TRUD release"
        ),
        "stderr should report no provenance refusal, got:\n{}",
        make_oci_stderr
    );
    assert!(
        make_oci_stderr.contains("To cite or publish it, get the archive through ods trud pull."),
        "stderr should guide user to pull, got:\n{}",
        make_oci_stderr
    );
    assert!(
        absent_stderr.contains(
            "has no provenance: it was built from an archive ods couldn't match to a TRUD release"
        ),
        "stderr should report no provenance warning for find, got:\n{}",
        absent_stderr
    );
}







use ods::commands::{parquet, find, diff};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const FIXTURE_XML: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/mock_hscorgrefdata.xml"
);

fn create_mock_trud_zip(dir: &Path, filename: &str) -> PathBuf {
    let zip_path = dir.join(filename);
    let zip_file = fs::File::create(&zip_path).unwrap();
    let mut zip_writer = zip::ZipWriter::new(zip_file);
    let options = zip::write::SimpleFileOptions::default();
    zip_writer.start_file("HSCOrgRefData_Full_mock.xml", options).unwrap();
    let xml_content = fs::read_to_string(FIXTURE_XML).unwrap();
    zip_writer.write_all(xml_content.as_bytes()).unwrap();
    zip_writer.finish().unwrap();
    zip_path
}

#[test]
fn test_workspace_full_lifecycle() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // 1. Initialise workspace with parquet export
    let workspace_dir = root.join("ods_data");
    fs::create_dir_all(&workspace_dir).unwrap();

    let release_1_dir = workspace_dir.join("releases").join("2026-05-29");
    fs::create_dir_all(&release_1_dir).unwrap();

    let zip1 = create_mock_trud_zip(root, "hscorgrefdataxml_data_7.0.0_20260529000001.zip");

    // Export fixture XML directly into workspace release 1
    parquet::run(parquet::Args {
        input: Some(zip1),
        output: Some(release_1_dir.clone()),
    })
    .expect("parquet run into release 1 should succeed");

    ods::workspace::set_active_release(&workspace_dir, "2026-05-29").unwrap();

    // Verify release 1 active status
    let (active_date, active_path) = ods::workspace::get_active_release(&workspace_dir).unwrap();
    assert_eq!(active_date, "2026-05-29");
    assert!(active_path.join("orgs.parquet").exists());

    // 2. Export release 2 (simulating a new monthly TRUD release)
    let release_2_dir = workspace_dir.join("releases").join("2026-06-26");
    fs::create_dir_all(&release_2_dir).unwrap();

    let zip2 = create_mock_trud_zip(root, "hscorgrefdataxml_data_7.0.0_20260626000001.zip");

    parquet::run(parquet::Args {
        input: Some(zip2),
        output: Some(release_2_dir.clone()),
    })
    .expect("parquet run into release 2 should succeed");

    ods::workspace::set_active_release(&workspace_dir, "2026-06-26").unwrap();

    // 3. Test release switching
    ods::workspace::set_active_release(&workspace_dir, "2026-05-29").unwrap();

    let (switched_date, _) = ods::workspace::get_active_release(&workspace_dir).unwrap();
    assert_eq!(switched_date, "2026-05-29");

    // 4. Test auto-discovery by `ods find`
    let discovered_parquet = ods::workspace::discover_parquet_dir(Some(&workspace_dir)).unwrap();
    assert!(discovered_parquet.join("orgs.parquet").exists());

    let mut find_out = Vec::new();
    find::run_with_writer(
        find::Args {
            query: Some("Mock".to_string()),
            code: Vec::new(),
            location: None,
            role: Vec::new(),
            all: false,
            verbose: false,
            sort: Some(find::SortBy::Code),
            format: find::OutputFormat::Json,
            input: discovered_parquet.clone(),
            ..Default::default()
        },
        &mut find_out,
        &discovered_parquet,
    )
    .expect("find run should succeed");
    assert!(String::from_utf8(find_out).unwrap().contains("Mock GP Practice"));

    // 5. Test rolling release diff
    diff::run(diff::Args {
        old: Some(release_1_dir),
        new: Some(release_2_dir),
        format: "summary".to_string(),
        role: None,
        verbose: false,
        only_active: false,
        output: None,
    })
    .expect("diff should succeed across rolling releases");
}

#[test]
fn test_parquet_handles_trud_zip_input() {
    let tmp = TempDir::new().unwrap();
    let zip_path = create_mock_trud_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_20260731000001.zip");
    let out_dir = tmp.path().join("output_parquet");

    // Run `ods parquet` directly with ZIP input
    parquet::run(parquet::Args {
        input: Some(zip_path),
        output: Some(out_dir.clone()),
    })
    .expect("ods parquet should successfully handle TRUD zip archive input");

    assert!(out_dir.join("orgs.parquet").exists());
    assert!(out_dir.join("roles.parquet").exists());
    assert!(out_dir.join("relationships.parquet").exists());
}

#[test]
fn test_prepare_release_dir_does_not_create_markdown_dir() {
    let tmp = TempDir::new().unwrap();
    let workspace_dir = tmp.path().join("ods_data");
    fs::create_dir_all(&workspace_dir).unwrap();

    let release_dir = ods::workspace::prepare_release_dir(&workspace_dir, "2026-07-31").unwrap();
    assert!(release_dir.join("trud").exists(), "trud directory must be created");
    assert!(!release_dir.join("markdown").exists(), "markdown directory must NOT be created unconditionally");
}

#[test]
fn test_ensure_workspace_gitignore_does_not_unignore_wiki_zip() {
    let tmp = TempDir::new().unwrap();
    let workspace_dir = tmp.path().join("ods_data");
    fs::create_dir_all(&workspace_dir).unwrap();

    ods::workspace::ensure_workspace_gitignore(&workspace_dir).unwrap();
    let gitignore_content = fs::read_to_string(workspace_dir.join(".gitignore")).unwrap();
    assert!(
        !gitignore_content.contains("!releases/*/markdown/wiki.zip"),
        ".gitignore must not un-ignore wiki.zip"
    );
}

#[test]
fn test_tightened_workspace_discovery_rules() {
    let tmp = TempDir::new().unwrap();

    // 1. Explicit wins. No further checks (even if path looks like or sits under a releases folder).
    let explicit_root = PathBuf::from("/custom/path/ws");
    assert_eq!(
        ods::workspace::find_workspace_root_from(tmp.path(), Some(&explicit_root)),
        Some(explicit_root)
    );

    let explicit_with_releases_parent = PathBuf::from("/foo/releases/ws");
    assert_eq!(
        ods::workspace::find_workspace_root_from(tmp.path(), Some(&explicit_with_releases_parent)),
        Some(explicit_with_releases_parent),
        "Explicit path must not be second-guessed"
    );

    // 2. start is root when start/_releases.json validates
    let ws = tmp.path().join("my-ws");
    fs::create_dir_all(&ws).unwrap();
    let cached = ods::index::CachedReleaseIndex {
        fetched_at: "2026-07-31T00:00:00Z".to_string(),
        index: ods::index::OdsReleaseIndex::baked().unwrap_or_default(),
    };
    cached.save_to_workspace(&ws).unwrap();

    assert_eq!(
        ods::workspace::find_workspace_root_from(&ws, None),
        Some(ws.clone()),
        "start must be recognized as root when _releases.json validates"
    );

    // 3. start is a release dir: start/_provenance.json exists, parent named releases, grandparent _releases.json validates
    let rel_dir = ws.join("releases").join("2026-07-31");
    fs::create_dir_all(&rel_dir).unwrap();
    let prov = ods::provenance::OdsProvenance::default();
    fs::write(
        rel_dir.join(ods::provenance::PROVENANCE_FILENAME),
        serde_json::to_string_pretty(&prov).unwrap(),
    ).unwrap();

    assert_eq!(
        ods::workspace::find_workspace_root_from(&rel_dir, None),
        Some(ws.clone()),
        "release dir with _provenance.json must resolve to enclosing workspace root"
    );

    // Test start is `current` symlink
    ods::workspace::set_active_release(&ws, "2026-07-31").unwrap();
    let current_path = ws.join("current");
    assert_eq!(
        ods::workspace::find_workspace_root_from(&current_path, None),
        Some(ws.clone()),
        "current pointer must resolve to enclosing workspace root"
    );

    // 4. Walk up
    // 4a. Ancestor directly validates
    let deep_child = ws.join("subdir").join("nested").join("deep");
    fs::create_dir_all(&deep_child).unwrap();
    assert_eq!(
        ods::workspace::find_workspace_root_from(&deep_child, None),
        Some(ws.clone()),
        "deep child must walk up to find ancestor with valid _releases.json"
    );

    // 4b. Ancestor contains default workspace (ods_data/_releases.json)
    let repo_dir = tmp.path().join("my-repo");
    let repo_ods_data = repo_dir.join("ods_data");
    fs::create_dir_all(&repo_ods_data).unwrap();
    cached.save_to_workspace(&repo_ods_data).unwrap();
    let repo_child = repo_dir.join("src").join("commands");
    fs::create_dir_all(&repo_child).unwrap();

    assert_eq!(
        ods::workspace::find_workspace_root_from(&repo_child, None),
        Some(repo_ods_data.clone()),
        "child in repo must find repo/ods_data"
    );

    // 4c. Stop boundary: .git entry prevents ascending past repo
    let outer_dir = tmp.path().join("outer_with_ws");
    fs::create_dir_all(&outer_dir).unwrap();
    cached.save_to_workspace(&outer_dir).unwrap();

    let inner_repo = outer_dir.join("inner_git_repo");
    let inner_child = inner_repo.join("sub").join("dir");
    fs::create_dir_all(&inner_child).unwrap();
    fs::create_dir_all(inner_repo.join(".git")).unwrap();

    // inner_repo has .git but NO workspace inside it. Discovery starting inside inner_child
    // must stop at inner_repo (.git boundary) and must NOT discover outer_dir
    assert_eq!(
        ods::workspace::find_workspace_root_from(&inner_child, None),
        None,
        ".git boundary must stop upward traversal before ascending past git root"
    );

    // 4d. Stop boundary: $HOME prevents ascending past home
    let fake_home = outer_dir.join("fake_home");
    let home_child = fake_home.join("workspace").join("project");
    fs::create_dir_all(&home_child).unwrap();

    assert_eq!(
        ods::workspace::find_workspace_root_from_with_home(&home_child, None, Some(&fake_home)),
        None,
        "$HOME boundary must stop upward traversal before ascending past home"
    );
}




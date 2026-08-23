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

    let release_1_dir = workspace_dir.join("releases").join("2026-05-29").join("parquet");
    fs::create_dir_all(&release_1_dir).unwrap();

    let zip1 = create_mock_trud_zip(root, "hscorgrefdataxml_data_7.0.0_20260529000001.zip");

    // Export fixture XML directly into workspace release 1
    parquet::run(parquet::Args {
        input: zip1,
        output: release_1_dir.clone(),
    })
    .expect("parquet run into release 1 should succeed");

    ods::workspace::set_active_release(&workspace_dir, "2026-05-29").unwrap();

    // Verify release 1 active status
    let (active_date, active_path) = ods::workspace::get_active_release(&workspace_dir).unwrap();
    assert_eq!(active_date, "2026-05-29");
    assert!(active_path.join("parquet").join("orgs.parquet").exists());

    // 2. Export release 2 (simulating a new monthly TRUD release)
    let release_2_dir = workspace_dir.join("releases").join("2026-06-26").join("parquet");
    fs::create_dir_all(&release_2_dir).unwrap();

    let zip2 = create_mock_trud_zip(root, "hscorgrefdataxml_data_7.0.0_20260626000001.zip");

    parquet::run(parquet::Args {
        input: zip2,
        output: release_2_dir.clone(),
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
        input: zip_path,
        output: out_dir.clone(),
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




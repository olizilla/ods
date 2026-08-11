use ods::commands::{parquet, find, diff};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

const FIXTURE_XML: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/mock_hscorgrefdata.xml"
);

#[test]
fn test_workspace_full_lifecycle() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // 1. Initialise workspace with parquet export
    let workspace_dir = root.join("ods_data");
    fs::create_dir_all(&workspace_dir).unwrap();

    let release_1_dir = workspace_dir.join("releases").join("2026-05-18").join("parquet");
    fs::create_dir_all(&release_1_dir).unwrap();

    // Export fixture XML directly into workspace release 1
    parquet::run(parquet::Args {
        input: Path::new(FIXTURE_XML).to_path_buf(),
        output: release_1_dir.clone(),
    })
    .expect("parquet run into release 1 should succeed");

    ods::workspace::set_active_release(&workspace_dir, "2026-05-18").unwrap();

    // Verify release 1 active status
    let (active_date, active_path) = ods::workspace::get_active_release(&workspace_dir).unwrap();
    assert_eq!(active_date, "2026-05-18");
    assert!(active_path.join("parquet").join("orgs.parquet").exists());

    // 2. Export release 2 (simulating a new monthly TRUD release)
    let release_2_dir = workspace_dir.join("releases").join("2026-06-22").join("parquet");
    fs::create_dir_all(&release_2_dir).unwrap();

    parquet::run(parquet::Args {
        input: Path::new(FIXTURE_XML).to_path_buf(),
        output: release_2_dir.clone(),
    })
    .expect("parquet run into release 2 should succeed");

    ods::workspace::set_active_release(&workspace_dir, "2026-06-22").unwrap();

    // 3. Test release switching
    ods::workspace::set_active_release(&workspace_dir, "2026-05-18").unwrap();

    let (switched_date, _) = ods::workspace::get_active_release(&workspace_dir).unwrap();
    assert_eq!(switched_date, "2026-05-18");

    // 4. Test auto-discovery by `ods find`
    let discovered_parquet = ods::workspace::discover_parquet_dir(Some(&workspace_dir)).unwrap();
    assert!(discovered_parquet.join("orgs.parquet").exists());

    let mut find_out = Vec::new();
    find::run_with_writer(
        find::Args {
            query: Some("Mock".to_string()),
            role: None,
            all: false,
            verbose: false,
            sort: find::SortBy::Code,
            format: find::OutputFormat::Json,
            input: discovered_parquet.clone(),
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
    use std::io::Write;

    let tmp = TempDir::new().unwrap();
    let zip_path = tmp.path().join("mock_trud_release.zip");

    // Create a zip archive containing the mock XML fixture
    let zip_file = fs::File::create(&zip_path).unwrap();
    let mut zip_writer = zip::ZipWriter::new(zip_file);
    let options = zip::write::SimpleFileOptions::default();
    zip_writer.start_file("HSCOrgRefData_Full_mock.xml", options).unwrap();

    let xml_content = fs::read_to_string(FIXTURE_XML).unwrap();
    zip_writer.write_all(xml_content.as_bytes()).unwrap();
    zip_writer.finish().unwrap();

    let out_dir = tmp.path().join("output_parquet");

    // Run `ods parquet` directly with ZIP input
    parquet::run(parquet::Args {
        input: zip_path,
        output: out_dir.clone(),
    })
    .expect("ods parquet should successfully handle TRUD zip archive input");

    assert!(out_dir.join("orgs.parquet").exists());
    assert!(out_dir.join("roles.parquet").exists());
    assert!(out_dir.join("rels.parquet").exists());
}


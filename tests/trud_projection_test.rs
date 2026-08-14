//! Integration tests enforcing dataset release invariants and projection parity.

use ods::commands::parquet;
use ods::workspace;
use std::fs::{self, File};
use std::io::Write;
use tempfile::TempDir;

const FIXTURE_XML: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/mock_hscorgrefdata.xml"
);

/// 1. Verifies that `extract_xml_from_zip` prioritizes `fullfile.zip` over `archive.zip`
/// when processing a TRUD distribution package containing both inner archives.
#[test]
fn test_trud_zip_selection_prioritizes_full_over_archive() {
    let tmp = TempDir::new().unwrap();
    let outer_zip_path = tmp.path().join("mock_trud_package.zip");

    // Create mock archive.zip (containing 1 archive XML record)
    let archive_xml_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<un:OrganisationManifest xmlns:un="http://refdata.hscic.gov.uk/org/v2-0-0">
  <un:ManifestHeader>
    <un:PublicationType value="Full" />
    <un:PublicationDate value="2026-05-18" />
    <un:PublicationSeqNum value="1" />
  </un:ManifestHeader>
  <un:Organisations>
    <un:Organisation>
      <un:Name>ARCHIVE ONLY RECORD</un:Name>
      <un:OrgId root="2.16.840.1.113883.2.1.3.2.4.18.48" extension="ARCH1" />
      <un:Status value="Active" />
    </un:Organisation>
  </un:Organisations>
</un:OrganisationManifest>"#;

    let archive_zip_bytes = create_inner_zip("HSCOrgRefData_Archive_20260518.xml", archive_xml_content);
    let full_xml_content = fs::read_to_string(FIXTURE_XML).unwrap();
    let full_zip_bytes = create_inner_zip("HSCOrgRefData_Full_20260518.xml", &full_xml_content);

    // Write outer release package containing BOTH archive.zip and fullfile.zip
    // Note: archive.zip is added first alphabetically to test priority handling
    let outer_file = File::create(&outer_zip_path).unwrap();
    let mut outer_zip = zip::ZipWriter::new(outer_file);
    let options = zip::write::SimpleFileOptions::default();

    outer_zip.start_file("archive.zip", options).unwrap();
    outer_zip.write_all(&archive_zip_bytes).unwrap();

    outer_zip.start_file("fullfile.zip", options).unwrap();
    outer_zip.write_all(&full_zip_bytes).unwrap();

    outer_zip.finish().unwrap();

    // Run parquet export
    let out_dir = tmp.path().join("parquet_out");
    parquet::run(parquet::Args {
        input: outer_zip_path,
        output: out_dir.clone(),
    })
    .expect("parquet run on dual-archive zip should succeed");

    // Verify that fullfile.zip was selected (producing 2 records from fixture, not 1 from archive)
    let orgs_parquet = out_dir.join("orgs.parquet");
    assert!(orgs_parquet.exists());

    let total_rows = workspace::count_records_in_parquet(&orgs_parquet).unwrap();

    assert_eq!(
        total_rows, 2,
        "ods parquet must select fullfile.zip (producing 2 records), not archive.zip (1 record)"
    );
}

/// 2. Verifies that child `<Status value="Inactive"/>` tags on secondary roles
/// or relationships do not corrupt/overwrite the top-level organisation status.
#[test]
fn test_status_scoping_prevents_role_status_leak() {
    let tmp = TempDir::new().unwrap();
    let xml_path = tmp.path().join("status_leak_test.xml");

    let xml_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<un:OrganisationManifest xmlns:un="http://refdata.hscic.gov.uk/org/v2-0-0">
  <un:ManifestHeader>
    <un:PublicationType value="Full" />
    <un:PublicationDate value="2026-05-18" />
    <un:PublicationSeqNum value="1" />
    <un:PrimaryRoleScope>
      <un:PrimaryRole id="RO197" displayName="NHS Trust" />
    </un:PrimaryRoleScope>
  </un:ManifestHeader>
  <un:Organisations>
    <un:Organisation orgRecordClass="RC1">
      <un:Name>ACTIVE ORG WITH INACTIVE ROLE</un:Name>
      <un:OrgId root="2.16.840.1.113883.2.1.3.2.4.18.48" extension="TEST1" />
      <un:Status value="Active" />
      <un:Roles>
        <un:Role id="RO197" uniqueRoleId="100" primaryRole="true">
          <un:Status value="Inactive" />
        </un:Role>
      </un:Roles>
    </un:Organisation>
  </un:Organisations>
</un:OrganisationManifest>"#;

    fs::write(&xml_path, xml_content).unwrap();

    let out_dir = tmp.path().join("parquet_out");
    parquet::run(parquet::Args {
        input: xml_path,
        output: out_dir.clone(),
    })
    .unwrap();

    let orgs_parquet = out_dir.join("orgs.parquet");
    assert!(orgs_parquet.exists());

    let total_rows = workspace::count_records_in_parquet(&orgs_parquet).unwrap();

    assert_eq!(
        total_rows, 1,
        "organisation with active status must be exported to orgs.parquet despite inactive child role"
    );
}

/// Helper to construct an in-memory zip file containing a named XML file.
fn create_inner_zip(filename: &str, content: &str) -> Vec<u8> {
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file(filename, options).unwrap();
        zip.write_all(content.as_bytes()).unwrap();
        zip.finish().unwrap();
    }
    cursor.into_inner()
}
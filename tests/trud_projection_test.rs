mod common;

use common::{create_inner_zip, create_nested_trud_zip, FIXTURE_MOCK_XML};
use ods::commands::parquet;
use ods::workspace;
use std::fs;
use tempfile::TempDir;

/// 1. Verifies that `extract_xml_from_zip` prioritizes `fullfile.zip` over `archive.zip`
/// when processing a TRUD distribution package containing both inner archives.
#[test]
fn test_trud_zip_selection_prioritizes_full_over_archive() {
    let tmp = TempDir::new().unwrap();
    let outer_zip_path = tmp.path().join("hscorgrefdataxml_data_7.0.0_20260529000001.zip");

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

    let archive_zip_bytes = create_inner_zip("HSCOrgRefData_Archive_20260518.xml", archive_xml_content.as_bytes());
    let full_xml_content = fs::read_to_string(FIXTURE_MOCK_XML).unwrap();
    let full_zip_bytes = create_inner_zip("HSCOrgRefData_Full_20260518.xml", full_xml_content.as_bytes());

    // Write outer release package containing BOTH archive.zip and fullfile.zip
    // Note: archive.zip is added first alphabetically to test priority handling
    create_nested_trud_zip(
        &outer_zip_path,
        &[
            ("archive.zip", &archive_zip_bytes),
            ("fullfile.zip", &full_zip_bytes),
        ],
    );

    // Run parquet export
    let out_dir = tmp.path().join("parquet_out");
    parquet::run(parquet::Args {
        input: Some(outer_zip_path),
        output: Some(out_dir.clone()),
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
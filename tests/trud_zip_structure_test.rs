use anyhow::Result;
use std::fs::File;
use std::io::Write;
use tempfile::TempDir;

#[test]
fn test_find_xml_file_succeeds_for_expected_trud_xml_filepath() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let zip_path = temp_dir.path().join("valid_trud_package.zip");

    let xml_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<un:OrganisationManifest xmlns:un="http://refdata.hscic.gov.uk/org/v2-0-0">
  <un:ManifestHeader>
    <un:PublicationType value="Full" />
    <un:PublicationDate value="2026-07-31" />
  </un:ManifestHeader>
</un:OrganisationManifest>"#;

    let file = File::create(&zip_path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();

    zip.start_file("HSCOrgRefData_Full_20260731.xml", options)?;
    zip.write_all(xml_content.as_bytes())?;
    zip.finish()?;

    let extracted_xml = ods::ods_xml::find_xml_file(&zip_path)?;
    assert!(extracted_xml.exists(), "extracted XML file must exist");
    assert!(
        extracted_xml.file_name().unwrap().to_str().unwrap().contains("HSCOrgRefData"),
        "extracted file must be the expected TRUD XML file"
    );

    Ok(())
}

#[test]
fn test_find_xml_file_fails_when_expected_xml_absent() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let zip_path = temp_dir.path().join("invalid_package.zip");

    let file = File::create(&zip_path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();

    zip.start_file("unrelated_document.txt", options)?;
    zip.write_all(b"Hello World")?;
    zip.finish()?;

    let result = ods::ods_xml::find_xml_file(&zip_path);
    assert!(result.is_err(), "find_xml_file must fail when no TRUD XML is in the zip");

    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("No XML file found") || err_msg.contains("XML"),
        "error message must describe missing XML, got: {}",
        err_msg
    );

    Ok(())
}

#[test]
fn test_find_xml_file_fails_when_zip_contains_only_archive_zip() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let outer_zip_path = temp_dir.path().join("archive_only_package.zip");

    let archive_xml_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<un:OrganisationManifest xmlns:un="http://refdata.hscic.gov.uk/org/v2-0-0">
  <un:ManifestHeader>
    <un:PublicationType value="Full" />
  </un:ManifestHeader>
</un:OrganisationManifest>"#;

    let inner_zip_path = temp_dir.path().join("archive.zip");
    let inner_file = File::create(&inner_zip_path)?;
    let mut inner_zip = zip::ZipWriter::new(inner_file);
    let options = zip::write::SimpleFileOptions::default();
    inner_zip.start_file("HSCOrgRefData_Archive_20260518.xml", options)?;
    inner_zip.write_all(archive_xml_content.as_bytes())?;
    inner_zip.finish()?;

    let inner_zip_bytes = std::fs::read(&inner_zip_path)?;

    let outer_file = File::create(&outer_zip_path)?;
    let mut outer_zip = zip::ZipWriter::new(outer_file);
    outer_zip.start_file("archive.zip", options)?;
    outer_zip.write_all(&inner_zip_bytes)?;
    outer_zip.finish()?;

    let result = ods::ods_xml::find_xml_file(&outer_zip_path);
    assert!(result.is_err(), "find_xml_file must fail when ZIP contains only archive.zip");

    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("archive.zip") || err_msg.contains("archive XML"),
        "error message must state that archive.zip is rejected, got: {}",
        err_msg
    );

    Ok(())
}

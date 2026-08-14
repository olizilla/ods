use anyhow::Result;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_provenance_validate_baseline_missing_fields() -> Result<()> {
    let mut prov = ods::provenance::OdsProvenance::default();
    assert!(prov.validate_baseline().is_err(), "empty provenance must fail baseline validation");

    prov.trud_release_name = Some("Release 7.0.0".to_string());
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_file = Some("hscorgrefdataxml_data_7.0.0_20260731000001.zip".to_string());
    prov.trud_release_filesize_bytes = Some(37_983_173);
    prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());
    prov.trud_release_sha256_verified = Some(true);
    prov.trud_release_url = Some("https://isd.digital.nhs.uk/trud/items/341/hscorgrefdataxml_data_7.0.0_20260731000001.zip".to_string());

    assert!(prov.validate_baseline().is_ok(), "provenance with all baseline fields must pass validation");
    Ok(())
}

#[test]
fn test_provenance_validate_baseline_rejects_file_url() -> Result<()> {
    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_name = Some("Release 7.0.0".to_string());
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_file = Some("hscorgrefdataxml_data_7.0.0_20260731000001.zip".to_string());
    prov.trud_release_filesize_bytes = Some(37_983_173);
    prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());
    prov.trud_release_sha256_verified = Some(true);

    prov.trud_release_url = Some("file:///tmp/hscorgrefdataxml_data_7.0.0_20260731000001.zip".to_string());
    assert!(prov.validate_baseline().is_err(), "file:// URL in trud_release_url must be rejected");

    prov.trud_release_url = Some("/local/path/hscorgrefdataxml_data_7.0.0_20260731000001.zip".to_string());
    assert!(prov.validate_baseline().is_err(), "local path in trud_release_url must be rejected");

    Ok(())
}

#[test]
fn test_provenance_validate_baseline_rejects_implausible_filesize() -> Result<()> {
    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_name = Some("Release 7.0.0".to_string());
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_file = Some("hscorgrefdataxml_data_7.0.0_20260731000001.zip".to_string());
    prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());
    prov.trud_release_sha256_verified = Some(true);
    prov.trud_release_url = Some("https://isd.digital.nhs.uk/trud/items/341/hscorgrefdataxml_data_7.0.0_20260731000001.zip".to_string());

    prov.trud_release_filesize_bytes = Some(16); // 16 bytes is implausible for TRUD zip archive
    assert!(prov.validate_baseline().is_err(), "implausible filesize (16 bytes) must be rejected");

    Ok(())
}

#[test]
fn test_provenance_validate_baseline_rejects_filename_mismatch() -> Result<()> {
    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_name = Some("Release 7.0.0".to_string());
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_filesize_bytes = Some(37_983_173);
    prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());
    prov.trud_release_sha256_verified = Some(true);
    prov.trud_release_url = Some("https://isd.digital.nhs.uk/trud/items/341/hscorgrefdataxml_data_7.0.0_20260731000001.zip".to_string());

    prov.trud_release_file = Some("hscorgrefdataxml_data_6.0.0_20250627000001.zip".to_string());
    assert!(prov.validate_baseline().is_err(), "mismatched version/date in trud_release_file must be rejected");

    Ok(())
}

#[test]
fn test_make_fails_without_valid_provenance() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let input_dir = temp_dir.path().join("input");
    fs::create_dir_all(&input_dir)?;

    let args = ods::commands::parquet::Args {
        input: input_dir.clone(),
        output: temp_dir.path().join("output"),
    };

    let result = ods::commands::parquet::run(args);
    assert!(result.is_err(), "ods make/parquet must fail when _provenance.json is missing");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("_provenance.json") || err_msg.contains("provenance"),
        "error message must mention _provenance.json, got: {}",
        err_msg
    );

    Ok(())
}

#[test]
fn test_provenance_preserves_xml_manifest_fields_on_make() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let output_dir = temp_dir.path();

    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_name = Some("Release 7.0.0".to_string());
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_file = Some("hscorgrefdataxml_data_7.0.0_20260731000001.zip".to_string());
    prov.trud_release_filesize_bytes = Some(37_983_173);
    prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());
    prov.trud_release_sha256_verified = Some(true);
    prov.trud_release_url = Some("https://isd.digital.nhs.uk/trud/items/341/hscorgrefdataxml_data_7.0.0_20260731000001.zip".to_string());
    
    prov.publication_date = Some("2026-07-28".to_string());
    prov.publication_seq_num = Some("4700".to_string());
    prov.publication_type = Some("Full".to_string());
    prov.publication_source = Some("HSCIC".to_string());
    prov.publication_record_count = Some(305541);

    let prov_json = serde_json::to_string_pretty(&prov)?;
    fs::write(output_dir.join(ods::provenance::PROVENANCE_FILENAME), prov_json)?;

    // Run update_provenance_and_write_sha256sums
    ods::provenance::update_provenance_and_write_sha256sums(output_dir)?;

    let saved_json = fs::read_to_string(output_dir.join(ods::provenance::PROVENANCE_FILENAME))?;
    assert!(saved_json.contains("publication_date"), "publication_date must be preserved");
    assert!(saved_json.contains("2026-07-28"), "publication_date value must be preserved");
    assert!(saved_json.contains("publication_seq_num"), "publication_seq_num must be preserved");
    assert!(saved_json.contains("4700"), "publication_seq_num value must be preserved");
    assert!(saved_json.contains("publication_type"), "publication_type must be preserved");
    assert!(saved_json.contains("Full"), "publication_type value must be preserved");
    assert!(saved_json.contains("publication_source"), "publication_source must be preserved");
    assert!(saved_json.contains("publication_record_count"), "publication_record_count must be preserved");
    assert!(saved_json.contains("305541"), "publication_record_count value must be preserved");
    assert!(!saved_json.contains("xml_manifest_created"), "xml_manifest_created must NOT be in JSON");
    assert!(!saved_json.contains("ods_cmd_version"), "ods_cmd_version must NOT be in JSON");

    Ok(())
}

#[test]
fn test_cite_output_formats_and_attribution() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let output_dir = temp_dir.path();

    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_name = Some("Release 7.0.0".to_string());
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_file = Some("hscorgrefdataxml_data_7.0.0_20260731000001.zip".to_string());
    prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());
    prov.trud_release_sha256_verified = Some(true);
    prov.trud_release_url = Some("https://isd.digital.nhs.uk/trud/items/341/hscorgrefdataxml_data_7.0.0_20260731000001.zip".to_string());

    prov.publication_date = Some("2026-07-28".to_string());
    prov.publication_seq_num = Some("4700".to_string());
    prov.publication_type = Some("Full".to_string());
    prov.publication_source = Some("HSCIC".to_string());
    prov.publication_record_count = Some(305541);

    let prov_json = serde_json::to_string_pretty(&prov)?;
    fs::write(output_dir.join(ods::provenance::PROVENANCE_FILENAME), prov_json)?;
    fs::write(output_dir.join("orgs.parquet"), b"mock parquet data")?;

    for fmt in ["text", "bibtex", "csljson", "apa"] {
        let args = ods::commands::cite::Args {
            input: Some(output_dir.to_path_buf()),
            format: fmt.to_string(),
        };
        let mut buf = Vec::new();
        ods::commands::cite::run_with_writer(args, &mut buf)?;
        let output = String::from_utf8(buf)?;

        assert!(output.contains("NHS England"), "cite format {} must attribute to NHS England", fmt);
        assert!(!output.contains("NHS Digital"), "cite format {} must NOT mention NHS Digital", fmt);
        assert!(!output.contains("HSCIC"), "cite format {} must NOT mention HSCIC", fmt);
        assert!(output.contains("2026-07-28"), "cite format {} must contain publication date 2026-07-28", fmt);
        assert!(output.contains("4700"), "cite format {} must contain publication sequence number 4700", fmt);
    }

    Ok(())
}

#[test]
fn test_cite_publication_date_no_cross_namespace_fallback() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let output_dir = temp_dir.path();

    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_name = Some("Release 7.0.0".to_string());
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_file = Some("hscorgrefdataxml_data_7.0.0_20260731000001.zip".to_string());
    prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());
    prov.trud_release_sha256_verified = Some(true);
    prov.trud_release_url = Some("https://isd.digital.nhs.uk/trud/items/341/hscorgrefdataxml_data_7.0.0_20260731000001.zip".to_string());

    // NOTE: publication_date is deliberately None here!
    prov.publication_date = None;
    prov.publication_seq_num = Some("4700".to_string());
    prov.publication_type = Some("Full".to_string());

    let prov_json = serde_json::to_string_pretty(&prov)?;
    fs::write(output_dir.join(ods::provenance::PROVENANCE_FILENAME), prov_json)?;
    fs::write(output_dir.join("orgs.parquet"), b"mock parquet data")?;

    let args = ods::commands::cite::Args {
        input: Some(output_dir.to_path_buf()),
        format: "text".to_string(),
    };
    let mut buf = Vec::new();
    ods::commands::cite::run_with_writer(args, &mut buf)?;
    let output = String::from_utf8(buf)?;

    // Must NOT borrow trud_release_date ("2026-07-31") for Publication date
    assert!(
        !output.contains("Publication date:   2026-07-31"),
        "ods cite must NOT borrow trud_release_date for publication date, output was:\n{}",
        output
    );

    Ok(())
}

#[test]
fn test_update_provenance_populates_missing_publication_fields_from_xml() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let output_dir = temp_dir.path();
    let trud_dir = output_dir.join("trud");
    fs::create_dir_all(&trud_dir)?;

    let xml_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<Manifest>
    <Version value="2-0-0" />
    <PublicationType value="Full" />
    <PublicationSource value="HSCIC" />
    <PublicationDate value="2026-07-28" />
    <PublicationSeqNum value="4700" />
    <RecordCount value="0" />
</Manifest>"#;
    fs::write(trud_dir.join("HSCOrgRefData.xml"), xml_content)?;

    // Write a legacy/stale _provenance.json containing ONLY trud_* fields
    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_name = Some("Release 7.0.0".to_string());
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_file = Some("hscorgrefdataxml_data_7.0.0_20260731000001.zip".to_string());
    prov.trud_release_filesize_bytes = Some(37_983_173);
    prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());
    prov.trud_release_sha256_verified = Some(true);
    prov.trud_release_url = Some("https://isd.digital.nhs.uk/trud/items/341/hscorgrefdataxml_data_7.0.0_20260731000001.zip".to_string());
    
    // Explicitly NO publication_* fields set here!

    let prov_json = serde_json::to_string_pretty(&prov)?;
    fs::write(output_dir.join(ods::provenance::PROVENANCE_FILENAME), prov_json)?;

    // Run update_provenance_and_write_sha256sums
    ods::provenance::update_provenance_and_write_sha256sums(output_dir)?;

    let saved_json = fs::read_to_string(output_dir.join(ods::provenance::PROVENANCE_FILENAME))?;
    let updated_prov: ods::provenance::OdsProvenance = serde_json::from_str(&saved_json)?;

    assert_eq!(updated_prov.publication_date, Some("2026-07-28".to_string()), "publication_date must be populated from XML manifest");
    assert_eq!(updated_prov.publication_seq_num, Some("4700".to_string()), "publication_seq_num must be populated from XML manifest");
    assert_eq!(updated_prov.publication_type, Some("Full".to_string()), "publication_type must be populated from XML manifest");
    assert_eq!(updated_prov.publication_source, Some("HSCIC".to_string()), "publication_source must be populated from XML manifest");
    assert_eq!(updated_prov.trud_release_name, Some("Release 7.0.0".to_string()), "trud_release_name must be preserved");

    Ok(())
}

#[test]
fn test_primary_role_scope_parsing_and_export() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let xml_path = temp_dir.path().join("test_manifest.xml");

    let xml_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<un:OrganisationManifest xmlns:un="http://refdata.hscic.gov.uk/org/v2-0-0">
  <un:ManifestHeader>
    <un:PrimaryRoleScope>
      <un:PrimaryRole id="RO180" displayName="PRIMARY CARE TRUST SITE" />
      <un:PrimaryRole id="RO198" displayName="NHS TRUST SITE" />
    </un:PrimaryRoleScope>
  </un:ManifestHeader>
  <un:Organisations>
    <un:Organisation>
      <un:Name>TEST ORG</un:Name>
      <un:OrgId root="2.16.840.1.113883.2.1.3.2.4.18.48" extension="A100" />
      <un:Status value="Active" />
      <un:OrgRecordClass value="RC1" />
      <un:PrimaryRoleId id="RO180" uniqueRoleId="1" status="Active" display_name="Primary Role" />
      <un:RoleId id="RO177" uniqueRoleId="2" status="Active" display_name="Inactive-only primary candidate" />
    </un:Organisation>
  </un:Organisations>
</un:OrganisationManifest>"#;

    fs::write(&xml_path, xml_content)?;
    let (prov, _concept_map, _parsed) = ods::commands::ndjson::parse_single_pass(&xml_path)?;

    let scope = prov.primary_role_scope.as_ref().expect("primary_role_scope must be parsed");
    assert_eq!(scope, &vec!["RO180".to_string(), "RO198".to_string()]);

    Ok(())
}


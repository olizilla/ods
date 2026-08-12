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
    prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());
    prov.trud_release_sha256_verified = Some(true);

    assert!(prov.validate_baseline().is_ok(), "provenance with all baseline fields must pass validation");
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
    prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());
    prov.trud_release_sha256_verified = Some(true);
    prov.xml_manifest_created = Some("2026-07-28T16:46:22".to_string());
    prov.xml_manifest_seq_num = Some("4700".to_string());
    prov.xml_manifest_record_count = Some(305541);

    let prov_json = serde_json::to_string_pretty(&prov)?;
    fs::write(output_dir.join(ods::provenance::PROVENANCE_FILENAME), prov_json)?;

    // Run update_provenance_and_write_sha256sums
    ods::provenance::update_provenance_and_write_sha256sums(output_dir)?;

    let saved_json = fs::read_to_string(output_dir.join(ods::provenance::PROVENANCE_FILENAME))?;
    assert!(saved_json.contains("xml_manifest_created"), "xml_manifest_created must be preserved");
    assert!(saved_json.contains("2026-07-28T16:46:22"), "xml_manifest_created value must be preserved");
    assert!(saved_json.contains("xml_manifest_seq_num"), "xml_manifest_seq_num must be preserved");
    assert!(saved_json.contains("4700"), "xml_manifest_seq_num value must be preserved");
    assert!(saved_json.contains("xml_manifest_record_count"), "xml_manifest_record_count must be preserved");
    assert!(saved_json.contains("305541"), "xml_manifest_record_count value must be preserved");

    Ok(())
}

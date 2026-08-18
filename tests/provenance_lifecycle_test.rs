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
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);

    assert!(prov.validate_baseline().is_ok(), "provenance with all baseline fields must pass validation");
    Ok(())
}

#[test]
fn test_provenance_validate_baseline_rejects_implausible_filesize() -> Result<()> {
    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_name = Some("Release 7.0.0".to_string());
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_file = Some("hscorgrefdataxml_data_7.0.0_20260731000001.zip".to_string());
    prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);

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
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);

    prov.trud_release_file = Some("hscorgrefdataxml_data_6.0.0_20250627000001.zip".to_string());
    assert!(prov.validate_baseline().is_err(), "mismatched version/date in trud_release_file must be rejected");

    Ok(())
}

#[test]
fn test_make_fails_without_valid_provenance() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let input_dir = temp_dir.path().join("input");
    fs::create_dir_all(&input_dir)?;

    let zip_path = input_dir.join("hscorgrefdataxml_data_7.0.0_20260731000001.zip");
    let zip_file = fs::File::create(&zip_path)?;
    let mut zip_writer = zip::ZipWriter::new(zip_file);
    let options = zip::write::SimpleFileOptions::default();
    zip_writer.start_file("HSCOrgRefData_Full.xml", options)?;
    zip_writer.finish()?;

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
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);
    
    prov.publication_date = Some("2026-07-28".to_string());
    prov.publication_seq_num = Some("4700".to_string());
    prov.publication_type = Some("Full".to_string());
    prov.publication_source = Some("HSCIC".to_string());
    prov.publication_record_count = Some(305541);

    let prov_json = serde_json::to_string_pretty(&prov)?;
    fs::write(output_dir.join(ods::provenance::PROVENANCE_FILENAME), prov_json)?;

    // Run update_provenance_and_write_sha256sums
    ods::provenance::update_provenance_and_write_sha256sums(output_dir, None)?;

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
    assert!(!saved_json.contains("tool_parquet_version"), "tool_parquet_version must NOT be in JSON");
    assert!(!saved_json.contains("tool_arrow_version"), "tool_arrow_version must NOT be in JSON");
    assert!(!saved_json.contains("tool_zstd_level"), "tool_zstd_level must NOT be in JSON");

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
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);

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
fn test_cite_declines_unverified_release() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let output_dir = temp_dir.path();

    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_name = Some("Release 7.0.0".to_string());
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_file = Some("hscorgrefdataxml_data_7.0.0_20260731000001.zip".to_string());
    prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::Unverified);

    let prov_json = serde_json::to_string_pretty(&prov)?;
    fs::write(output_dir.join(ods::provenance::PROVENANCE_FILENAME), prov_json)?;
    fs::write(output_dir.join("orgs.parquet"), b"mock parquet data")?;

    let args = ods::commands::cite::Args {
        input: Some(output_dir.to_path_buf()),
        format: "text".to_string(),
    };
    let mut buf = Vec::new();
    let result = ods::commands::cite::run_with_writer(args, &mut buf);
    assert!(result.is_err(), "ods cite must decline to generate citation for unverified release");
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("unverified release"), "error must mention unverified release, got: {}", err_msg);

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
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);

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
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);
    
    // Explicitly NO publication_* fields set here!

    let prov_json = serde_json::to_string_pretty(&prov)?;
    fs::write(output_dir.join(ods::provenance::PROVENANCE_FILENAME), prov_json)?;

    // Run update_provenance_and_write_sha256sums
    ods::provenance::update_provenance_and_write_sha256sums(output_dir, None)?;

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
    let header = ods::commands::ndjson::extract_manifest_header(&xml_path)?;

    let scope = header.primary_role_scope.expect("primary_role_scope must be parsed");
    assert_eq!(scope, vec!["RO180".to_string(), "RO198".to_string()]);

    Ok(())
}

#[test]
fn test_sha256sums_and_dataset_file_sha256_are_uppercase_and_match() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let output_dir = temp_dir.path();

    // Create dummy files for artifacts
    let files = [
        "orgs.parquet",
        "orgs_all.parquet",
        "org_roles.parquet",
        "roles.parquet",
        "relationships.parquet",
        "successions.parquet",
        "category_rules.json",
        "datapackage.json",
    ];
    for f in &files {
        fs::write(output_dir.join(f), format!("dummy content for {}", f))?;
    }

    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_date = Some("2026-07-31".to_string());
    fs::write(
        output_dir.join(ods::provenance::PROVENANCE_FILENAME),
        serde_json::to_string_pretty(&prov)?,
    )?;

    ods::provenance::update_provenance_and_write_sha256sums(output_dir, Some(1))?;

    let prov_content = fs::read_to_string(output_dir.join(ods::provenance::PROVENANCE_FILENAME))?;
    let updated_prov: ods::provenance::OdsProvenance = serde_json::from_str(&prov_content)?;
    let dataset_files = updated_prov.dataset_file_sha256.expect("dataset_file_sha256 must be present");

    assert_eq!(updated_prov.dataset_revision, Some(1));
    assert_eq!(updated_prov.dataset_parquet_schema_version, Some("0.1.0".to_string()));

    let sha256sums_content = fs::read_to_string(output_dir.join("SHA256SUMS"))?;
    
    for line in sha256sums_content.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() == 2 {
            let hash = parts[0];
            let filename = parts[1];

            // Verify hash is uppercase
            assert_eq!(
                hash,
                hash.to_uppercase(),
                "hash in SHA256SUMS for {} must be uppercase, got {}",
                filename,
                hash
            );

            if filename != ods::provenance::PROVENANCE_FILENAME {
                let prov_hash = dataset_files.get(filename).unwrap_or_else(|| {
                    panic!("{} not found in _provenance.json dataset_file_sha256", filename)
                });
                assert_eq!(
                    hash,
                    prov_hash,
                    "SHA256SUMS and _provenance.json dataset_file_sha256 must agree exactly (including case) for {}",
                    filename
                );
            }
        }
    }

    Ok(())
}

#[test]
fn test_make_verifies_archive_checksum_before_amending_provenance() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let output_dir = temp_dir.path();
    let trud_dir = output_dir.join("trud");
    fs::create_dir_all(&trud_dir)?;

    let archive_file = "hscorgrefdataxml_data_7.0.0_20260731000001.zip";
    let archive_path = trud_dir.join(archive_file);
    fs::write(&archive_path, b"corrupted archive content")?;

    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_file = Some(archive_file.to_string());
    prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());
    fs::write(
        output_dir.join(ods::provenance::PROVENANCE_FILENAME),
        serde_json::to_string_pretty(&prov)?,
    )?;

    let result = ods::provenance::update_provenance_and_write_sha256sums(output_dir, None);
    assert!(result.is_err(), "update_provenance_and_write_sha256sums must fail when trud archive hash mismatches");
    let err = result.unwrap_err().to_string();
    assert!(err.contains("verification mismatch"), "error must mention verification mismatch, got: {}", err);

    Ok(())
}

#[test]
fn test_trud_pull_writes_no_tool_or_dataset_keys() -> Result<()> {
    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_name = Some("Release 7.0.0".to_string());
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_file = Some("hscorgrefdataxml_data_7.0.0_20260731000001.zip".to_string());
    prov.trud_release_filesize_bytes = Some(37983173);
    prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);
    prov.publication_date = Some("2026-07-28".to_string());
    prov.publication_seq_num = Some("4700".to_string());
    prov.publication_type = Some("Full".to_string());
    prov.publication_source = Some("HSCIC".to_string());
    prov.publication_record_count = Some(305541);

    let prov_json = serde_json::to_string_pretty(&prov)?;
    let raw_val: serde_json::Value = serde_json::from_str(&prov_json)?;
    let obj = raw_val.as_object().expect("provenance must be JSON object");

    // Must ONLY contain _type, trud_*, and publication_* keys
    for key in obj.keys() {
        assert!(
            key == "_type" || key.starts_with("trud_") || key.starts_with("publication_"),
            "trud pull baseline must NOT write key '{}'. Only _type, trud_*, and publication_* allowed.",
            key
        );
    }

    assert!(!obj.contains_key("trud_release_url"), "trud_release_url must not exist in provenance");
    assert!(!obj.contains_key("tool_version"), "tool_version must not exist after trud pull");
    assert!(!obj.contains_key("tool_git_sha"), "tool_git_sha must not exist after trud pull");
    assert!(!obj.contains_key("tool_git_dirty"), "tool_git_dirty must not exist after trud pull");
    assert!(!obj.contains_key("dataset_revision"), "dataset_revision must not exist after trud pull");
    assert!(!obj.contains_key("dataset_parquet_schema_version"), "dataset_parquet_schema_version must not exist after trud pull");
    assert!(!obj.contains_key("dataset_file_sha256"), "dataset_file_sha256 must not exist after trud pull");

    Ok(())
}

#[test]
fn test_release_datapackage_contains_enriched_fields() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let output_dir = temp_dir.path();

    // Create dummy files for resources
    let files = [
        "orgs.parquet",
        "orgs_all.parquet",
        "org_roles.parquet",
        "roles.parquet",
        "relationships.parquet",
        "successions.parquet",
    ];
    for f in &files {
        fs::write(output_dir.join(f), format!("data for {}", f))?;
    }

    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_name = Some("Release 7.0.0".to_string());
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.dataset_doi = Some("10.5281/zenodo.1234567".to_string());

    let release_pkg = ods::datapackage::generate_release_datapackage(output_dir, Some(&prov));

    assert_eq!(release_pkg["id"], "10.5281/zenodo.1234567");
    let sources = release_pkg["sources"].as_array().expect("sources must be array");
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0]["title"], "Release 7.0.0");
    assert_eq!(sources[0]["version"], "2026-07-31");
    assert!(sources[0].get("path").is_none(), "sources[0] must not contain path");

    let resources = release_pkg["resources"].as_array().expect("resources must be array");
    for res in resources {
        let name = res["name"].as_str().unwrap();
        assert!(res.get("bytes").is_some(), "resource '{}' must have 'bytes'", name);
        assert!(res.get("hash").is_some(), "resource '{}' must have 'hash'", name);
        let hash_str = res["hash"].as_str().unwrap();
        assert!(hash_str.starts_with("sha256:"), "hash format must be sha256:<hex>, got: {}", hash_str);
    }

    Ok(())
}

#[test]
fn test_reproducibility_two_different_working_directories_produce_identical_sha256sums() -> Result<()> {
    let tmp_a = TempDir::new()?;
    let tmp_b = TempDir::new()?;

    let xml_fixture = "tests/fixtures/mock_hscorgrefdata.xml";
    let xml_bytes = fs::read(xml_fixture)?;

    // Create the archive bytes once so both directories start from the exact same TRUD zip archive
    let mut zip_bytes = Vec::new();
    {
        let cursor = std::io::Cursor::new(&mut zip_bytes);
        let mut zip = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default()
            .last_modified_time(zip::DateTime::from_date_and_time(2026, 7, 31, 0, 0, 0).unwrap());
        zip.start_file("HSCOrgRefData_Full.xml", options)?;
        use std::io::Write;
        zip.write_all(&xml_bytes)?;
        zip.finish()?;
    }

    for tmp in [&tmp_a, &tmp_b] {
        let zip_path = tmp.path().join("hscorgrefdataxml_data_7.0.0_20260731000001.zip");
        fs::write(&zip_path, &zip_bytes)?;

        let out_dir = tmp.path().join("out");
        let args = ods::commands::parquet::Args {
            input: zip_path,
            output: out_dir.clone(),
        };
        ods::commands::parquet::run(args)?;
        ods::provenance::update_provenance_and_write_sha256sums(&out_dir, Some(1))?;
    }

    let prov_a = fs::read_to_string(tmp_a.path().join("out").join("_provenance.json"))?;
    let prov_b = fs::read_to_string(tmp_b.path().join("out").join("_provenance.json"))?;
    assert_eq!(prov_a, prov_b, "_provenance.json must be byte-identical regardless of working directory");

    let dp_a = fs::read_to_string(tmp_a.path().join("out").join("datapackage.json"))?;
    let dp_b = fs::read_to_string(tmp_b.path().join("out").join("datapackage.json"))?;
    assert_eq!(dp_a, dp_b, "datapackage.json must be byte-identical regardless of working directory");

    let sums_a = fs::read_to_string(tmp_a.path().join("out").join("SHA256SUMS"))?;
    let sums_b = fs::read_to_string(tmp_b.path().join("out").join("SHA256SUMS"))?;
    assert_eq!(sums_a, sums_b, "SHA256SUMS must be byte-identical regardless of working directory");

    // Assert no trud_release_url in provenance
    assert!(!prov_a.contains("trud_release_url"), "_provenance.json must not contain trud_release_url");

    // Assert no absolute paths leaked
    let tmp_a_path = tmp_a.path().to_str().unwrap();
    let tmp_b_path = tmp_b.path().to_str().unwrap();
    assert!(!prov_a.contains(tmp_a_path), "no absolute path leaked in _provenance.json");
    assert!(!dp_a.contains(tmp_a_path), "no absolute path leaked in datapackage.json");
    assert!(!prov_b.contains(tmp_b_path), "no absolute path leaked in _provenance.json");
    assert!(!dp_b.contains(tmp_b_path), "no absolute path leaked in datapackage.json");

    Ok(())
}

#[test]
fn test_concurrent_extractions_with_identical_inner_filenames_do_not_collide() -> Result<()> {
    use std::sync::{Arc, Barrier};
    use std::thread;

    let tmp = TempDir::new()?;
    let zip_alpha_path = tmp.path().join("alpha_release.zip");
    let zip_beta_path = tmp.path().join("beta_release.zip");

    let xml_alpha = r#"<?xml version="1.0" encoding="UTF-8"?>
<Manifest>
    <Version value="2-0-0" />
    <PublicationType value="Full" />
    <PublicationSource value="HSCIC" />
    <PublicationDate value="2026-07-28" />
    <PublicationSeqNum value="1111" />
</Manifest>"#;

    let xml_beta = r#"<?xml version="1.0" encoding="UTF-8"?>
<Manifest>
    <Version value="2-0-0" />
    <PublicationType value="Full" />
    <PublicationSource value="HSCIC" />
    <PublicationDate value="2026-08-28" />
    <PublicationSeqNum value="2222" />
</Manifest>"#;

    for (path, content) in [(&zip_alpha_path, xml_alpha), (&zip_beta_path, xml_beta)] {
        let zip_file = fs::File::create(path)?;
        let mut zip = zip::ZipWriter::new(zip_file);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("HSCOrgRefData_Full.xml", options)?;
        use std::io::Write;
        zip.write_all(content.as_bytes())?;
        zip.finish()?;
    }

    let iterations = 10;
    let barrier = Arc::new(Barrier::new(2));

    for _ in 0..iterations {
        let b1 = barrier.clone();
        let b2 = barrier.clone();
        let p1 = zip_alpha_path.clone();
        let p2 = zip_beta_path.clone();

        let handle1 = thread::spawn(move || -> Result<()> {
            b1.wait();
            let extracted = ods::commands::ndjson::extract_xml_from_zip(&p1)?;
            let content = fs::read_to_string(&extracted)?;
            assert!(content.contains("1111"), "thread 1 must contain its own sequence number 1111");
            assert!(!content.contains("2222"), "thread 1 must NOT contain thread 2 sequence number 2222");
            Ok(())
        });

        let handle2 = thread::spawn(move || -> Result<()> {
            b2.wait();
            let extracted = ods::commands::ndjson::extract_xml_from_zip(&p2)?;
            let content = fs::read_to_string(&extracted)?;
            assert!(content.contains("2222"), "thread 2 must contain its own sequence number 2222");
            assert!(!content.contains("1111"), "thread 2 must NOT contain thread 1 sequence number 1111");
            Ok(())
        });

        handle1.join().expect("thread 1 panicked")?;
        handle2.join().expect("thread 2 panicked")?;
    }

    Ok(())
}


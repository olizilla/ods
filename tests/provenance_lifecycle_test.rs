mod common;

use anyhow::Result;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_provenance_validate_baseline_missing_fields() -> Result<()> {
    let mut prov = ods::provenance::OdsProvenance::default();
    assert!(prov.validate_baseline().is_err(), "empty provenance must fail baseline validation");

    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_filesize_bytes = Some(37_983_173);
    prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());

    assert!(prov.validate_baseline().is_ok(), "provenance with all baseline fields must pass validation");
    Ok(())
}

#[test]
fn test_provenance_validate_publishable_rejects_implausible_filesize() -> Result<()> {
    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());

    prov.trud_release_filesize_bytes = Some(16); // 16 bytes is implausible for TRUD zip archive
    assert!(prov.validate_baseline().is_ok(), "16-byte filesize must pass structural baseline validation");
    let pub_failures = prov.validate_publishable();
    assert_eq!(pub_failures.len(), 1, "implausible filesize must be reported by validate_publishable");
    assert!(
        pub_failures[0].contains("implausibly small") && pub_failures[0].contains("16 bytes"),
        "validate_publishable must name the filesize problem: {}",
        pub_failures[0]
    );

    Ok(())
}

#[test]
fn test_provenance_validate_publishable_accepts_a_real_sized_archive() -> Result<()> {
    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_filesize_bytes = Some(37_983_173);
    prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());

    assert!(prov.validate_baseline().is_ok());
    assert_eq!(prov.validate_publishable(), Vec::<String>::new(), "how the archive was checked is not provenance");

    Ok(())
}

#[test]
fn test_provenance_validate_baseline_rejects_missing_sha256() -> Result<()> {
    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_filesize_bytes = Some(37_983_173);

    assert!(prov.validate_baseline().is_err(), "missing trud_release_sha256 must be rejected");

    Ok(())
}

#[test]
fn test_make_fails_without_valid_provenance() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let input_dir = temp_dir.path().join("input");
    fs::create_dir_all(&input_dir)?;

    // Write an invalid _provenance.json with missing sha256
    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_date = Some("2026-07-31".to_string());
    fs::write(input_dir.join(ods::provenance::PROVENANCE_FILENAME), serde_json::to_string_pretty(&prov)?)?;

    let zip_path = input_dir.join("hscorgrefdataxml_data_7.0.0_20260731000001.zip");
    let zip_file = fs::File::create(&zip_path)?;
    let mut zip_writer = zip::ZipWriter::new(zip_file);
    let options = zip::write::SimpleFileOptions::default();
    zip_writer.start_file("HSCOrgRefData_Full.xml", options)?;
    zip_writer.finish()?;

    let args = ods::commands::parquet::Args {
        input: Some(input_dir.clone()),
        output: Some(temp_dir.path().join("output")), ..Default::default() };

    let result = ods::commands::parquet::run(args);
    assert!(result.is_err(), "ods make/parquet must fail when _provenance.json is invalid");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("_provenance.json") || err_msg.contains("provenance"),
        "error message must mention _provenance.json, got: {}",
        err_msg
    );

    Ok(())
}

#[test]
fn test_provenance_keys_on_make() -> Result<()> {
    let prov = ods::provenance::OdsProvenance::from_trud_statement(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37_983_173,
    );

    let saved_json = serde_json::to_string_pretty(&prov)?;
    let saved: serde_json::Value = serde_json::from_str(&saved_json)?;
    let keys: Vec<&str> = saved.as_object().unwrap().keys().map(|k| k.as_str()).collect();
    let mut expected = vec![
        "$schema",
        "attribution",
        "license",
        "trud_release_date",
        "trud_release_filesize_bytes",
        "trud_release_sha256",
    ];
    let mut actual = keys.clone();
    actual.sort();
    expected.sort();
    assert_eq!(
        actual, expected,
        "provenance must hold exactly the six archive keys"
    );
    assert!(
        !saved_json.contains("trud_schema_version"),
        "trud_schema_version must NOT be in JSON"
    );
    assert!(
        !saved_json.contains("xml_manifest_created"),
        "xml_manifest_created must NOT be in JSON"
    );
    assert!(
        !saved_json.contains("ods_cmd_version"),
        "ods_cmd_version must NOT be in JSON"
    );
    assert!(
        !saved_json.contains("tool_parquet_version"),
        "tool_parquet_version must NOT be in JSON"
    );
    assert!(
        !saved_json.contains("tool_arrow_version"),
        "tool_arrow_version must NOT be in JSON"
    );
    assert!(
        !saved_json.contains("tool_zstd_level"),
        "tool_zstd_level must NOT be in JSON"
    );
    assert!(
        !saved_json.contains("dataset_version"),
        "dataset_version must NOT be in JSON"
    );

    Ok(())
}

#[test]
fn test_cite_output_formats_and_attribution() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let output_dir = temp_dir.path();

    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());

    let prov_json = serde_json::to_string_pretty(&prov)?;
    fs::write(output_dir.join(ods::provenance::PROVENANCE_FILENAME), prov_json)?;
    fs::write(output_dir.join("orgs.parquet"), b"mock parquet data")?;
    fs::write(output_dir.join("datapackage.json"), b"{\"name\": \"ods\", \"version\": \"0.1.0\"}")?;

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
        if fmt == "text" || fmt == "csljson" {
            assert!(output.contains("2026-07-31"), "cite format {} must contain trud_release_date 2026-07-31", fmt);
        } else {
            assert!(output.contains("2026"), "cite format {} must contain year 2026", fmt);
        }
    }

    Ok(())
}

#[test]
fn test_cite_unverified_release_delivers_citation_and_warns() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let output_dir = temp_dir.path();

    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());

    let prov_json = serde_json::to_string_pretty(&prov)?;
    fs::write(output_dir.join(ods::provenance::PROVENANCE_FILENAME), prov_json)?;
    fs::write(output_dir.join("orgs.parquet"), b"mock parquet data")?;
    fs::write(output_dir.join("datapackage.json"), b"{\"name\": \"ods\", \"version\": \"0.1.0\"}")?;

    let args = ods::commands::cite::Args {
        input: Some(output_dir.to_path_buf()),
        format: "text".to_string(),
    };
    let mut buf = Vec::new();
    let result = ods::commands::cite::run_with_writer(args, &mut buf);
    assert!(result.is_ok(), "ods cite must succeed and deliver citation for unverified release");
    let out = String::from_utf8(buf)?;
    assert!(out.contains("How to Cite"));
    assert!(out.contains("Evans, O."));

    Ok(())
}

#[test]
fn test_make_fails_when_provenance_has_older_terms() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let output_dir = temp_dir.path();
    let trud_dir = output_dir.join("trud");
    fs::create_dir_all(&trud_dir)?;

    let fixture_zip =
        common::create_mock_trud_zip(&trud_dir, "hscorgrefdataxml_data_7.0.0_20260731000001.zip");
    let zip_sha256 = ods::provenance::compute_file_sha256(&fixture_zip)?;
    let zip_size = fs::metadata(&fixture_zip)?.len();

    let mut prov =
        ods::provenance::OdsProvenance::from_trud_statement("2026-07-31", &zip_sha256, zip_size);
    prov.attribution = Some("Old terms".to_string());

    let prov_json = serde_json::to_string_pretty(&prov)?;
    fs::write(output_dir.join(ods::provenance::PROVENANCE_FILENAME), prov_json)?;

    let args = ods::commands::parquet::Args {
        input: Some(output_dir.to_path_buf()),
        output: Some(temp_dir.path().join("out")),
        ..Default::default()
    };
    let result = ods::commands::parquet::run(args);
    assert!(
        result.is_err(),
        "ods make must fail when provenance has older terms"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("older licence terms"),
        "must report older licence terms, got: {}",
        err
    );
    assert!(
        err.contains("Refresh it without downloading the archive"),
        "must suggest refresh, got: {}",
        err
    );

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
    let file = fs::File::open(&xml_path)?;
    let header = ods::ods_xml::parse_manifest_header(quick_xml::Reader::from_reader(std::io::BufReader::new(file)))?;

    let scope = header.primary_role_scope.expect("primary_role_scope must be parsed");
    assert_eq!(scope, vec!["RO180".to_string(), "RO198".to_string()]);

    Ok(())
}

#[test]
fn test_provenance_does_not_have_dataset_version() -> Result<()> {
    let prov = ods::provenance::OdsProvenance::from_trud_statement(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37_983_173,
    );
    let prov_content = serde_json::to_string_pretty(&prov)?;
    assert!(
        !prov_content.contains("dataset_version"),
        "dataset_version must not be in _provenance.json"
    );
    Ok(())
}

#[test]
fn test_make_fails_when_archive_hash_mismatches() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let output_dir = temp_dir.path();

    let archive_file = "hscorgrefdataxml_data_7.0.0_20260731000001.zip";
    let trud_dir = output_dir.join("trud");
    fs::create_dir_all(&trud_dir)?;
    fs::write(trud_dir.join(archive_file), b"corrupted archive bytes")?;

    let prov = ods::provenance::OdsProvenance::from_trud_statement(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37_983_173,
    );
    fs::write(
        output_dir.join(ods::provenance::PROVENANCE_FILENAME),
        serde_json::to_string_pretty(&prov)?,
    )?;

    let args = ods::commands::parquet::Args {
        input: Some(output_dir.to_path_buf()),
        output: Some(temp_dir.path().join("out")),
        ..Default::default()
    };
    let result = ods::commands::parquet::run(args);
    assert!(
        result.is_err(),
        "ods make must fail when trud archive hash mismatches"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("verification mismatch") || err.contains("Pre-build archive verification"),
        "error must mention archive match failure, got: {}",
        err
    );

    Ok(())
}

#[test]
fn test_trud_pull_writes_no_tool_or_dataset_keys() -> Result<()> {
    let prov = ods::provenance::OdsProvenance::from_trud_statement(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
    );

    let prov_json = serde_json::to_string_pretty(&prov)?;
    let raw_val: serde_json::Value = serde_json::from_str(&prov_json)?;
    let obj = raw_val.as_object().expect("provenance must be JSON object");

    // Must ONLY contain $schema, trud_*, license, attribution
    for key in obj.keys() {
        assert!(
            key == "$schema" || key.starts_with("trud_") || key == "license" || key == "attribution",
            "trud pull baseline must NOT write key '{}'. Only $schema, trud_*, license, and attribution allowed.",
            key
        );
    }
    assert!(
        !obj.contains_key("trud_release_url"),
        "trud_release_url must not exist in provenance"
    );
    assert!(
        !obj.contains_key("tool_version"),
        "tool_version must not exist after trud pull"
    );
    assert!(
        !obj.contains_key("tool_git_sha"),
        "tool_git_sha must not exist after trud pull"
    );
    assert!(
        !obj.contains_key("tool_git_dirty"),
        "tool_git_dirty must not exist after trud pull"
    );
    assert!(
        !obj.contains_key("dataset_version"),
        "dataset_version must not exist after trud pull"
    );
    Ok(())
}

#[test]
fn test_release_datapackage_contains_enriched_fields() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let output_dir = temp_dir.path();

    // Create dummy files for resources
    let files = [
        "orgs.parquet",
        "roles.parquet",
        "relationships.parquet",
        "successions.parquet",
    ];
    for f in &files {
        fs::write(output_dir.join(f), format!("data for {}", f))?;
    }

    let release_pkg = ods::datapackage::generate_release_datapackage(
        output_dir,
        Some("10.5281/zenodo.1234567"),
        Some("1.0.1"),
    );

    assert_eq!(release_pkg["id"], "10.5281/zenodo.1234567");
    assert_eq!(release_pkg["version"], "1.0.1");
    let sources = release_pkg["sources"].as_array().expect("sources must be array");
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0]["title"], "NHS Organisation Data Service XML Data, NHS England");
    assert_eq!(sources[0]["path"], "https://isd.digital.nhs.uk/trud");

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
        fs::write(path, common::create_inner_zip("HSCOrgRefData_Full.xml", content.as_bytes()))?;
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
            let extracted = ods::ods_xml::extract_xml_from_zip(&p1)?;
            let content = fs::read_to_string(&extracted[0])?;
            assert!(content.contains("1111"), "thread 1 must contain its own sequence number 1111");
            assert!(!content.contains("2222"), "thread 1 must NOT contain thread 2 sequence number 2222");
            Ok(())
        });

        let handle2 = thread::spawn(move || -> Result<()> {
            b2.wait();
            let extracted = ods::ods_xml::extract_xml_from_zip(&p2)?;
            let content = fs::read_to_string(&extracted[0])?;
            assert!(content.contains("2222"), "thread 2 must contain its own sequence number 2222");
            assert!(!content.contains("1111"), "thread 2 must NOT contain thread 1 sequence number 1111");
            Ok(())
        });

        handle1.join().expect("thread 1 panicked")?;
        handle2.join().expect("thread 2 panicked")?;
    }

    Ok(())
}


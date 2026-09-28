mod common;

use anyhow::Result;
use std::fs;
use tempfile::TempDir;

const SHA: &str = "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933";

fn record() -> ods::provenance::PullRecord {
    ods::provenance::PullRecord::for_trud_release(
        "2026-07-31",
        "hscorgrefdataxml_data_7.0.0_20260731000001.zip",
        SHA,
        37_983_173,
        &[],
    )
    .unwrap()
}

#[test]
fn test_pull_record_validate_baseline() -> Result<()> {
    assert!(record().validate_baseline().is_ok(), "a record from TRUD's word passes");

    let mut no_date = record();
    no_date.version = String::new();
    assert!(no_date.validate_baseline().is_err(), "a record with no release date fails");

    let mut no_archive = record();
    no_archive.resources.clear();
    assert!(no_archive.validate_baseline().is_err(), "a record with no archive fails");

    let mut bad_hash = record();
    bad_hash.resources[0].hash = SHA.to_string();
    assert!(bad_hash.validate_baseline().is_err(), "a hash without its sha256: prefix fails");
    Ok(())
}

#[test]
fn test_make_fails_without_valid_provenance() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let input_dir = temp_dir.path().join("input");
    fs::create_dir_all(&input_dir)?;

    // A pull record whose archive has no hash
    let mut bad = record();
    bad.resources[0].hash = String::new();
    fs::write(input_dir.join(ods::provenance::PULL_RECORD_FILENAME), bad.to_json_string()?)?;

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
    assert!(result.is_err(), "ods make/parquet must fail when the pull record is invalid");
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("pull record"), "error message must name the pull record, got: {}", err_msg);

    Ok(())
}

#[test]
fn test_pull_record_is_the_trud_release_as_a_data_package() -> Result<()> {
    let saved: serde_json::Value = serde_json::from_str(&record().to_json_string()?)?;
    let keys: Vec<&str> = saved.as_object().unwrap().keys().map(|k| k.as_str()).collect();
    assert_eq!(
        keys,
        vec!["$schema", "name", "version", "title", "homepage", "licenses", "contributors", "resources"],
        "the pull record holds Data Package properties, in this order"
    );
    assert_eq!(saved["name"], "nhs-ods-xml");
    assert_eq!(saved["version"], "2026-07-31", "the version is the TRUD release date, never a dataset version");
    let text = saved.to_string();
    for absent in ["tool_version", "tool_git_sha", "dataset_version", "_0.1.0", "trud_release_url"] {
        assert!(!text.contains(absent), "the pull record must not name {absent}: {text}");
    }
    Ok(())
}

/// A release directory with one `orgs.parquet` carrying the provenance of a 2026-07-31 build.
fn cite_fixture(dir: &std::path::Path) {
    common::write_fixture_parquet(&dir.join("orgs.parquet"), "2026-07-31", SHA, "0.1.0", "mock parquet data");
}

#[test]
fn test_cite_output_formats_and_attribution() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let output_dir = temp_dir.path();
    cite_fixture(output_dir);

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
    cite_fixture(output_dir);

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

    let mut record = ods::provenance::PullRecord::for_trud_release(
        "2026-07-31",
        "hscorgrefdataxml_data_7.0.0_20260731000001.zip",
        &zip_sha256,
        zip_size,
        &[],
    )?;
    record.licenses[0].attribution = "Old terms".to_string();
    record.write_to_dir(output_dir)?;

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
fn test_make_fails_when_archive_hash_mismatches() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let output_dir = temp_dir.path();

    let archive_file = "hscorgrefdataxml_data_7.0.0_20260731000001.zip";
    let trud_dir = output_dir.join("trud");
    fs::create_dir_all(&trud_dir)?;
    fs::write(trud_dir.join(archive_file), b"corrupted archive bytes")?;

    record().write_to_dir(output_dir)?;

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
fn test_release_datapackage_contains_enriched_fields() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let output_dir = temp_dir.path();

    for f in ["orgs", "roles", "relationships", "successions"] {
        common::write_fixture_parquet(&output_dir.join(format!("{f}.parquet")), "2026-07-31", SHA, "1.0.1", f);
    }
    let (manifest, _) = ods::commands::make_oci::build_manifest_from_dir(output_dir)?;
    let mut index = common::make_v1_index(&[("2026-07-31", SHA, 37_983_173, &[("1.0.1", &manifest.digest()?)])]);
    index.releases[0].datasets[0].dataset_doi = Some("10.5281/zenodo.1234567".to_string());

    let release_pkg = ods::datapackage::generate_view(output_dir, Some(&index))?;

    assert_eq!(release_pkg["id"], "10.5281/zenodo.1234567");
    assert_eq!(release_pkg["name"], "ods-data");
    assert_eq!(release_pkg["version"], "2026-07-31_1.0.1");
    let sources = release_pkg["sources"].as_array().expect("sources must be array");
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0]["title"], "NHS Organisation Data Service XML Data");
    assert_eq!(sources[0]["path"], ods::terms::LANDING_PAGE);
    assert_eq!(sources[0]["_cache"][0], format!("https://ghcr.io/v2/olizilla/nhs-ods-xml/blobs/sha256:{}", SHA.to_lowercase()));

    let resources = release_pkg["resources"].as_array().expect("resources must be array");
    assert_eq!(resources.len(), 4);
    for res in resources {
        let name = res["name"].as_str().unwrap();
        assert!(res.get("bytes").is_some(), "resource '{}' must have 'bytes'", name);
        assert!(res.get("hash").is_some(), "resource '{}' must have 'hash'", name);
        let hash_str = res["hash"].as_str().unwrap();
        assert!(hash_str.starts_with("sha256:"), "hash format must be sha256:<hex>, got: {}", hash_str);
    }

    // Without a DOI on the row, `id` is the release's oci purl, from the rebuilt manifest.
    let no_doi = ods::datapackage::generate_view(output_dir, None)?;
    assert_eq!(
        no_doi["id"],
        format!(
            "pkg:oci/ods-data@sha256%3A{}?repository_url=ods.fyi%2Fods-data",
            manifest.digest()?.trim_start_matches("sha256:")
        )
    );

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


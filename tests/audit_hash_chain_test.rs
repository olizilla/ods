use anyhow::Result;
use std::fs::{self, File};
use std::io::Write;
use tempfile::TempDir;

fn setup_valid_workspace_with_provenance() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let workspace_root = tmp.path().join("ods_data");
    let rel_dir = workspace_root.join("releases").join("2026-07-31");
    let trud_dir = rel_dir.join("trud");
    fs::create_dir_all(&trud_dir).unwrap();

    let xml_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<un:OrganisationManifest xmlns:un="http://refdata.hscic.gov.uk/org/v2-0-0">
  <un:ManifestHeader>
    <un:PublicationType value="Full" />
    <un:PublicationDate value="2026-07-31" />
    <un:PublicationSeqNum value="4700" />
    <un:PrimaryRoleScope>
      <un:PrimaryRole id="RO177" displayName="Prescribing Cost Centre" />
    </un:PrimaryRoleScope>
  </un:ManifestHeader>
  <un:CodeSystems>
    <un:CodeSystem name="ODS_Role" id="2.16.840.1.113883.2.1.3.2.4.17.507">
      <un:concept id="RO177" displayName="Prescribing Cost Centre" />
    </un:CodeSystem>
  </un:CodeSystems>
  <un:Organisations>
    <un:Organisation>
      <un:Name>PREDECESSOR PRACTICE</un:Name>
      <un:OrgId root="2.16.840.1.113883.2.1.3.2.4.18.48" extension="A200" />
      <un:Status value="Inactive" />
      <un:Date type="Operational"><un:Start value="2010-01-01" /><un:End value="2020-01-01" /></un:Date>
      <un:OrgRecordClass value="RC1" />
      <un:Role id="RO177" uniqueRoleId="2" primaryRole="true" status="Active" />
    </un:Organisation>
    <un:Organisation>
      <un:Name>TEST PRACTICE</un:Name>
      <un:OrgId root="2.16.840.1.113883.2.1.3.2.4.18.48" extension="A100" />
      <un:Status value="Active" />
      <un:Date type="Operational"><un:Start value="2020-01-01" /></un:Date>
      <un:OrgRecordClass value="RC1" />
      <un:Role id="RO177" uniqueRoleId="1" primaryRole="true" status="Active" />
      <un:Succ uniqueSuccId="100">
        <Date><Type value="Operational" /><Start value="2020-01-01" /></Date>
        <Type>Predecessor</Type>
        <Target><OrgId extension="A200" /></Target>
      </un:Succ>
    </un:Organisation>
  </un:Organisations>
</un:OrganisationManifest>"#;

    let inner_zip_bytes = create_inner_zip("HSCOrgRefData_Full_20260731.xml", xml_content);
    let outer_zip_path = trud_dir.join("hscorgrefdataxml_data_7.0.0_20260731000001.zip");

    {
        let outer_file = File::create(&outer_zip_path).unwrap();
        let mut outer_zip = zip::ZipWriter::new(outer_file);
        let options = zip::write::SimpleFileOptions::default();
        outer_zip.start_file("fullfile.zip", options).unwrap();
        outer_zip.write_all(&inner_zip_bytes).unwrap();
        outer_zip.finish().unwrap();
    }

    let zip_sha256 = ods::provenance::compute_file_sha256(&outer_zip_path).unwrap();

    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_name = Some("Release 7.0.0".to_string());
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_file = Some("hscorgrefdataxml_data_7.0.0_20260731000001.zip".to_string());
    prov.trud_release_filesize_bytes = Some(37_983_173);
    prov.trud_release_sha256 = Some(zip_sha256.clone());
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);
    prov.publication_date = Some("2026-07-28".to_string());
    prov.publication_seq_num = Some("4700".to_string());
    prov.publication_type = Some("Full".to_string());
    prov.publication_record_count = Some(2);

    fs::write(
        rel_dir.join(ods::provenance::PROVENANCE_FILENAME),
        serde_json::to_string_pretty(&prov).unwrap(),
    ).unwrap();

    ods::workspace::set_active_release(&workspace_root, "2026-07-31").unwrap();

    // Run parquet compilation
    ods::commands::parquet::run(ods::commands::parquet::Args {
        input: rel_dir.clone(),
        output: rel_dir.clone(),
    }).unwrap();

    ods::provenance::update_provenance_and_write_sha256sums(&rel_dir, None).unwrap();

    (tmp, workspace_root, outer_zip_path)
}

fn create_inner_zip(filename: &str, content: &str) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file(filename, options).unwrap();
        zip.write_all(content.as_bytes()).unwrap();
        zip.finish().unwrap();
    }
    buf
}

#[test]
fn test_audit_passes_on_verified_trud_release() -> Result<()> {
    let (_tmp, workspace_root, zip_path) = setup_valid_workspace_with_provenance();

    let args = ods::commands::audit::Args {
        input: Some(zip_path),
        workspace: Some(workspace_root),
        json: false,
        sample: 50,
        full: true,
        all: false,
    };

    let result = ods::commands::audit::run(args);
    assert!(result.is_ok(), "audit must pass on verified release");
    Ok(())
}

#[test]
fn test_audit_runs_full_suite_and_fails_on_unverified_local_archive() -> Result<()> {
    let (_tmp, workspace_root, zip_path) = setup_valid_workspace_with_provenance();
    let (date, active_dir) = ods::workspace::get_active_release(&workspace_root)?;
    assert_eq!(date, "2026-07-31");

    // Set trud_release_sha256_verified to false
    let prov_path = active_dir.join(ods::provenance::PROVENANCE_FILENAME);
    let mut prov: ods::provenance::OdsProvenance = serde_json::from_str(&fs::read_to_string(&prov_path)?)?;
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::Unverified);
    fs::write(&prov_path, serde_json::to_string_pretty(&prov)?)?;

    let args = ods::commands::audit::Args {
        input: Some(zip_path),
        workspace: Some(workspace_root),
        json: false,
        sample: 50,
        full: true,
        all: false,
    };

    let result = ods::commands::audit::run(args);
    assert!(result.is_err(), "audit must fail when archive is unverified");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("unverified") || err_msg.contains("discrepanc"),
        "error message must describe unverified archive, got: {}",
        err_msg
    );
    Ok(())
}

#[test]
fn test_audit_runs_full_suite_and_fails_on_corrupted_parquet_file() -> Result<()> {
    let (_tmp, workspace_root, zip_path) = setup_valid_workspace_with_provenance();
    let (_, active_dir) = ods::workspace::get_active_release(&workspace_root)?;

    // Mutate 1 byte of orgs.parquet
    let orgs_path = active_dir.join("orgs.parquet");
    let mut bytes = fs::read(&orgs_path)?;
    bytes[0] ^= 0xFF;
    fs::write(&orgs_path, bytes)?;

    let args = ods::commands::audit::Args {
        input: Some(zip_path),
        workspace: Some(workspace_root),
        json: false,
        sample: 50,
        full: true,
        all: false,
    };

    let result = ods::commands::audit::run(args);
    assert!(result.is_err(), "audit must fail on corrupted parquet file");
    Ok(())
}

#[test]
fn test_audit_runs_full_suite_and_fails_on_tampered_sha256sums_file() -> Result<()> {
    let (_tmp, workspace_root, zip_path) = setup_valid_workspace_with_provenance();
    let (_, active_dir) = ods::workspace::get_active_release(&workspace_root)?;

    // Mutate SHA256SUMS file
    let sums_path = active_dir.join("SHA256SUMS");
    let content = fs::read_to_string(&sums_path)?;
    let tampered = content.replace('0', "1");
    fs::write(&sums_path, tampered)?;

    let args = ods::commands::audit::Args {
        input: Some(zip_path),
        workspace: Some(workspace_root),
        json: false,
        sample: 50,
        full: true,
        all: false,
    };

    let result = ods::commands::audit::run(args);
    assert!(result.is_err(), "audit must fail on tampered SHA256SUMS file");
    Ok(())
}

#[test]
fn test_audit_runs_full_suite_and_fails_on_corrupted_provenance_derived_artifacts() -> Result<()> {
    let (_tmp, workspace_root, zip_path) = setup_valid_workspace_with_provenance();
    let (_, active_dir) = ods::workspace::get_active_release(&workspace_root)?;

    // Mutate dataset_file_sha256 in _provenance.json
    let prov_path = active_dir.join(ods::provenance::PROVENANCE_FILENAME);
    let mut prov: ods::provenance::OdsProvenance = serde_json::from_str(&fs::read_to_string(&prov_path)?)?;
    if let Some(ref mut map) = prov.dataset_file_sha256 {
        map.insert("orgs.parquet".to_string(), "0000000000000000000000000000000000000000000000000000000000000000".to_string());
    }
    fs::write(&prov_path, serde_json::to_string_pretty(&prov)?)?;

    let args = ods::commands::audit::Args {
        input: Some(zip_path),
        workspace: Some(workspace_root),
        json: false,
        sample: 50,
        full: true,
        all: false,
    };

    let result = ods::commands::audit::run(args);
    assert!(result.is_err(), "audit must fail on corrupted dataset_file_sha256 in _provenance.json");
    Ok(())
}

#[test]
fn test_audit_fails_on_unaccounted_file_in_release_directory() -> Result<()> {
    let (_tmp, workspace_root, zip_path) = setup_valid_workspace_with_provenance();
    let (_, active_dir) = ods::workspace::get_active_release(&workspace_root)?;

    // Create stray unaccounted file in active release directory
    let stray_path = active_dir.join("rels.parquet");
    fs::write(&stray_path, b"stray content")?;

    let args = ods::commands::audit::Args {
        input: Some(zip_path),
        workspace: Some(workspace_root),
        json: false,
        sample: 50,
        full: true,
        all: false,
    };

    let result = ods::commands::audit::run(args);
    assert!(result.is_err(), "audit must fail when unaccounted files exist in release directory");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Unaccounted") || err_msg.contains("rels.parquet") || err_msg.contains("discrepanc"),
        "error message must mention unaccounted file, got: {}",
        err_msg
    );
    Ok(())
}

#[test]
fn test_audit_fails_on_successions_count_mismatch() -> Result<()> {
    let (_tmp, workspace_root, zip_path) = setup_valid_workspace_with_provenance();
    let (_, active_dir) = ods::workspace::get_active_release(&workspace_root)?;

    // Overwrite successions.parquet with empty/different file
    let empty_records: Vec<ods::commands::ndjson::OdsRecord> = Vec::new();
    let prov = ods::provenance::OdsProvenance::load_from_dir(&active_dir);
    ods::commands::parquet::export_successions(&active_dir, &empty_records, prov.as_ref())?;
    ods::provenance::update_provenance_and_write_sha256sums(&active_dir, None)?;

    let args = ods::commands::audit::Args {
        input: Some(zip_path),
        workspace: Some(workspace_root),
        json: false,
        sample: 50,
        full: true,
        all: false,
    };

    let result = ods::commands::audit::run(args);
    assert!(result.is_err(), "audit must fail when XML successions count does not match successions.parquet");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Succession") || err_msg.contains("discrepanc"),
        "error message must describe successions mismatch, got: {}",
        err_msg
    );
    Ok(())
}

#[test]
fn test_audit_fails_on_orphan_successions() -> Result<()> {
    let (_tmp, workspace_root, zip_path) = setup_valid_workspace_with_provenance();
    let (_, active_dir) = ods::workspace::get_active_release(&workspace_root)?;

    // Create an orphan succession edge
    let record_with_orphan = ods::commands::ndjson::OdsRecord {
        ods_code: "A100".to_string(),
        name: "TEST".to_string(),
        status: "active".to_string(),
        successors: vec![ods::commands::ndjson::OdsSuccessor {
            unique_succ_id: "999".to_string(),
            succ_type: "Predecessor".to_string(),
            dates: Vec::new(),
            target: ods::commands::ndjson::OdsRelationshipTarget {
                ods_code: "NONEXISTENT_ORG_999".to_string(),
                ..Default::default()
            },
        }],
        ..Default::default()
    };
    let prov = ods::provenance::OdsProvenance::load_from_dir(&active_dir);
    ods::commands::parquet::export_successions(&active_dir, &[record_with_orphan], prov.as_ref())?;
    ods::provenance::update_provenance_and_write_sha256sums(&active_dir, None)?;

    let args = ods::commands::audit::Args {
        input: Some(zip_path),
        workspace: Some(workspace_root),
        json: false,
        sample: 50,
        full: true,
        all: false,
    };

    let result = ods::commands::audit::run(args);
    assert!(result.is_err(), "audit must fail when successions reference non-existent organisation codes");
    Ok(())
}

#[test]
fn test_unexpected_files_detection() -> Result<()> {
    let (_tmp, workspace_root, _zip_path) = setup_valid_workspace_with_provenance();
    let (_, active_dir) = ods::workspace::get_active_release(&workspace_root)?;

    assert!(ods::commands::parquet::get_unexpected_files(&active_dir).is_empty());

    let stray1 = active_dir.join("rels.parquet");
    let stray2 = active_dir.join("old_rules.json");
    let ignored1 = active_dir.join(".DS_Store");
    let ignored2 = active_dir.join("markdown");
    fs::write(&stray1, b"stray1")?;
    fs::write(&stray2, b"stray2")?;
    fs::write(&ignored1, b"ds_store")?;
    fs::create_dir_all(&ignored2)?;

    let unexpected = ods::commands::parquet::get_unexpected_files(&active_dir);
    assert_eq!(unexpected, vec!["old_rules.json".to_string(), "rels.parquet".to_string()]);
    Ok(())
}

#[test]
fn test_audit_fails_on_corrupted_transitive_closure() -> Result<()> {
    let (_tmp, workspace_root, zip_path) = setup_valid_workspace_with_provenance();
    let (_, active_dir) = ods::workspace::get_active_release(&workspace_root)?;

    // Corrupt the transitive closure by re-exporting orgs with empty closures
    let record = ods::commands::ndjson::OdsRecord {
        ods_code: "A100".to_string(),
        name: "TEST PRACTICE".to_string(),
        status: "active".to_string(),
        record_class: "org".to_string(),
        roles: vec![ods::commands::ndjson::OdsRole {
            id: "RO177".to_string(),
            code: None,
            display_name: Some("Prescribing Cost Centre".to_string()),
            unique_role_id: "1".to_string(),
            primary_role: true,
            status: "active".to_string(),
            dates: vec![],
        }],
        ..Default::default()
    };

    let empty_closures = std::collections::HashMap::new();
    let prov = ods::provenance::OdsProvenance::load_from_dir(&active_dir);
    // Export with empty closures when XML has a predecessor edge
    ods::commands::parquet::export_orgs(&active_dir, &[record.clone()], &empty_closures, &empty_closures, prov.as_ref())?;
    ods::commands::parquet::export_orgs_all(&active_dir, &[record], &empty_closures, &empty_closures, prov.as_ref())?;
    ods::provenance::update_provenance_and_write_sha256sums(&active_dir, None)?;

    let args = ods::commands::audit::Args {
        input: Some(zip_path),
        workspace: Some(workspace_root),
        json: false,
        sample: 50,
        full: true,
        all: false,
    };

    let result = ods::commands::audit::run(args);
    assert!(result.is_err(), "audit must fail when successor/predecessor closure does not match ground truth");
    Ok(())
}

#[test]
fn test_audit_fails_on_source_invariant_violation() -> Result<()> {
    let tmp = TempDir::new().unwrap();
    let workspace_root = tmp.path().join("ods_data");
    let rel_dir = workspace_root.join("releases").join("2026-07-31");
    let trud_dir = rel_dir.join("trud");
    fs::create_dir_all(&trud_dir).unwrap();

    // XML with duplicate uniqueRoleId="1" across two orgs
    let xml_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<un:OrganisationManifest xmlns:un="http://refdata.hscic.gov.uk/org/v2-0-0">
  <un:ManifestHeader>
    <un:PublicationType value="Full" />
    <un:PublicationDate value="2026-07-31" />
    <un:PublicationSeqNum value="4700" />
    <un:PrimaryRoleScope>
      <un:PrimaryRole id="RO177" displayName="Prescribing Cost Centre" />
    </un:PrimaryRoleScope>
  </un:ManifestHeader>
  <un:CodeSystems>
    <un:CodeSystem name="ODS_Role" id="2.16.840.1.113883.2.1.3.2.4.17.507">
      <un:concept id="RO177" displayName="Prescribing Cost Centre" />
    </un:CodeSystem>
  </un:CodeSystems>
  <un:Organisations>
    <un:Organisation>
      <un:Name>ORG A</un:Name>
      <un:OrgId root="2.16.840.1.113883.2.1.3.2.4.18.48" extension="A100" />
      <un:Status value="Active" />
      <un:Date type="Operational"><un:Start value="2020-01-01" /></un:Date>
      <un:Role id="RO177" uniqueRoleId="1" primaryRole="true" status="Active" />
    </un:Organisation>
    <un:Organisation>
      <un:Name>ORG B</un:Name>
      <un:OrgId root="2.16.840.1.113883.2.1.3.2.4.18.48" extension="A200" />
      <un:Status value="Active" />
      <un:Date type="Operational"><un:Start value="2020-01-01" /></un:Date>
      <un:Role id="RO177" uniqueRoleId="1" primaryRole="true" status="Active" />
    </un:Organisation>
  </un:Organisations>
</un:OrganisationManifest>"#;

    let inner_zip_bytes = create_inner_zip("HSCOrgRefData_Full_20260731.xml", xml_content);
    let outer_zip_path = trud_dir.join("hscorgrefdataxml_data_7.0.0_20260731000001.zip");

    {
        let outer_file = File::create(&outer_zip_path).unwrap();
        let mut outer_zip = zip::ZipWriter::new(outer_file);
        let options = zip::write::SimpleFileOptions::default();
        outer_zip.start_file("fullfile.zip", options).unwrap();
        outer_zip.write_all(&inner_zip_bytes).unwrap();
        outer_zip.finish().unwrap();
    }

    let zip_sha256 = ods::provenance::compute_file_sha256(&outer_zip_path).unwrap();
    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_sha256 = Some(zip_sha256);
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);
    fs::write(
        rel_dir.join(ods::provenance::PROVENANCE_FILENAME),
        serde_json::to_string_pretty(&prov).unwrap(),
    ).unwrap();

    ods::workspace::set_active_release(&workspace_root, "2026-07-31").unwrap();
    ods::commands::parquet::run(ods::commands::parquet::Args {
        input: rel_dir.clone(),
        output: rel_dir.clone(),
    }).unwrap();
    ods::provenance::update_provenance_and_write_sha256sums(&rel_dir, None).unwrap();

    let args = ods::commands::audit::Args {
        input: Some(outer_zip_path),
        workspace: Some(workspace_root),
        json: false,
        sample: 50,
        full: true,
        all: false,
    };

    let result = ods::commands::audit::run(args);
    assert!(result.is_err(), "audit must fail when source structural invariant is violated");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("duplicate uniqueRoleId") || err_msg.contains("discrepanc"),
        "error message must describe invariant violation, got: {}",
        err_msg
    );
    Ok(())
}

#[test]
fn test_audit_all_skips_unmade_releases() -> Result<()> {
    let (_tmp, workspace_root, _zip_path) = setup_valid_workspace_with_provenance();

    // Create a second release that has trud/ and _provenance.json but NO derived parquet files
    let unmade_dir = workspace_root.join("releases").join("2020-01-01");
    fs::create_dir_all(unmade_dir.join("trud")).unwrap();
    let prov = ods::provenance::OdsProvenance {
        trud_release_date: Some("2020-01-01".to_string()),
        ..Default::default()
    };
    fs::write(
        unmade_dir.join(ods::provenance::PROVENANCE_FILENAME),
        serde_json::to_string_pretty(&prov).unwrap(),
    ).unwrap();

    let args = ods::commands::audit::Args {
        input: None,
        workspace: Some(workspace_root),
        json: false,
        sample: 50,
        full: true,
        all: true,
    };

    let result = ods::commands::audit::run(args);
    assert!(result.is_ok(), "audit --all must succeed by skipping unmade release 2020-01-01 when at least one release is audited");
    Ok(())
}

#[test]
fn test_audit_all_fails_when_all_releases_skipped() -> Result<()> {
    let tmp = TempDir::new().unwrap();
    let workspace_root = tmp.path().join("ods_data");

    // Create 2 releases that both have trud/ and _provenance.json but NO derived parquet files
    for date in &["2020-01-01", "2020-02-01"] {
        let unmade_dir = workspace_root.join("releases").join(date);
        fs::create_dir_all(unmade_dir.join("trud")).unwrap();
        let prov = ods::provenance::OdsProvenance {
            trud_release_date: Some(date.to_string()),
            ..Default::default()
        };
        fs::write(
            unmade_dir.join(ods::provenance::PROVENANCE_FILENAME),
            serde_json::to_string_pretty(&prov).unwrap(),
        ).unwrap();
    }

    let args = ods::commands::audit::Args {
        input: None,
        workspace: Some(workspace_root),
        json: false,
        sample: 50,
        full: true,
        all: true,
    };

    let result = ods::commands::audit::run(args);
    assert!(result.is_err(), "audit --all must fail with non-zero exit when 100% of releases are skipped");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("nothing to audit"),
        "error message must describe 'nothing to audit', got: {}",
        err_msg
    );
    Ok(())
}

#[test]
fn test_audit_fails_on_dangling_relationship_target_invariant() -> Result<()> {
    let tmp = TempDir::new().unwrap();
    let workspace_root = tmp.path().join("ods_data");
    let rel_dir = workspace_root.join("releases").join("2026-07-31");
    let trud_dir = rel_dir.join("trud");
    fs::create_dir_all(&trud_dir).unwrap();

    // XML with relationship target pointing to non-existent org "NONEXISTENT_ORG"
    let xml_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<un:OrganisationManifest xmlns:un="http://refdata.hscic.gov.uk/org/v2-0-0">
  <un:ManifestHeader>
    <un:PublicationType value="Full" />
    <un:PublicationDate value="2026-07-31" />
    <un:PublicationSeqNum value="4700" />
    <un:PrimaryRoleScope>
      <un:PrimaryRole id="RO177" displayName="Prescribing Cost Centre" />
    </un:PrimaryRoleScope>
  </un:ManifestHeader>
  <un:CodeSystems>
    <un:CodeSystem name="ODS_Role" id="2.16.840.1.113883.2.1.3.2.4.17.507">
      <un:concept id="RO177" displayName="Prescribing Cost Centre" />
    </un:CodeSystem>
  </un:CodeSystems>
  <un:Organisations>
    <un:Organisation>
      <un:Name>ORG A</un:Name>
      <un:OrgId root="2.16.840.1.113883.2.1.3.2.4.18.48" extension="A100" />
      <un:Status value="Active" />
      <un:Date type="Operational"><un:Start value="2020-01-01" /></un:Date>
      <un:Role id="RO177" uniqueRoleId="1" primaryRole="true" status="Active" />
      <un:Rel id="RE1" uniqueRelId="100">
        <Date><Type value="Operational" /><Start value="2020-01-01" /></Date>
        <Target><OrgId extension="NONEXISTENT_ORG" /></Target>
      </un:Rel>
    </un:Organisation>
  </un:Organisations>
</un:OrganisationManifest>"#;

    let inner_zip_bytes = create_inner_zip("HSCOrgRefData_Full_20260731.xml", xml_content);
    let outer_zip_path = trud_dir.join("hscorgrefdataxml_data_7.0.0_20260731000001.zip");

    {
        let outer_file = File::create(&outer_zip_path).unwrap();
        let mut outer_zip = zip::ZipWriter::new(outer_file);
        let options = zip::write::SimpleFileOptions::default();
        outer_zip.start_file("fullfile.zip", options).unwrap();
        outer_zip.write_all(&inner_zip_bytes).unwrap();
        outer_zip.finish().unwrap();
    }

    let zip_sha256 = ods::provenance::compute_file_sha256(&outer_zip_path).unwrap();
    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_sha256 = Some(zip_sha256);
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);
    fs::write(
        rel_dir.join(ods::provenance::PROVENANCE_FILENAME),
        serde_json::to_string_pretty(&prov).unwrap(),
    ).unwrap();

    ods::workspace::set_active_release(&workspace_root, "2026-07-31").unwrap();
    ods::commands::parquet::run(ods::commands::parquet::Args {
        input: rel_dir.clone(),
        output: rel_dir.clone(),
    }).unwrap();
    ods::provenance::update_provenance_and_write_sha256sums(&rel_dir, None).unwrap();

    let args = ods::commands::audit::Args {
        input: Some(outer_zip_path),
        workspace: Some(workspace_root),
        json: false,
        sample: 50,
        full: true,
        all: false,
    };

    let result = ods::commands::audit::run(args);
    assert!(result.is_err(), "audit must fail when relationship target is dangling in source XML");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("dangling") || err_msg.contains("discrepanc"),
        "error message must describe dangling target, got: {}",
        err_msg
    );
    Ok(())
}


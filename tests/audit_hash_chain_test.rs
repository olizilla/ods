mod common;
use anyhow::Result;
use arrow::array::RecordBatchReader;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::ArrowWriter;
use std::fs::{self, File};
use std::io::Write;
use tempfile::TempDir;

static CACHED_WORKSPACE: std::sync::OnceLock<(TempDir, std::path::PathBuf, std::path::PathBuf)> =
    std::sync::OnceLock::new();

fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = dst.join(entry.file_name());
        if file_type.is_symlink() {
            let link_target = fs::read_link(entry.path())?;
            #[cfg(unix)]
            std::os::unix::fs::symlink(&link_target, &target)?;
            #[cfg(windows)]
            if entry.path().is_dir() {
                std::os::windows::fs::symlink_dir(&link_target, &target)?;
            } else {
                std::os::windows::fs::symlink_file(&link_target, &target)?;
            }
        } else if file_type.is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn setup_valid_workspace_impl() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
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
      <un:Rel id="RE4" uniqueRelId="1001" status="Active">
        <Date><Type value="Operational" /><Start value="2020-01-01" /></Date>
        <Target><OrgId extension="A200" /></Target>
      </un:Rel>
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

    common::create_nested_trud_zip(&outer_zip_path, &[("fullfile.zip", &inner_zip_bytes)]);

    let zip_sha256 = ods::provenance::compute_file_sha256(&outer_zip_path).unwrap();

    ods::provenance::write_pull_record(
        &rel_dir,
        "2026-07-31",
        "hscorgrefdataxml_data_7.0.0_20260731000001.zip",
        &zip_sha256,
        fs::metadata(&outer_zip_path).unwrap().len(),
        &[],
    )
    .unwrap();

    ods::workspace::Workspace::open_or_create(Some(&workspace_root)).unwrap().set_active("2026-07-31").unwrap();

    // Run parquet compilation
    ods::commands::parquet::run(ods::commands::parquet::Args {
        input: Some(rel_dir.clone()),
        output: Some(rel_dir.clone()), ..Default::default() })
    .unwrap();

    // Record the build in the workspace's release index, as a published release is, so audit
    // has a genuine baseline to verify the files against: the manifest rebuilt from them must
    // have the digest the index row names.
    let (manifest, _) = ods::commands::make_oci::build_manifest_from_dir(&rel_dir).unwrap();
    let index = common::make_v1_index(&[(
        "2026-07-31",
        &zip_sha256,
        fs::metadata(&outer_zip_path).unwrap().len(),
        &[(ods::datapackage::DATASET_VERSION, &manifest.digest().unwrap())],
    )]);
    ods::index::OdsReleaseIndex::save_to_workspace_bytes(&serde_json::to_vec_pretty(&index).unwrap(), &workspace_root)
        .unwrap();

    (tmp, workspace_root, outer_zip_path)
}

fn setup_valid_workspace_with_provenance() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
    let (_cached_tmp, cached_ws, _cached_zip) = CACHED_WORKSPACE.get_or_init(setup_valid_workspace_impl);
    let tmp = TempDir::new().unwrap();
    let workspace_root = tmp.path().join("ods_data");
    copy_dir_all(cached_ws, &workspace_root).unwrap();
    let outer_zip_path = workspace_root
        .join("releases")
        .join("2026-07-31")
        .join("trud")
        .join("hscorgrefdataxml_data_7.0.0_20260731000001.zip");
    (tmp, workspace_root, outer_zip_path)
}

/// Writer properties carrying the `datapackage` object `rel_dir`'s files carry now.
fn same_provenance(rel_dir: &std::path::Path) -> parquet::file::properties::WriterProperties {
    let value = ods::provenance::read_embedded_value(&rel_dir.join("roles.parquet")).unwrap();
    parquet::file::properties::WriterProperties::builder()
        .set_key_value_metadata(Some(vec![parquet::file::metadata::KeyValue {
            key: ods::provenance::DATAPACKAGE_KEY.to_string(),
            value,
        }]))
        .build()
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
fn test_audit_passes_on_a_release_whose_provenance_matches_the_archive() -> Result<()> {
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
    assert!(result.is_ok(), "audit must pass when the release matches its archive");
    Ok(())
}

#[test]
fn test_audit_runs_full_suite_and_fails_on_corrupted_parquet_file() -> Result<()> {
    let (_tmp, workspace_root, zip_path) = setup_valid_workspace_with_provenance();
    let (_, active_dir) = ods::workspace::Workspace::open(Some(&workspace_root))?.active_release()?;

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
fn test_audit_runs_full_suite_and_fails_on_corrupted_provenance_archive_hash() -> Result<()> {
    let (_tmp, workspace_root, zip_path) = setup_valid_workspace_with_provenance();
    let (_, active_dir) = ods::workspace::Workspace::open(Some(&workspace_root))?.active_release()?;

    // Mutate the archive's hash in the pull record, trud/datapackage.json
    let prov_path = ods::provenance::pull_record_path(&active_dir);
    let mut prov: ods::provenance::PullRecord = serde_json::from_str(&fs::read_to_string(&prov_path)?)?;
    prov.resources[0].hash = format!("sha256:{}", "0".repeat(64));
    fs::write(&prov_path, prov.to_json_string()?)?;

    let args = ods::commands::audit::Args {
        input: Some(zip_path),
        workspace: Some(workspace_root),
        json: false,
        sample: 50,
        full: true,
        all: false,
    };

    let result = ods::commands::audit::run(args);
    assert!(
        result.is_err(),
        "audit must fail on a corrupted archive hash in trud/datapackage.json"
    );
    Ok(())
}

#[test]
fn test_audit_fails_on_unaccounted_file_in_release_directory() -> Result<()> {
    let (_tmp, workspace_root, zip_path) = setup_valid_workspace_with_provenance();
    let (_, active_dir) = ods::workspace::Workspace::open(Some(&workspace_root))?.active_release()?;

    assert!(ods::commands::parquet::get_unexpected_files(&active_dir).is_empty());

    // Create stray unaccounted file in active release directory
    let stray_path = active_dir.join("rels.parquet");
    fs::write(&stray_path, b"stray content")?;

    let unexpected = ods::commands::parquet::get_unexpected_files(&active_dir);
    assert_eq!(unexpected, vec!["rels.parquet".to_string()]);

    let args = ods::commands::audit::Args {
        input: Some(zip_path),
        workspace: Some(workspace_root),
        json: false,
        sample: 50,
        full: true,
        all: false,
    };

    let result = ods::commands::audit::run(args);
    assert!(
        result.is_err(),
        "audit must fail when unaccounted files exist in release directory"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Unaccounted")
            || err_msg.contains("rels.parquet")
            || err_msg.contains("discrepanc"),
        "error message must mention unaccounted file, got: {}",
        err_msg
    );
    Ok(())
}

#[test]
fn test_audit_fails_on_successions_count_mismatch() -> Result<()> {
    let (_tmp, workspace_root, zip_path) = setup_valid_workspace_with_provenance();
    let (_, active_dir) = ods::workspace::Workspace::open(Some(&workspace_root))?.active_release()?;

    // Overwrite successions.parquet with empty/different file
    let empty_records: Vec<ods::ods_xml::OdsRecord> = Vec::new();
    let release_record = ods::provenance::read_release(&active_dir)?;
    let prov = release_record.facts().map(|f| f.embedded.clone());
    ods::commands::parquet::export_successions(&active_dir, &empty_records, prov.as_ref())?;

    let args = ods::commands::audit::Args {
        input: Some(zip_path),
        workspace: Some(workspace_root),
        json: false,
        sample: 50,
        full: true,
        all: false,
    };

    let result = ods::commands::audit::run(args);
    assert!(
        result.is_err(),
        "audit must fail when XML successions count does not match successions.parquet"
    );
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
    let (_, active_dir) = ods::workspace::Workspace::open(Some(&workspace_root))?.active_release()?;

    // Create an orphan succession edge
    let record_with_orphan = ods::ods_xml::OdsRecord {
        ods_code: "A100".to_string(),
        name: "TEST".to_string(),
        status: "active".to_string(),
        successors: vec![ods::ods_xml::OdsSuccessor {
            unique_succ_id: "999".to_string(),
            succ_type: "Predecessor".to_string(),
            dates: Vec::new(),
            target: ods::ods_xml::OdsRelationshipTarget {
                ods_code: "NONEXISTENT_ORG_999".to_string(),
                ..Default::default()
            },
        }],
        ..Default::default()
    };
    let release_record = ods::provenance::read_release(&active_dir)?;
    let prov = release_record.facts().map(|f| f.embedded.clone());
    ods::commands::parquet::export_successions(&active_dir, &[record_with_orphan], prov.as_ref())?;

    let args = ods::commands::audit::Args {
        input: Some(zip_path),
        workspace: Some(workspace_root),
        json: false,
        sample: 50,
        full: true,
        all: false,
    };

    let result = ods::commands::audit::run(args);
    assert!(
        result.is_err(),
        "audit must fail when successions reference non-existent organisation codes"
    );
    Ok(())
}


#[test]
fn test_audit_fails_on_corrupted_transitive_closure() -> Result<()> {
    let (_tmp, workspace_root, zip_path) = setup_valid_workspace_with_provenance();
    let (_, active_dir) = ods::workspace::Workspace::open(Some(&workspace_root))?.active_release()?;

    // Corrupt the transitive closure by re-exporting orgs with empty closures
    let record = ods::ods_xml::OdsRecord {
        ods_code: "A100".to_string(),
        name: "TEST PRACTICE".to_string(),
        status: "active".to_string(),
        record_class: "org".to_string(),
        roles: vec![ods::ods_xml::OdsRole {
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
    let release_record = ods::provenance::read_release(&active_dir)?;
    let prov = release_record.facts().map(|f| f.embedded.clone());
    // Export with empty closures when XML has a predecessor edge
    ods::commands::parquet::export_orgs(
        &active_dir,
        std::slice::from_ref(&record),
        &empty_closures,
        &empty_closures,
        prov.as_ref(),
    )?;

    let args = ods::commands::audit::Args {
        input: Some(zip_path),
        workspace: Some(workspace_root),
        json: false,
        sample: 50,
        full: true,
        all: false,
    };

    let result = ods::commands::audit::run(args);
    assert!(
        result.is_err(),
        "audit must fail when successor/predecessor closure does not match ground truth"
    );
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

    common::create_nested_trud_zip(&outer_zip_path, &[("fullfile.zip", &inner_zip_bytes)]);

    ods::provenance::write_pull_record(
        &rel_dir,
        "2026-07-31",
        "hscorgrefdataxml_data_7.0.0_20260731000001.zip",
        &ods::provenance::compute_file_sha256(&outer_zip_path).unwrap(),
        fs::metadata(&outer_zip_path).unwrap().len(),
        &[],
    )
    .unwrap();

    ods::workspace::Workspace::open_or_create(Some(&workspace_root)).unwrap().set_active("2026-07-31").unwrap();
    ods::commands::parquet::run(ods::commands::parquet::Args {
        input: Some(rel_dir.clone()),
        output: Some(rel_dir.clone()), ..Default::default() })
    .unwrap();

    let args = ods::commands::audit::Args {
        input: Some(outer_zip_path),
        workspace: Some(workspace_root),
        json: false,
        sample: 50,
        full: true,
        all: false,
    };

    let result = ods::commands::audit::run(args);
    assert!(
        result.is_err(),
        "audit must fail when source structural invariant is violated"
    );
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

    // Create a second release that has trud/ and its pull record but NO derived parquet files
    let unmade_dir = workspace_root.join("releases").join("2020-01-01");
    fs::create_dir_all(unmade_dir.join("trud")).unwrap();
    ods::provenance::write_pull_record(
        &unmade_dir,
        "2020-01-01",
        "hscorgrefdataxml_data_7.0.0_20260731000001.zip",
        "0000000000000000000000000000000000000000000000000000000000000000",
        0,
        &[],
    )
    .unwrap();

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
    ods::workspace::ensure_workspace_root(&workspace_root).unwrap();

    // Create 2 releases that both have trud/ and a pull record but NO derived parquet files
    for date in &["2020-01-01", "2020-02-01"] {
        let unmade_dir = workspace_root.join("releases").join(date);
        fs::create_dir_all(unmade_dir.join("trud")).unwrap();
        ods::provenance::write_pull_record(
            &unmade_dir,
            date,
            "hscorgrefdataxml_data_7.0.0_20260731000001.zip",
            "0000000000000000000000000000000000000000000000000000000000000000",
            0,
            &[],
        )
        .unwrap();
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
    assert!(
        result.is_err(),
        "audit --all must fail with non-zero exit when 100% of releases are skipped"
    );
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

    common::create_nested_trud_zip(&outer_zip_path, &[("fullfile.zip", &inner_zip_bytes)]);

    ods::provenance::write_pull_record(
        &rel_dir,
        "2026-07-31",
        "hscorgrefdataxml_data_7.0.0_20260731000001.zip",
        &ods::provenance::compute_file_sha256(&outer_zip_path).unwrap(),
        fs::metadata(&outer_zip_path).unwrap().len(),
        &[],
    )
    .unwrap();

    ods::workspace::Workspace::open_or_create(Some(&workspace_root)).unwrap().set_active("2026-07-31").unwrap();
    ods::commands::parquet::run(ods::commands::parquet::Args {
        input: Some(rel_dir.clone()),
        output: Some(rel_dir.clone()), ..Default::default() })
    .unwrap();

    let args = ods::commands::audit::Args {
        input: Some(outer_zip_path),
        workspace: Some(workspace_root),
        json: false,
        sample: 50,
        full: true,
        all: false,
    };

    let result = ods::commands::audit::run(args);
    assert!(
        result.is_err(),
        "audit must fail when relationship target is dangling in source XML"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("dangling") || err_msg.contains("discrepanc"),
        "error message must describe dangling target, got: {}",
        err_msg
    );
    Ok(())
}

#[test]
fn test_audit_fails_on_inactive_row_in_orgs_parquet() -> Result<()> {
    let (_tmp, workspace_root, outer_zip_path) = setup_valid_workspace_with_provenance();
    let rel_dir = workspace_root.join("releases").join("2026-07-31");
    let orgs_file = rel_dir.join("orgs.parquet");

    // Read existing orgs.parquet
    let file = File::open(&orgs_file)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let reader = builder.build()?;
    let schema = reader.schema();
    let mut batches = Vec::new();
    for b in reader {
        let b = b?;
        let mut cols = b.columns().to_vec();
        let status_idx = schema.index_of("status")?;
        let inactive_arr: std::sync::Arc<dyn arrow::array::Array> =
            std::sync::Arc::new(arrow::array::StringArray::from(vec![
                "Inactive";
                b.num_rows()
            ]));
        cols[status_idx] = inactive_arr;
        let modified_batch = arrow::record_batch::RecordBatch::try_new(schema.clone(), cols)?;
        batches.push(modified_batch);
    }

    // Write back modified orgs.parquet
    {
        let out_file = File::create(&orgs_file)?;
        // The rewritten table keeps the provenance the build embedded: only its rows change.
        let mut writer = ArrowWriter::try_new(out_file, schema, Some(same_provenance(&rel_dir)))?;
        for b in &batches {
            writer.write(b)?;
        }
        writer.close()?;
    }

    let args = ods::commands::audit::Args {
        input: Some(outer_zip_path),
        workspace: Some(workspace_root),
        json: false,
        sample: 50,
        full: true,
        all: false,
    };

    let result = ods::commands::audit::run(args);
    assert!(
        result.is_err(),
        "audit must fail when orgs.parquet contains inactive rows"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("discrepanc"),
        "error message must report discrepancies, got: {}",
        err_msg
    );
    Ok(())
}

#[test]
fn test_audit_fails_on_mismatched_role_codes_and_names() -> Result<()> {
    let (_tmp, workspace_root, outer_zip_path) = setup_valid_workspace_with_provenance();
    let rel_dir = workspace_root.join("releases").join("2026-07-31");
    let orgs_file = rel_dir.join("orgs.parquet");

    // Read existing orgs.parquet
    let file = File::open(&orgs_file)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let reader = builder.build()?;
    let schema = reader.schema();
    let mut batches = Vec::new();
    for b in reader {
        let b = b?;
        let mut cols = b.columns().to_vec();
        let role_names_idx = schema.index_of("role_names")?;
        // Build mismatched role_names list with empty array
        let offsets = arrow::buffer::OffsetBuffer::from_lengths(vec![0; b.num_rows()]);
        let values = std::sync::Arc::new(arrow::array::StringArray::from(Vec::<String>::new()))
            as arrow::array::ArrayRef;
        let field = std::sync::Arc::new(arrow::datatypes::Field::new(
            "item",
            arrow::datatypes::DataType::Utf8,
            true,
        ));
        let empty_list = arrow::array::ListArray::new(field, offsets, values, None);
        cols[role_names_idx] = std::sync::Arc::new(empty_list);
        let modified_batch = arrow::record_batch::RecordBatch::try_new(schema.clone(), cols)?;
        batches.push(modified_batch);
    }

    // Write back modified orgs.parquet
    {
        let out_file = File::create(&orgs_file)?;
        // The rewritten table keeps the provenance the build embedded: only its rows change.
        let mut writer = ArrowWriter::try_new(out_file, schema, Some(same_provenance(&rel_dir)))?;
        for b in &batches {
            writer.write(b)?;
        }
        writer.close()?;
    }

    let args = ods::commands::audit::Args {
        input: Some(outer_zip_path),
        workspace: Some(workspace_root),
        json: false,
        sample: 50,
        full: true,
        all: false,
    };

    let result = ods::commands::audit::run(args);
    assert!(
        result.is_err(),
        "audit must fail when role_codes and role_names length mismatches"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("discrepanc"),
        "error message must report discrepancies, got: {}",
        err_msg
    );
    Ok(())
}

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
  <un:Organisations>
    <un:Organisation>
      <un:Name>TEST PRACTICE</un:Name>
      <un:OrgId root="2.16.840.1.113883.2.1.3.2.4.18.48" extension="A100" />
      <un:Status value="Active" />
      <un:Date type="Legal"><un:Start value="2020-01-01" /></un:Date>
      <un:OrgRecordClass value="RC1" />
      <un:PrimaryRoleId id="RO177" uniqueRoleId="1" status="Active" display_name="Prescribing Cost Centre" />
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
    prov.trud_release_sha256_verified = Some(true);
    prov.trud_release_url = Some("https://isd.digital.nhs.uk/trud/items/341/hscorgrefdataxml_data_7.0.0_20260731000001.zip".to_string());
    prov.publication_date = Some("2026-07-28".to_string());
    prov.publication_seq_num = Some("4700".to_string());
    prov.publication_type = Some("Full".to_string());
    prov.publication_record_count = Some(1);

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

    ods::provenance::update_provenance_and_write_sha256sums(&rel_dir).unwrap();

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
    prov.trud_release_sha256_verified = Some(false);
    fs::write(&prov_path, serde_json::to_string_pretty(&prov)?)?;

    let args = ods::commands::audit::Args {
        input: Some(zip_path),
        workspace: Some(workspace_root),
        json: false,
        sample: 50,
        full: true,
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
    };

    let result = ods::commands::audit::run(args);
    assert!(result.is_err(), "audit must fail on tampered SHA256SUMS file");
    Ok(())
}

#[test]
fn test_audit_runs_full_suite_and_fails_on_corrupted_provenance_derived_artifacts() -> Result<()> {
    let (_tmp, workspace_root, zip_path) = setup_valid_workspace_with_provenance();
    let (_, active_dir) = ods::workspace::get_active_release(&workspace_root)?;

    // Mutate derived_artifacts in _provenance.json
    let prov_path = active_dir.join(ods::provenance::PROVENANCE_FILENAME);
    let mut prov: ods::provenance::OdsProvenance = serde_json::from_str(&fs::read_to_string(&prov_path)?)?;
    if let Some(ref mut map) = prov.derived_artifacts {
        map.insert("orgs.parquet".to_string(), "0000000000000000000000000000000000000000000000000000000000000000".to_string());
    }
    fs::write(&prov_path, serde_json::to_string_pretty(&prov)?)?;

    let args = ods::commands::audit::Args {
        input: Some(zip_path),
        workspace: Some(workspace_root),
        json: false,
        sample: 50,
        full: true,
    };

    let result = ods::commands::audit::run(args);
    assert!(result.is_err(), "audit must fail on corrupted derived_artifacts in _provenance.json");
    Ok(())
}

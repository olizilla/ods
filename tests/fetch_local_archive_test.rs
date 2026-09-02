use anyhow::Result;
use std::fs;
use tempfile::TempDir;

fn create_mock_zip(path: &std::path::Path, date_str: &str) -> Result<()> {
    use std::io::Write;
    let file = fs::File::create(path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file(format!("test_{}.xml", date_str), options)?;
    zip.write_all(format!("<HSCOrgRefData><Manifest><PublicationDate value=\"{}\"/></Manifest></HSCOrgRefData>", date_str).as_bytes())?;
    zip.finish()?;
    Ok(())
}

#[test]
fn test_fetch_local_archive_isolates_release_dir_and_updates_current_link() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let workspace_root = temp_dir.path().join("ods_data");
    fs::create_dir_all(&workspace_root)?;

    // 1. Create initial release 2026-07-31 and set active link
    let rel_2026 = workspace_root.join("releases").join("2026-07-31");
    let trud_2026 = rel_2026.join("trud");
    fs::create_dir_all(&trud_2026)?;
    create_mock_zip(&trud_2026.join("hscorgrefdataxml_data_7.0.0_20260731000001.zip"), "2026-07-31")?;
    ods::workspace::Workspace::open_or_create(Some(&workspace_root))?.set_active("2026-07-31")?;

    // 2. Prepare a local archive for release 2025-05-01
    let local_archive_dir = temp_dir.path().join("local_source");
    fs::create_dir_all(&local_archive_dir)?;
    let local_zip = local_archive_dir.join("hscorgrefdataxml_data_6.5.0_20250501000001.zip");
    create_mock_zip(&local_zip, "2025-05-01")?;

    // 3. Execute fetch with local_archive targeting workspace_root
    let args = ods::commands::fetch::Args {
        local_archive: Some(local_zip.clone()),
        workspace: Some(workspace_root.clone()),
        ..Default::default()
    };

    ods::commands::fetch::run(args)?;

    // 4. Assert 2025 release dir contains 2025 zip
    let rel_2025 = workspace_root.join("releases").join("2025-05-01");
    let zip_2025 = rel_2025.join("trud").join("hscorgrefdataxml_data_6.5.0_20250501000001.zip");
    assert!(zip_2025.exists(), "2025 release dir must contain 2025 zip");

    // 5. Assert 2026 release dir does NOT contain 2025 zip
    let bled_zip = trud_2026.join("hscorgrefdataxml_data_6.5.0_20250501000001.zip");
    assert!(!bled_zip.exists(), "2026 release dir must NOT contain 2025 zip");

    // 6. Assert current link was updated to 2025-05-01
    let (active_date, active_path) = ods::workspace::Workspace::open(Some(&workspace_root))?.active_release()?;
    assert_eq!(active_date, "2025-05-01", "current release link must be updated to 2025-05-01");
    let canon_rel_2025 = fs::canonicalize(&rel_2025)?;
    let canon_active_path = fs::canonicalize(&active_path)?;
    assert_eq!(canon_active_path, canon_rel_2025, "current release path must point to 2025 release dir");

    let prov_path = rel_2025.join(ods::provenance::PROVENANCE_FILENAME);
    let prov_content = fs::read_to_string(&prov_path)?;
    let prov: ods::provenance::OdsProvenance = serde_json::from_str(&prov_content)?;

    assert_eq!(prov.trud_release_date.as_deref(), Some("2025-05-01"));
    assert_eq!(prov.trud_release_name.as_deref(), Some("Release 6.5.0"));
    assert_eq!(
        prov.trud_release_sha256_verified,
        Some(ods::provenance::TrudVerificationSource::Unverified)
    );

    Ok(())
}

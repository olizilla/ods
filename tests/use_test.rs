mod common;

use anyhow::Result;
use common::make_v1_index;
use ods::commands::use_cmd::{run as use_run, Args as UseArgs};
use std::fs;
use tempfile::TempDir;

#[test]
fn test_use_refuses_when_release_does_not_exist() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");
    fs::create_dir_all(&workspace).unwrap();

    let res = use_run(UseArgs {
        release_date: "2026-07-31".to_string(),
        workspace: Some(workspace.clone()),
    });

    assert!(res.is_err());
    let err = res.unwrap_err().to_string();
    assert!(err.contains("Release 2026-07-31 not found"));
    assert!(err.contains("Run 'ods pull 2026-07-31' to download it"));
}

#[test]
fn test_use_refuses_when_release_unverified() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");
    let rel_dir = workspace.join("releases").join("2026-07-31");
    fs::create_dir_all(&rel_dir).unwrap();

    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_date = Some("2026-07-31".to_string());
    fs::write(
        rel_dir.join(ods::provenance::PROVENANCE_FILENAME),
        serde_json::to_string_pretty(&prov).unwrap(),
    )
    .unwrap();
    fs::write(rel_dir.join("datapackage.json"), b"{\"name\": \"ods\", \"version\": \"1.0.1\"}").unwrap();

    fs::write(rel_dir.join("orgs.parquet"), b"dummy content").unwrap();

    // Cache an index with a different manifest digest
    let index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("1.0.1", "sha256:0000000000000000000000000000000000000000000000000000000000000000")],
    )]);
    let index_bytes = serde_json::to_vec_pretty(&index).unwrap();
    ods::index::OdsReleaseIndex::save_to_workspace_bytes(&index_bytes, &workspace).unwrap();

    let res = use_run(UseArgs {
        release_date: "2026-07-31".to_string(),
        workspace: Some(workspace),
    });

    assert!(res.is_err());
    let err = res.unwrap_err().to_string();
    assert!(err.contains("does not match the index"));
    assert!(err.contains("A file in this directory does not match the published release"));
}

#[test]
fn test_use_pins_verified_release_and_creates_current_link() -> Result<()> {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");
    let rel_dir = workspace.join("releases").join("2026-07-31");
    fs::create_dir_all(&rel_dir).unwrap();

    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_date = Some("2026-07-31".to_string());
    fs::write(
        rel_dir.join(ods::provenance::PROVENANCE_FILENAME),
        serde_json::to_string_pretty(&prov)?,
    )?;
    fs::write(rel_dir.join("datapackage.json"), b"{\"name\": \"ods\", \"version\": \"1.0.1\"}")?;

    let orgs_bytes = b"sample orgs parquet bytes";
    fs::write(rel_dir.join("orgs.parquet"), orgs_bytes)?;

    use_run(UseArgs {
        release_date: "2026-07-31".to_string(),
        workspace: Some(workspace.clone()),
    })?;

    let current = workspace.join("current");
    assert!(current.exists());
    let (active_date, path) = ods::workspace::Workspace::open(Some(&workspace))?.active_release()?;
    assert_eq!(active_date, "2026-07-31");
    assert_eq!(path, fs::canonicalize(&rel_dir)?);

    // Verify workspace README was generated
    let readme = workspace.join("README.md");
    assert!(readme.exists());
    let readme_content = fs::read_to_string(readme)?;
    assert!(readme_content.contains("2026-07-31"));

    Ok(())
}

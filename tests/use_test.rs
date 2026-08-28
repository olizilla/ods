use anyhow::Result;
use ods::commands::use_cmd::{run as use_run, Args as UseArgs};
use ods::provenance::compute_file_sha256;
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

    fs::write(rel_dir.join("orgs.parquet"), b"dummy content").unwrap();
    fs::write(
        rel_dir.join("SHA256SUMS"),
        "0000000000000000000000000000000000000000000000000000000000000000  orgs.parquet\n",
    )
    .unwrap();

    let res = use_run(UseArgs {
        release_date: "2026-07-31".to_string(),
        workspace: Some(workspace),
    });

    assert!(res.is_err());
    let err = res.unwrap_err().to_string();
    assert!(err.contains("has unverified or missing files (SHA256SUMS mismatch)"));
}

#[test]
fn test_use_pins_verified_release_and_creates_current_link() -> Result<()> {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");
    let rel_dir = workspace.join("releases").join("2026-07-31");
    fs::create_dir_all(&rel_dir).unwrap();

    let orgs_bytes = b"sample orgs parquet bytes";
    fs::write(rel_dir.join("orgs.parquet"), orgs_bytes).unwrap();
    let hash = compute_file_sha256(&rel_dir.join("orgs.parquet"))?;
    fs::write(
        rel_dir.join("SHA256SUMS"),
        format!("{}  orgs.parquet\n", hash),
    )?;

    use_run(UseArgs {
        release_date: "2026-07-31".to_string(),
        workspace: Some(workspace.clone()),
    })?;

    let current = workspace.join("current");
    assert!(current.exists());
    let (active_date, path) = ods::workspace::get_active_release(&workspace)?;
    assert_eq!(active_date, "2026-07-31");
    assert_eq!(path, fs::canonicalize(&rel_dir)?);

    // Verify workspace README was generated
    let readme = workspace.join("README.md");
    assert!(readme.exists());
    let readme_content = fs::read_to_string(readme)?;
    assert!(readme_content.contains("2026-07-31"));

    Ok(())
}

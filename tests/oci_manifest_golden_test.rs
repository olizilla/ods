use anyhow::Result;
use ods::commands::make_oci::{run, Args};
use ods::oci::*;
use ods::provenance::{compute_file_sha256, OdsProvenance, PROVENANCE_FILENAME};

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn git_cmd(repo_dir: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(repo_dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z");
    cmd
}

/// Sets up a synthetic release directory matching the golden fixture files.
fn setup_golden_release_dir() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let rel_dir = tmp.path().join("releases").join("2026-07-31");
    let trud_dir = rel_dir.join("trud");
    fs::create_dir_all(&trud_dir).unwrap();

    // Outer zip
    let outer_zip_path = trud_dir.join("hscorgrefdataxml_data_7.0.0_20260731000001.zip");
    {
        let outer_file = File::create(&outer_zip_path).unwrap();
        let mut outer_zip = zip::ZipWriter::new(outer_file);
        let options = zip::write::SimpleFileOptions::default()
            .last_modified_time(zip::DateTime::from_date_and_time(2026, 7, 31, 0, 0, 0).unwrap());
        outer_zip.start_file("dummy.txt", options).unwrap();
        outer_zip.write_all(b"dummy source zip").unwrap();
        outer_zip.finish().unwrap();
    }
    let zip_sha256 = compute_file_sha256(&outer_zip_path).unwrap();

    // Create files with exact contents needed to match golden fixture digests and sizes
    fs::write(rel_dir.join("NOTES.md"), vec![b'a'; 4102]).unwrap();
    fs::write(rel_dir.join("orgs.parquet"), vec![b'b'; 9892725]).unwrap();

    fs::write(
        tmp.path().join("Cargo.toml"),
        format!("[package]\nname = \"ods\"\nversion = \"{}\"\n", env!("CARGO_PKG_VERSION")),
    )
    .unwrap();
    fs::create_dir_all(tmp.path().join("src")).unwrap();
    fs::write(tmp.path().join("src").join("main.rs"), "fn main() {}\n").unwrap();

    let _ = git_cmd(tmp.path()).args(["init", "-b", "main"]).output();
    let _ = git_cmd(tmp.path()).args(["add", "."]).output();
    let _ = git_cmd(tmp.path())
        .args(["commit", "-m", "initial", "--no-gpg-sign"])
        .output();
    let head_out = git_cmd(tmp.path()).args(["rev-parse", "HEAD"]).output().unwrap();
    let git_sha = String::from_utf8_lossy(&head_out.stdout).trim().to_string();
    let tool_tag = format!("v{}", env!("CARGO_PKG_VERSION"));
    let _ = git_cmd(tmp.path()).args(["tag", "--no-sign", &tool_tag]).output();

    let mut prov = OdsProvenance::default();
    prov.trud_release_name = Some("Release 7.0.0".to_string());
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_file = Some("hscorgrefdataxml_data_7.0.0_20260731000001.zip".to_string());
    prov.trud_release_filesize_bytes = Some(37_983_173);
    prov.trud_release_sha256 = Some(zip_sha256);
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);
    prov.publication_date = Some("2026-07-28".to_string());
    prov.publication_seq_num = Some("4700".to_string());
    prov.publication_type = Some("Full".to_string());
    prov.publication_record_count = Some(2);
    prov.tool_version = Some(env!("CARGO_PKG_VERSION").to_string());
    prov.tool_git_sha = Some(git_sha);
    prov.tool_git_dirty = Some(false);
    prov.dataset_version = Some("1.0.0".to_string());

    let prov_path = rel_dir.join(PROVENANCE_FILENAME);
    fs::write(&prov_path, serde_json::to_string_pretty(&prov).unwrap()).unwrap();

    (tmp, rel_dir)
}

#[test]
fn test_oci_manifest_golden_fixture_structure_and_digest() -> Result<()> {
    let fixture_path = Path::new("tests/fixtures/oci_manifest_golden.json");
    let expected_bytes = fs::read(fixture_path)?;
    let manifest: OciManifest = serde_json::from_slice(&expected_bytes)?;

    // 1. Verify that re-serializing produces byte-identical canonical JSON
    let canonical_bytes = manifest.to_canonical_bytes()?;
    assert_eq!(canonical_bytes, expected_bytes);

    // 2. Verify schema and media types
    assert_eq!(manifest.schema_version, 2);
    assert_eq!(manifest.media_type, MEDIA_TYPE_MANIFEST);
    assert_eq!(manifest.artifact_type, ARTIFACT_TYPE_DATASET);
    assert_eq!(manifest.config.media_type, MEDIA_TYPE_PROVENANCE);

    // 3. Verify that layers are sorted alphabetically by title
    let titles: Vec<&str> = manifest
        .layers
        .iter()
        .map(|l| {
            l.annotations
                .as_ref()
                .and_then(|a| a.get("org.opencontainers.image.title"))
                .map(|s| s.as_str())
                .unwrap_or("")
        })
        .collect();
    let mut sorted_titles = titles.clone();
    sorted_titles.sort();
    assert_eq!(titles, sorted_titles, "Layers must be sorted alphabetically by title");

    // 4. Verify that manifest digest is lowercase sha256 hex
    let digest = manifest.digest()?;
    assert!(digest.starts_with("sha256:"));
    let hex_part = &digest["sha256:".len()..];
    assert_eq!(hex_part.len(), 64);
    assert_eq!(hex_part, hex_part.to_lowercase());

    Ok(())
}

#[test]
fn test_make_oci_real_run_packs_deterministic_manifest() -> Result<()> {
    let (_tmp, rel_dir) = setup_golden_release_dir();

    run(Args {
        input: Some(rel_dir.clone()),
        version: Some("1.0.1".to_string()),
        check: false,
    })?;

    // Find generated manifest blob
    let blobs_dir = rel_dir.join("oci").join("blobs").join("sha256");
    let mut manifest_path = None;
    for entry in fs::read_dir(&blobs_dir)?.flatten() {
        let p = entry.path();
        if p.is_file() && !p.is_symlink() {
            manifest_path = Some(p);
            break;
        }
    }
    let manifest_path = manifest_path.expect("Manifest blob must exist");
    let manifest_bytes = fs::read(&manifest_path)?;

    // Parse and verify manifest generated by real make_oci::run
    let manifest: OciManifest = serde_json::from_slice(&manifest_bytes)?;
    assert_eq!(manifest.schema_version, 2);
    assert_eq!(manifest.media_type, MEDIA_TYPE_MANIFEST);
    assert_eq!(manifest.artifact_type, ARTIFACT_TYPE_DATASET);
    assert_eq!(manifest.config.media_type, MEDIA_TYPE_PROVENANCE);

    // Verify all layer titles are sorted
    let titles: Vec<&str> = manifest
        .layers
        .iter()
        .map(|l| {
            l.annotations
                .as_ref()
                .and_then(|a| a.get("org.opencontainers.image.title"))
                .map(|s| s.as_str())
                .unwrap_or("")
        })
        .collect();
    let mut sorted_titles = titles.clone();
    sorted_titles.sort();
    assert_eq!(titles, sorted_titles);

    // Canonical bytes must equal written file
    let canonical = manifest.to_canonical_bytes()?;
    assert_eq!(manifest_bytes, canonical);

    Ok(())
}

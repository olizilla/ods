use anyhow::Result;
use ods::commands::make_oci::{perform_structural_checks, run, Args};
use ods::provenance::{compute_file_sha256, OdsProvenance, PROVENANCE_FILENAME};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Sets up a synthetic release directory ready for `ods make oci`.
fn setup_synthetic_release_dir() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let rel_dir = tmp.path().join("releases").join("2026-07-31");
    let trud_dir = rel_dir.join("trud");
    fs::create_dir_all(&trud_dir).unwrap();

    // Create synthetic outer zip in trud/
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

    // Create synthetic Parquet tables and notes
    fs::write(rel_dir.join("orgs.parquet"), b"dummy orgs parquet content").unwrap();
    fs::write(rel_dir.join("roles.parquet"), b"dummy roles parquet content").unwrap();
    fs::write(rel_dir.join("datapackage.json"), b"{\"name\": \"test\", \"version\": \"1.0.1\"}").unwrap();
    fs::write(rel_dir.join("NOTES.md"), b"# Release Notes\nTest release.").unwrap();

    let mut prov = OdsProvenance::default();
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_filesize_bytes = Some(37_983_173);
    prov.trud_release_sha256 = Some(zip_sha256.clone());

    let prov_path = rel_dir.join(PROVENANCE_FILENAME);
    fs::write(&prov_path, serde_json::to_string_pretty(&prov).unwrap()).unwrap();

    (tmp, rel_dir)
}

fn test_args(input: PathBuf, check: bool) -> Args {
    Args {
        input: Some(input),
        check,
        ..Default::default()
    }
}

#[test]
fn test_make_oci_generates_valid_layout_and_relative_symlinks() -> Result<()> {
    let (_tmp, rel_dir) = setup_synthetic_release_dir();

    run(test_args(rel_dir.clone(), false))?;

    // Verify root files exist
    assert!(rel_dir.join("_provenance.json").exists());
    assert!(!rel_dir.join("_release.json").exists(), "ods make oci must not write _release.json");
    assert!(!rel_dir.join("SHA256SUMS").exists(), "ods make oci must not write SHA256SUMS");

    // Verify oci/ files exist
    let oci_dir = rel_dir.join("oci");
    assert!(oci_dir.join("oci-layout").exists());
    assert!(oci_dir.join("index.json").exists());

    let blobs_dir = oci_dir.join("blobs").join("sha256");
    assert!(blobs_dir.is_dir());

    // Verify symlinks resolve and point to release root files
    let mut real_manifest_count = 0;
    let mut symlink_count = 0;
    for entry in fs::read_dir(&blobs_dir)?.flatten() {
        let p = entry.path();
        if p.is_symlink() {
            symlink_count += 1;
            let target = fs::canonicalize(&p)?;
            assert!(target.exists(), "Symlink target must exist: {}", target.display());
            assert_eq!(target.parent().unwrap(), fs::canonicalize(&rel_dir)?.as_path());
        } else if p.is_file() {
            real_manifest_count += 1;
        }
    }

    assert_eq!(real_manifest_count, 1, "Exactly one real manifest file in blobs/sha256/");
    assert!(symlink_count >= 4, "Must have symlinks for all published layers");

    // Verify index.json contents
    let index_bytes = fs::read(oci_dir.join("index.json"))?;
    let index: ods::oci::OciIndex = serde_json::from_slice(&index_bytes)?;
    assert_eq!(index.schema_version, 2);
    assert_eq!(index.media_type, ods::oci::MEDIA_TYPE_INDEX);
    assert_eq!(index.manifests.len(), 2);
    let ref0 = index.manifests[0]
        .annotations
        .as_ref()
        .unwrap()
        .get(ods::oci::ANNOTATION_REF_NAME)
        .unwrap();
    let ref1 = index.manifests[1]
        .annotations
        .as_ref()
        .unwrap()
        .get(ods::oci::ANNOTATION_REF_NAME)
        .unwrap();
    assert_eq!(ref0, "2026-07-31");
    assert_eq!(ref1, "2026-07-31_1.0.1");
    assert_eq!(index.manifests[0].media_type, ods::oci::MEDIA_TYPE_MANIFEST);
    assert_eq!(index.manifests[0].artifact_type, ods::oci::ARTIFACT_TYPE_DATASET);
    assert_eq!(index.manifests[1].media_type, ods::oci::MEDIA_TYPE_MANIFEST);
    assert_eq!(index.manifests[1].artifact_type, ods::oci::ARTIFACT_TYPE_DATASET);
    assert_eq!(index.manifests[0].digest, index.manifests[1].digest);

    // Re-running with check = true must succeed
    let failures = perform_structural_checks(&rel_dir, "1.0.1")?;
    assert!(failures.is_empty(), "All structural checks must pass on cleanly generated layout");

    Ok(())
}

#[test]
fn test_make_oci_regenerates_datapackage_with_full_semver() -> Result<()> {
    let (_tmp, rel_dir) = setup_synthetic_release_dir();
    run(test_args(rel_dir.clone(), false))?;

    let dp_content = fs::read_to_string(rel_dir.join("datapackage.json"))?;
    let dp_val: serde_json::Value = serde_json::from_str(&dp_content)?;
    assert_eq!(dp_val["version"], "1.0.1");

    // Also verify manifest layer digest for datapackage.json matches the 1.0.1 file on disk
    let dp_sha = compute_file_sha256(&rel_dir.join("datapackage.json"))?;
    let oci_dir = rel_dir.join("oci");
    let manifest_path = fs::read_dir(oci_dir.join("blobs").join("sha256"))?
        .flatten()
        .find(|e| e.path().is_file() && !e.path().is_symlink())
        .unwrap()
        .path();
    let manifest: ods::oci::OciManifest = serde_json::from_slice(&fs::read(manifest_path)?)?;
    let dp_layer = manifest
        .layers
        .iter()
        .find(|l| {
            l.annotations
                .as_ref()
                .and_then(|a| a.get("org.opencontainers.image.title"))
                .map(|t| t == "datapackage.json")
                .unwrap_or(false)
        })
        .unwrap();
    assert_eq!(dp_layer.digest, format!("sha256:{}", dp_sha.to_lowercase()));

    Ok(())
}

#[test]
fn test_make_oci_check_mode_verifies_without_writing() -> Result<()> {
    let (_tmp, rel_dir) = setup_synthetic_release_dir();
    run(test_args(rel_dir.clone(), false))?;

    // Now run with check: true
    let res = run(test_args(rel_dir, true));
    assert!(res.is_ok(), "Check mode must succeed on valid OCI bundle");

    Ok(())
}

#[test]
fn test_make_oci_idempotent_repack_after_rebuilt_content() -> Result<()> {
    let (_tmp, rel_dir) = setup_synthetic_release_dir();

    // First pack
    run(test_args(rel_dir.clone(), false))?;

    // Rebuild/modify a parquet file on disk
    fs::write(rel_dir.join("orgs.parquet"), b"rebuilt parquet content with changed bytes")?;

    // Second pack: must regenerate oci/ wholesale without stale blobs or symlinks
    let res = run(test_args(rel_dir.clone(), false));
    assert!(res.is_ok(), "Re-pack after changed content must succeed: {:?}", res.err());

    // Verify exactly one real manifest blob in blobs/sha256/
    let blobs_dir = rel_dir.join("oci").join("blobs").join("sha256");
    let mut real_manifests = 0;
    for entry in fs::read_dir(&blobs_dir)?.flatten() {
        if entry.path().is_file() && !entry.path().is_symlink() {
            real_manifests += 1;
        }
    }
    assert_eq!(real_manifests, 1, "Exactly one real manifest file after re-pack");

    // Verify all structural checks pass cleanly
    let failures = perform_structural_checks(&rel_dir, "1.0.1")?;
    assert!(failures.is_empty(), "All structural checks must pass on cleanly regenerated layout: {:?}", failures);

    Ok(())
}

#[test]
fn test_determinism_two_different_working_directories_produce_identical_manifests() -> Result<()> {
    let (tmp_a, rel_dir_a) = setup_synthetic_release_dir();
    let (tmp_b, rel_dir_b) = setup_synthetic_release_dir();

    run(test_args(rel_dir_a.clone(), false))?;
    run(test_args(rel_dir_b.clone(), false))?;

    let prov_a = fs::read_to_string(rel_dir_a.join(PROVENANCE_FILENAME))?;
    let prov_b = fs::read_to_string(rel_dir_b.join(PROVENANCE_FILENAME))?;
    assert_eq!(prov_a, prov_b, "_provenance.json must be byte-identical regardless of working directory");

    let dp_a = fs::read_to_string(rel_dir_a.join("datapackage.json"))?;
    let dp_b = fs::read_to_string(rel_dir_b.join("datapackage.json"))?;
    assert_eq!(dp_a, dp_b, "datapackage.json must be byte-identical regardless of working directory");

    let tmp_a_path = tmp_a.path().to_str().unwrap();
    let tmp_b_path = tmp_b.path().to_str().unwrap();
    assert!(!prov_a.contains(tmp_a_path), "no absolute path leaked in _provenance.json");
    assert!(!dp_a.contains(tmp_a_path), "no absolute path leaked in datapackage.json");
    assert!(!prov_b.contains(tmp_b_path), "no absolute path leaked in _provenance.json");
    assert!(!dp_b.contains(tmp_b_path), "no absolute path leaked in datapackage.json");

    let get_manifest_bytes = |dir: &Path| -> Result<Vec<u8>> {
        let blobs = dir.join("oci").join("blobs").join("sha256");
        for entry in fs::read_dir(blobs)?.flatten() {
            let path = entry.path();
            if path.is_file() && !path.is_symlink() {
                return Ok(fs::read(path)?);
            }
        }
        anyhow::bail!("No manifest file found");
    };

    let bytes_a = get_manifest_bytes(&rel_dir_a)?;
    let bytes_b = get_manifest_bytes(&rel_dir_b)?;

    assert_eq!(bytes_a, bytes_b, "Manifests built across different directories must be byte-identical");
    Ok(())
}

#[test]
fn test_refusal_repacking_from_disk_mismatch() -> Result<()> {
    let (_tmp, rel_dir) = setup_synthetic_release_dir();
    run(test_args(rel_dir.clone(), false))?;

    // Tamper with a source file on disk after packaging
    fs::write(rel_dir.join("orgs.parquet"), b"tampered content after packing")?;

    let failures = perform_structural_checks(&rel_dir, "1.0.1")?;
    assert!(
        failures.iter().any(|f| f.contains("Re-packing from source files yields non-identical manifest bytes")),
        "Must catch disk divergence during re-packing: {:?}",
        failures
    );
    Ok(())
}

#[test]
fn test_refusal_baseline_invalid_before_disk_mutations() {
    let tmp = TempDir::new().unwrap();
    let rel_dir = tmp.path().join("releases").join("2026-07-31");
    fs::create_dir_all(&rel_dir).unwrap();

    // Create an invalid baseline provenance (missing publication fields, tool version, etc.)
    let mut prov = OdsProvenance::default();
    prov.trud_release_date = Some("2026-07-31".to_string());
    fs::write(rel_dir.join(PROVENANCE_FILENAME), serde_json::to_string(&prov).unwrap()).unwrap();

    let res = run(test_args(rel_dir.clone(), false));
    assert!(res.is_err());
    assert!(!rel_dir.join("SHA256SUMS").exists(), "Must not write SHA256SUMS on baseline failure");
    assert!(!rel_dir.join("oci").exists(), "Must not create oci/ on baseline failure");
}

#[test]
fn test_refusal_stale_symlink_target_modified() -> Result<()> {
    let (_tmp, rel_dir) = setup_synthetic_release_dir();
    run(test_args(rel_dir.clone(), false))?;

    // Tamper with target file without updating symlink
    fs::write(rel_dir.join("roles.parquet"), b"modified roles content")?;

    let failures = perform_structural_checks(&rel_dir, "1.0.1")?;
    assert!(
        failures.iter().any(|f| f.contains("blob symlink stale")),
        "Must report stale symlink refusal: {:?}",
        failures
    );
    Ok(())
}

#[test]
fn test_refusal_broken_symlink() -> Result<()> {
    let (_tmp, rel_dir) = setup_synthetic_release_dir();
    run(test_args(rel_dir.clone(), false))?;

    let blobs_dir = rel_dir.join("oci").join("blobs").join("sha256");
    let broken_link = blobs_dir.join("ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff");
    #[cfg(unix)]
    std::os::unix::fs::symlink("../../../nonexistent_file.parquet", &broken_link)?;

    let failures = perform_structural_checks(&rel_dir, "1.0.1")?;
    assert!(
        failures.iter().any(|f| f.contains("blob symlink broken")),
        "Must report broken symlink refusal: {:?}",
        failures
    );
    Ok(())
}

#[test]
fn test_refusal_tampered_index_json_digest() -> Result<()> {
    let (_tmp, rel_dir) = setup_synthetic_release_dir();
    run(test_args(rel_dir.clone(), false))?;

    let index_path = rel_dir.join("oci").join("index.json");
    let mut index: ods::oci::OciIndex = serde_json::from_slice(&fs::read(&index_path)?)?;
    index.manifests[0].digest = "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_string();
    fs::write(&index_path, serde_json::to_vec_pretty(&index)?)?;

    let failures = perform_structural_checks(&rel_dir, "1.0.1")?;
    assert!(
        failures.iter().any(|f| f.contains("index.json names manifest digest")),
        "Must report tampered index.json refusal: {:?}",
        failures
    );
    Ok(())
}

#[test]
fn test_refusal_config_digest_disagrees_with_provenance_layer() -> Result<()> {
    let (_tmp, rel_dir) = setup_synthetic_release_dir();
    run(test_args(rel_dir.clone(), false))?;

    let blobs_dir = rel_dir.join("oci").join("blobs").join("sha256");
    for entry in fs::read_dir(&blobs_dir)?.flatten() {
        let p = entry.path();
        if p.is_file() && !p.is_symlink() {
            let mut manifest: ods::oci::OciManifest = serde_json::from_slice(&fs::read(&p)?)?;
            manifest.config.digest = "sha256:1111111111111111111111111111111111111111111111111111111111111111".to_string();
            fs::write(&p, manifest.to_canonical_bytes()?)?;
            break;
        }
    }

    let failures = perform_structural_checks(&rel_dir, "1.0.1")?;
    assert!(
        failures.iter().any(|f| f.contains("config.digest") && f.contains("_provenance.json layer digest")),
        "Must report config digest mismatch refusal: {:?}",
        failures
    );
    Ok(())
}

#[test]
fn test_refusal_uppercase_digest_in_manifest() -> Result<()> {
    let (_tmp, rel_dir) = setup_synthetic_release_dir();
    run(test_args(rel_dir.clone(), false))?;

    let blobs_dir = rel_dir.join("oci").join("blobs").join("sha256");
    for entry in fs::read_dir(&blobs_dir)?.flatten() {
        let p = entry.path();
        if p.is_file() && !p.is_symlink() {
            let mut manifest: ods::oci::OciManifest = serde_json::from_slice(&fs::read(&p)?)?;
            let hex_part = manifest.layers[0].digest.trim_start_matches("sha256:");
            manifest.layers[0].digest = format!("sha256:{}", hex_part.to_uppercase());
            fs::write(&p, manifest.to_canonical_bytes()?)?;
            break;
        }
    }

    let failures = perform_structural_checks(&rel_dir, "1.0.1")?;
    assert!(
        failures.iter().any(|f| f.contains("digest is not lowercase hex")),
        "Must report uppercase digest refusal: {:?}",
        failures
    );
    Ok(())
}

#[test]
fn test_refusal_datapackage_resource_hash_disagrees_with_layers() -> Result<()> {
    let (_tmp, rel_dir) = setup_synthetic_release_dir();
    run(test_args(rel_dir.clone(), false))?;

    // Tamper with datapackage.json resource hash
    let dp_path = rel_dir.join("datapackage.json");
    let mut dp: serde_json::Value = serde_json::from_slice(&fs::read(&dp_path)?)?;
    dp["resources"][0]["hash"] = serde_json::json!("sha256:0000000000000000000000000000000000000000000000000000000000000000");
    fs::write(&dp_path, serde_json::to_string_pretty(&dp)?)?;

    let failures = perform_structural_checks(&rel_dir, "1.0.1")?;
    assert!(
        failures.iter().any(|f| f.contains("disagrees with layer digest")),
        "Must report datapackage resource hash disagreement: {:?}",
        failures
    );
    Ok(())
}

#[test]
fn test_make_oci_refuses_missing_trud_release_sha256() -> Result<()> {
    let (_tmp, rel_dir) = setup_synthetic_release_dir();

    // Remove trud_release_sha256 from provenance
    let prov_path = rel_dir.join(PROVENANCE_FILENAME);
    let mut prov: OdsProvenance = serde_json::from_str(&fs::read_to_string(&prov_path)?)?;
    prov.trud_release_sha256 = None;
    fs::write(&prov_path, serde_json::to_string_pretty(&prov)?)?;

    let result = run(test_args(rel_dir.clone(), false));
    assert!(result.is_err(), "ods make oci must refuse provenance missing trud_release_sha256");
    let err_str = format!("{:#}", result.unwrap_err());
    assert!(
        err_str.contains("trud_release_sha256"),
        "error message must mention missing trud_release_sha256, got: {}",
        err_str
    );

    Ok(())
}

#[test]
fn test_make_oci_reads_version_from_datapackage_json() -> Result<()> {
    let (_tmp, rel_dir) = setup_synthetic_release_dir();

    // Set dataset_version to 1.2.3 in datapackage.json
    let dp_path = rel_dir.join("datapackage.json");
    fs::write(&dp_path, b"{\"name\": \"test\", \"version\": \"1.2.3\"}")?;

    run(test_args(rel_dir.clone(), false))?;

    // Verify manifest annotations
    let oci_dir = rel_dir.join("oci");
    let blobs_dir = oci_dir.join("blobs").join("sha256");
    let mut manifest_path = None;
    for entry in fs::read_dir(&blobs_dir)?.flatten() {
        let p = entry.path();
        if p.is_file() && !p.is_symlink() {
            manifest_path = Some(p);
            break;
        }
    }
    let manifest_bytes = fs::read(manifest_path.unwrap())?;
    let manifest: ods::oci::OciManifest = serde_json::from_slice(&manifest_bytes)?;
    let annotations = manifest.annotations.as_ref().unwrap();
    assert_eq!(
        annotations.get(ods::oci::ANNOTATION_VERSION).map(|s| s.as_str()),
        Some("1.2.3")
    );
    assert_eq!(
        annotations.get(ods::oci::ANNOTATION_FYI_DATASET_VERSION).map(|s| s.as_str()),
        Some("1.2.3")
    );

    // Verify index.json has tag 2026-07-31_1.2.3
    let index_bytes = fs::read(oci_dir.join("index.json"))?;
    let index: ods::oci::OciIndex = serde_json::from_slice(&index_bytes)?;
    let has_tagged_entry = index.manifests.iter().any(|m| {
        m.annotations
            .as_ref()
            .and_then(|a| a.get(ods::oci::ANNOTATION_REF_NAME))
            .map(|r| r == "2026-07-31_1.2.3")
            .unwrap_or(false)
    });
    assert!(has_tagged_entry, "index.json must contain 2026-07-31_1.2.3 tag");

    Ok(())
}

#[test]
fn test_make_oci_refuses_when_missing_dataset_version() -> Result<()> {
    let (_tmp, rel_dir) = setup_synthetic_release_dir();

    // Remove version from datapackage.json
    let dp_path = rel_dir.join("datapackage.json");
    fs::write(&dp_path, b"{\"name\": \"test\"}")?;

    let result = run(test_args(rel_dir.clone(), false));
    assert!(result.is_err(), "ods make oci must refuse when dataset_version is missing");
    let err_str = format!("{:#}", result.unwrap_err());
    assert!(
        err_str.contains("Missing version in datapackage.json"),
        "error must mention missing version in datapackage.json, got: {}",
        err_str
    );
    assert!(
        err_str.contains("ods make"),
        "error must point at ods make as the fix, got: {}",
        err_str
    );

    Ok(())
}


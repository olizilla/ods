use anyhow::Result;
use ods::commands::make_oci::{perform_structural_checks, run, Args};
use ods::provenance::compute_file_sha256;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// The fixture's files carry this build's dataset version, as `ods make` embeds it, so every
/// structural check in this file expects it too.
const EXPECTED_VERSION: &str = ods::datapackage::DATASET_VERSION;

/// The object the fixture's files carry: a 2026-07-31 build from the fixture zip.
fn embedded(zip_sha256: &str) -> ods::provenance::Embedded {
    ods::provenance::fixture_embedded_for("2026-07-31", zip_sha256, 37_983_173)
}

/// Rewrites `rel_dir/name` as a stub Parquet file with new content, carrying the same object
/// its sibling `orgs.parquet` or `roles.parquet` carries: a file changed after packing, still
/// one release's file.
fn rewrite(rel_dir: &Path, name: &str, content: &str) {
    let record = ods::provenance::read_release(rel_dir).unwrap();
    let embedded = record.facts().unwrap().embedded.clone();
    ods::commands::parquet::write_stub_parquet(&rel_dir.join(name), Some(&embedded), content).unwrap();
}

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

    // Create synthetic Parquet tables, carrying the release's provenance, and notes
    let embedded = embedded(&zip_sha256);
    ods::commands::parquet::write_stub_parquet(&rel_dir.join("orgs.parquet"), Some(&embedded), "dummy orgs parquet content").unwrap();
    ods::commands::parquet::write_stub_parquet(&rel_dir.join("roles.parquet"), Some(&embedded), "dummy roles parquet content").unwrap();
    fs::write(rel_dir.join("NOTES.md"), b"# Release Notes\nTest release.").unwrap();

    // The pull record lives beside the archive it describes, not the release root.
    ods::provenance::write_pull_record(
        &rel_dir,
        "2026-07-31",
        "hscorgrefdataxml_data_7.0.0_20260731000001.zip",
        &zip_sha256,
        37_983_173,
        &[],
    )
    .unwrap();

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

    // Verify the pull record stays where it was written (trud/), and nothing is added at the root
    assert!(ods::provenance::pull_record_path(&rel_dir).exists());
    assert!(!rel_dir.join("datapackage.json").exists(), "ods make oci writes only oci/");
    assert!(!rel_dir.join("_release.json").exists(), "ods make oci must not write _release.json");
    assert!(!rel_dir.join("SHA256SUMS").exists(), "ods make oci must not write SHA256SUMS");

    // Verify oci/ files exist
    let oci_dir = rel_dir.join("oci");
    assert!(oci_dir.join("oci-layout").exists());
    assert!(oci_dir.join("index.json").exists());

    let blobs_dir = oci_dir.join("blobs").join("sha256");
    assert!(blobs_dir.is_dir());

    // Verify symlinks resolve and point to release root files
    let mut real_file_count = 0;
    let mut symlink_count = 0;
    for entry in fs::read_dir(&blobs_dir)?.flatten() {
        let p = entry.path();
        if p.is_symlink() {
            symlink_count += 1;
            let target = fs::canonicalize(&p)?;
            assert!(target.exists(), "Symlink target must exist: {}", target.display());
            assert_eq!(target.parent().unwrap(), fs::canonicalize(&rel_dir)?.as_path());
        } else if p.is_file() {
            real_file_count += 1;
        }
    }

    // Manifest-only: the two real blobs are the manifest and the fixed empty config; every
    // layer (one per top-level `*.parquet` file — `orgs.parquet` and `roles.parquet` here) is a
    // symlink to the real file at the release root.
    assert_eq!(real_file_count, 2, "Exactly the manifest and the empty config blob, both real files");
    assert_eq!(symlink_count, 2, "One symlink per Parquet layer");

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
    assert_eq!(ref1, &format!("2026-07-31_{EXPECTED_VERSION}"));
    assert_eq!(index.manifests[0].media_type, ods::oci::MEDIA_TYPE_MANIFEST);
    assert_eq!(index.manifests[0].artifact_type.as_deref(), Some(ods::oci::ARTIFACT_TYPE_DATASET));
    assert_eq!(index.manifests[1].media_type, ods::oci::MEDIA_TYPE_MANIFEST);
    assert_eq!(index.manifests[1].artifact_type.as_deref(), Some(ods::oci::ARTIFACT_TYPE_DATASET));
    assert_eq!(index.manifests[0].digest, index.manifests[1].digest);

    // Re-running with check = true must succeed
    let failures = perform_structural_checks(&rel_dir, EXPECTED_VERSION)?;
    assert!(failures.is_empty(), "All structural checks must pass on cleanly generated layout: {:?}", failures);

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
    rewrite(&rel_dir, "orgs.parquet", "rebuilt parquet content with changed bytes");

    // Second pack: must regenerate oci/ wholesale without stale blobs or symlinks
    let res = run(test_args(rel_dir.clone(), false));
    assert!(res.is_ok(), "Re-pack after changed content must succeed: {:?}", res.err());

    // Verify exactly the manifest and the fixed empty config blob remain as real files — no
    // stale manifest blob left behind from the first pack.
    let blobs_dir = rel_dir.join("oci").join("blobs").join("sha256");
    let mut real_files = 0;
    for entry in fs::read_dir(&blobs_dir)?.flatten() {
        if entry.path().is_file() && !entry.path().is_symlink() {
            real_files += 1;
        }
    }
    assert_eq!(real_files, 2, "Exactly the manifest and the empty config blob after re-pack");

    // Verify all structural checks pass cleanly
    let failures = perform_structural_checks(&rel_dir, EXPECTED_VERSION)?;
    assert!(failures.is_empty(), "All structural checks must pass on cleanly regenerated layout: {:?}", failures);

    Ok(())
}

#[test]
fn test_determinism_two_different_working_directories_produce_identical_manifests() -> Result<()> {
    let (tmp_a, rel_dir_a) = setup_synthetic_release_dir();
    let (tmp_b, rel_dir_b) = setup_synthetic_release_dir();

    run(test_args(rel_dir_a.clone(), false))?;
    run(test_args(rel_dir_b.clone(), false))?;

    let prov_a = fs::read_to_string(ods::provenance::pull_record_path(&rel_dir_a))?;
    let prov_b = fs::read_to_string(ods::provenance::pull_record_path(&rel_dir_b))?;
    assert_eq!(prov_a, prov_b, "trud/datapackage.json must be byte-identical regardless of working directory");

    let tmp_a_path = tmp_a.path().to_str().unwrap();
    let tmp_b_path = tmp_b.path().to_str().unwrap();
    assert!(!prov_a.contains(tmp_a_path), "no absolute path leaked in trud/datapackage.json");
    assert!(!prov_b.contains(tmp_b_path), "no absolute path leaked in trud/datapackage.json");

    let get_manifest_bytes = |dir: &Path| -> Result<Vec<u8>> {
        let (path, _) = find_manifest_blob_path(dir)?;
        Ok(fs::read(path)?)
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
    rewrite(&rel_dir, "orgs.parquet", "tampered content after packing");

    let failures = perform_structural_checks(&rel_dir, EXPECTED_VERSION)?;
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

    // A file built without provenance: nothing to annotate a manifest with
    ods::commands::parquet::write_stub_parquet(
        &rel_dir.join("orgs.parquet"),
        None,
        "orgs",
    )
    .unwrap();

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
    rewrite(&rel_dir, "roles.parquet", "modified roles content");

    let failures = perform_structural_checks(&rel_dir, EXPECTED_VERSION)?;
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

    let failures = perform_structural_checks(&rel_dir, EXPECTED_VERSION)?;
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

    let failures = perform_structural_checks(&rel_dir, EXPECTED_VERSION)?;
    assert!(
        failures.iter().any(|f| f.contains("index.json names manifest digest")),
        "Must report tampered index.json refusal: {:?}",
        failures
    );
    Ok(())
}

/// The real manifest blob's path and parsed content — never the empty config blob, which sits
/// beside it in `blobs/sha256/` as another real (non-symlink) file and doesn't parse as a
/// manifest (it's just the two bytes `{}`), but which `fs::read_dir`'s arbitrary order could
/// otherwise hand back first.
fn find_manifest_blob_path(rel_dir: &Path) -> Result<(PathBuf, ods::oci::OciManifest)> {
    let blobs_dir = rel_dir.join("oci").join("blobs").join("sha256");
    for entry in fs::read_dir(&blobs_dir)?.flatten() {
        let p = entry.path();
        if p.is_file() && !p.is_symlink() {
            if let Ok(m) = serde_json::from_slice::<ods::oci::OciManifest>(&fs::read(&p)?) {
                return Ok((p, m));
            }
        }
    }
    anyhow::bail!("no manifest blob found under {}", blobs_dir.display());
}

#[test]
fn test_refusal_config_digest_isnt_the_fixed_empty_descriptor() -> Result<()> {
    let (_tmp, rel_dir) = setup_synthetic_release_dir();
    run(test_args(rel_dir.clone(), false))?;

    let (manifest_path, mut manifest) = find_manifest_blob_path(&rel_dir)?;
    manifest.config.digest = "sha256:1111111111111111111111111111111111111111111111111111111111111111".to_string();
    fs::write(&manifest_path, manifest.to_canonical_bytes()?)?;

    let failures = perform_structural_checks(&rel_dir, EXPECTED_VERSION)?;
    assert!(
        failures.iter().any(|f| f.contains("config isn't the fixed empty descriptor")),
        "Must report config digest mismatch refusal: {:?}",
        failures
    );
    Ok(())
}

#[test]
fn test_refusal_uppercase_digest_in_manifest() -> Result<()> {
    let (_tmp, rel_dir) = setup_synthetic_release_dir();
    run(test_args(rel_dir.clone(), false))?;

    let (manifest_path, mut manifest) = find_manifest_blob_path(&rel_dir)?;
    let hex_part = manifest.layers[0].digest.trim_start_matches("sha256:");
    manifest.layers[0].digest = format!("sha256:{}", hex_part.to_uppercase());
    fs::write(&manifest_path, manifest.to_canonical_bytes()?)?;

    let failures = perform_structural_checks(&rel_dir, EXPECTED_VERSION)?;
    assert!(
        failures.iter().any(|f| f.contains("digest is not lowercase hex")),
        "Must report uppercase digest refusal: {:?}",
        failures
    );
    Ok(())
}

#[test]
fn test_make_oci_refuses_an_embedded_object_without_a_source_hash() -> Result<()> {
    let (_tmp, rel_dir) = setup_synthetic_release_dir();

    // The files carry a source with no hash
    let mut broken = embedded(&"0".repeat(64));
    broken.sources[0].hash = String::new();
    for name in ["orgs.parquet", "roles.parquet"] {
        ods::commands::parquet::write_stub_parquet(&rel_dir.join(name), Some(&broken), name)?;
    }

    let result = run(test_args(rel_dir.clone(), false));
    assert!(result.is_err(), "ods make oci must refuse files whose source has no hash");
    let err_str = format!("{:#}", result.unwrap_err());
    assert!(err_str.contains("carry provenance this ods can't read"), "got: {}", err_str);
    assert!(err_str.contains("hash"), "error message must name the hash, got: {}", err_str);
    assert!(!rel_dir.join("oci").exists(), "Must not create oci/ on refusal");

    Ok(())
}

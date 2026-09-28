use anyhow::Result;
use ods::commands::make_oci::{run, Args};
use ods::oci::*;
use ods::provenance::compute_file_sha256;

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Sets up a synthetic release directory: `orgs.parquet` carrying the release's provenance, the
/// pull record under `trud/`, and `NOTES.md` and `datapackage.json` alongside, to prove the
/// manifest is made of the top-level `*.parquet` files alone.
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

    fs::write(rel_dir.join("NOTES.md"), vec![b'a'; 4102]).unwrap();
    let embedded = ods::provenance::fixture_embedded_for("2026-07-31", &zip_sha256, 37_983_173);
    ods::commands::parquet::write_stub_parquet(&rel_dir.join("orgs.parquet"), Some(&embedded), "orgs").unwrap();
    fs::write(rel_dir.join("datapackage.json"), b"{\"name\": \"test\", \"version\": \"1.0.1\"}").unwrap();

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

#[test]
fn test_oci_manifest_golden_fixture_structure_and_digest() -> Result<()> {
    let fixture_path = Path::new("tests/fixtures/oci_manifest_golden.json");
    let expected_bytes = fs::read(fixture_path)?;
    let manifest: OciManifest = serde_json::from_slice(&expected_bytes)?;

    // 1. Verify that re-serializing produces byte-identical canonical JSON
    let canonical_bytes = manifest.to_canonical_bytes()?;
    assert_eq!(canonical_bytes, expected_bytes);

    // 2. Verify schema and media types: manifest-only, so the config is the fixed empty
    // descriptor, never a descriptor blob's own digest.
    assert_eq!(manifest.schema_version, 2);
    assert_eq!(manifest.media_type, MEDIA_TYPE_MANIFEST);
    assert_eq!(manifest.artifact_type, ARTIFACT_TYPE_DATASET);
    assert!(ods::oci::dataset::is_empty_config(&manifest.config));
    for layer in &manifest.layers {
        assert_eq!(layer.media_type, MEDIA_TYPE_PARQUET);
    }

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

    // D1: the manifest says nothing about which ods built it, so a dataset's digest is a function
    // of the files, whichever ods rebuilds it.
    let tool_annotations = |m: &OciManifest| -> Vec<String> {
        m.annotations
            .iter()
            .flat_map(|a| a.keys())
            .filter(|k| k.starts_with("fyi.ods.tool-"))
            .cloned()
            .collect()
    };
    assert_eq!(tool_annotations(&manifest), Vec::<String>::new(), "the golden manifest names a tool build");

    // 4. Verify that manifest digest is lowercase sha256 hex
    let digest = manifest.digest()?;
    assert!(digest.starts_with("sha256:"));
    let hex_part = &digest["sha256:".len()..];
    assert_eq!(hex_part.len(), 64);
    assert_eq!(hex_part, hex_part.to_lowercase());

    // 5. Verify real make_oci run packs valid manifest matching canonical bytes, from a
    // directory that also holds NOTES.md and datapackage.json — neither becomes a layer.
    let (_tmp, rel_dir) = setup_golden_release_dir();
    run(Args {
        input: Some(rel_dir.clone()),
        ..Default::default()
    })?;

    let blobs_dir = rel_dir.join("oci").join("blobs").join("sha256");
    let mut manifest_path = None;
    for entry in fs::read_dir(&blobs_dir)?.flatten() {
        let p = entry.path();
        if p.is_file() && !p.is_symlink() && fs::read(&p)? != ods::oci::dataset::EMPTY_CONFIG {
            manifest_path = Some(p);
            break;
        }
    }
    let manifest_path = manifest_path.expect("Manifest blob must exist");
    let manifest_bytes = fs::read(&manifest_path)?;
    let gen_manifest: OciManifest = serde_json::from_slice(&manifest_bytes)?;
    assert_eq!(gen_manifest.schema_version, 2);
    assert_eq!(manifest_bytes, gen_manifest.to_canonical_bytes()?);
    assert_eq!(tool_annotations(&gen_manifest), Vec::<String>::new(), "ods make oci wrote a tool annotation");
    assert_eq!(gen_manifest.layers.len(), 1, "only orgs.parquet is a layer; NOTES.md and datapackage.json are not");

    Ok(())
}

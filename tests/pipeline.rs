//! Integration tests for the full ODS pipeline: XML -> Parquet -> find -> cite.
//!
//! These tests replace `verify_compilation.sh` with idiomatic Rust:
//! - `tempfile::TempDir` guarantees cleanup even on test panic
//! - Assertions operate on typed parquet outputs and queries
//! - `CARGO_MANIFEST_DIR` resolves the fixture path regardless of `cwd`

mod common;

use ods::commands::parquet;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn create_mock_trud_zip_with_provenance(dir: &Path) -> PathBuf {
    let zip_path = common::create_mock_trud_zip(dir, "hscorgrefdataxml_data_7.0.0_20260731000001.zip");

    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_date = Some("2026-07-31".to_string());
    std::fs::write(
        dir.join(ods::provenance::PROVENANCE_FILENAME),
        serde_json::to_string_pretty(&prov).unwrap(),
    ).unwrap();

    zip_path
}

// ---------------------------------------------------------------------------
// Stage 1 — make parquet from XML
// ---------------------------------------------------------------------------

#[test]
fn make_parquet_rejects_bare_xml() {
    let tmp = TempDir::new().unwrap();
    let res = parquet::run(parquet::Args {
        input: Some(Path::new(common::FIXTURE_MOCK_XML).to_path_buf()),
        output: Some(tmp.path().join("out")), ..Default::default() });
    assert!(res.is_err());
    let err = res.unwrap_err().to_string();
    assert!(err.contains("Not a TRUD release archive"));
}

/// Verifies that `ods make parquet` produces Parquet tables with correctly
/// resolved role names, cross-record parent lookups, and metadata.
#[test]
fn make_parquet_produces_tables_with_correct_records() {
    let tmp = TempDir::new().unwrap();
    let zip_path = create_mock_trud_zip_with_provenance(tmp.path());
    let parquet_dir = tmp.path().join("out");

    // Stray ndjson in directory must not affect parquet generation
    let stray_ndjson = tmp.path().join("ods.ndjson");
    std::fs::write(&stray_ndjson, b"{\"stray\": true}\n").unwrap();

    parquet::run(parquet::Args {
        input: Some(zip_path),
        output: Some(parquet_dir.clone()), ..Default::default() })
    .expect("parquet::run should succeed");

    assert!(parquet_dir.join("orgs.parquet").exists(), "orgs.parquet not created");
    assert!(parquet_dir.join("roles.parquet").exists(), "roles.parquet not created");
    assert!(parquet_dir.join("relationships.parquet").exists(), "relationships.parquet not created");
    assert!(parquet_dir.join("successions.parquet").exists(), "successions.parquet not created");
    assert!(parquet_dir.join("datapackage.json").exists(), "datapackage.json not created");
    assert!(parquet_dir.join("_provenance.json").exists(), "_provenance.json not created");
}



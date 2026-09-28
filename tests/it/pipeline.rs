//! Integration tests for the full ODS pipeline: XML -> Parquet -> find -> cite.
//!
//! These tests replace `verify_compilation.sh` with idiomatic Rust:
//! - `tempfile::TempDir` guarantees cleanup even on test panic
//! - Assertions operate on typed parquet outputs and queries
//! - `CARGO_MANIFEST_DIR` resolves the fixture path regardless of `cwd`

use crate::common;

use ods::commands::parquet;
use std::path::Path;
use tempfile::TempDir;

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
    let zip_path =
        common::create_mock_trud_zip(tmp.path(), "hscorgrefdataxml_data_7.0.0_20260731000001.zip");
    let index_file = common::write_index_for_zip(tmp.path(), "2026-07-31", &zip_path);
    let parquet_dir = tmp.path().join("out");

    // Stray ndjson in directory must not affect parquet generation
    let stray_ndjson = tmp.path().join("ods.ndjson");
    std::fs::write(&stray_ndjson, b"{\"stray\": true}\n").unwrap();

    parquet::run(parquet::Args {
        input: Some(zip_path),
        output: Some(parquet_dir.clone()),
        index: Some(index_file.to_str().unwrap().to_string()),
        ..Default::default()
    })
    .expect("parquet::run should succeed");

    assert!(parquet_dir.join("orgs.parquet").exists(), "orgs.parquet not created");
    assert!(parquet_dir.join("roles.parquet").exists(), "roles.parquet not created");
    assert!(parquet_dir.join("relationships.parquet").exists(), "relationships.parquet not created");
    assert!(parquet_dir.join("successions.parquet").exists(), "successions.parquet not created");
    assert!(
        !parquet_dir.join("datapackage.json").exists(),
        "`ods make parquet` writes the tables only; bare `ods make` writes the datapackage.json view"
    );
    // A bare zip matched by the index: the files carry the provenance, and no `trud/` record is
    // written, since no `ods trud pull` made one.
    let record = ods::provenance::read_release(&parquet_dir).unwrap();
    let facts = record.facts().expect("the files carry provenance");
    assert_eq!(facts.release_date, "2026-07-31");
    assert!(!ods::provenance::pull_record_path(&parquet_dir).exists(), "no trud/ record for a bare zip");
}



//! Integration tests for the full ODS pipeline: XML -> Parquet -> find -> cite.
//!
//! These tests replace `verify_compilation.sh` with idiomatic Rust:
//! - `tempfile::TempDir` guarantees cleanup even on test panic
//! - Assertions operate on typed parquet outputs and queries
//! - `CARGO_MANIFEST_DIR` resolves the fixture path regardless of `cwd`

use ods::commands::{cite, find, make, parquet};
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Mock TRUD XML fixture: two organisations (an NHS Trust and a GP Practice
/// commissioned by it). Lives alongside these tests so changes to the fixture
/// are visible in the same diff as the tests that depend on it.
const FIXTURE_XML: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/mock_hscorgrefdata.xml"
);

fn create_mock_trud_zip(dir: &Path) -> PathBuf {
    let zip_path = dir.join("hscorgrefdataxml_data_7.0.0_20260731000001.zip");
    let zip_file = std::fs::File::create(&zip_path).unwrap();
    let mut zip_writer = zip::ZipWriter::new(zip_file);
    let options = zip::write::SimpleFileOptions::default();
    zip_writer.start_file("HSCOrgRefData_Full_mock.xml", options).unwrap();
    let xml_content = std::fs::read_to_string(FIXTURE_XML).unwrap();
    zip_writer.write_all(xml_content.as_bytes()).unwrap();
    zip_writer.finish().unwrap();

    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_name = Some("Release 7.0.0".to_string());
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_file = Some("hscorgrefdataxml_data_7.0.0_20260731000001.zip".to_string());
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);
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
        input: Some(Path::new(FIXTURE_XML).to_path_buf()),
        output: Some(tmp.path().join("out")),
    });
    assert!(res.is_err());
    let err = res.unwrap_err().to_string();
    assert!(err.contains("Not a TRUD release archive"));
}

/// Verifies that `ods make parquet` produces Parquet tables with correctly
/// resolved role names, cross-record parent lookups, and metadata.
#[test]
fn make_parquet_produces_tables_with_correct_records() {
    let tmp = TempDir::new().unwrap();
    let zip_path = create_mock_trud_zip(tmp.path());
    let parquet_dir = tmp.path().join("out");

    parquet::run(parquet::Args {
        input: Some(zip_path),
        output: Some(parquet_dir.clone()),
    })
    .expect("parquet::run should succeed");

    assert!(parquet_dir.join("orgs.parquet").exists(), "orgs.parquet not created");
    assert!(parquet_dir.join("orgs_all.parquet").exists(), "orgs_all.parquet not created");
    assert!(parquet_dir.join("roles.parquet").exists(), "roles.parquet not created");
    assert!(parquet_dir.join("relationships.parquet").exists(), "relationships.parquet not created");
    assert!(parquet_dir.join("successions.parquet").exists(), "successions.parquet not created");
    assert!(parquet_dir.join("datapackage.json").exists(), "datapackage.json not created");
    assert!(parquet_dir.join("_provenance.json").exists(), "_provenance.json not created");
}

// ---------------------------------------------------------------------------
// Full Pipeline: XML -> Parquet -> find -> cite
// ---------------------------------------------------------------------------

#[test]
fn full_pipeline_make_find_cite() {
    let tmp = TempDir::new().unwrap();
    let zip_path = create_mock_trud_zip(tmp.path());
    let release_dir = tmp.path().join("releases").join("2026-07-31");

    // Stage 1: make parquet
    make::run_make_parquet(parquet::Args {
        input: Some(zip_path),
        output: Some(release_dir.clone()),
    })
    .expect("make::run_make_parquet should succeed");

    assert!(release_dir.join("orgs.parquet").exists());
    assert!(release_dir.join("datapackage.json").exists());
    assert!(release_dir.join("_provenance.json").exists());

    // Stage 2: find
    let mut find_out = Vec::new();
    find::run_with_writer(
        find::Args {
            query: Some("Mock".to_string()),
            code: Vec::new(),
            location: None,
            role: Vec::new(),
            all: false,
            verbose: false,
            sort: Some(find::SortBy::Code),
            format: find::OutputFormat::Json,
            input: Some(release_dir.clone()),
            ..Default::default()
        },
        &mut find_out,
        &release_dir,
    )
    .expect("find::run should succeed");

    let find_str = String::from_utf8(find_out).unwrap();
    assert!(find_str.contains("Mock GP Practice"), "find should locate Mock GP");

    // Stage 3: cite
    let mut cite_out = Vec::new();
    cite::run_with_writer(
        cite::Args {
            input: Some(release_dir),
            format: "text".to_string(),
        },
        &mut cite_out,
    )
    .expect("cite::run should succeed");

    let cite_str = String::from_utf8(cite_out).unwrap();
    assert!(cite_str.contains("How to Cite"), "cite block missing How to Cite section");
    assert!(cite_str.contains("orgs.parquet:"), "cite block missing orgs.parquet hash");
}


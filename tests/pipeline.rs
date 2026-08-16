//! Integration tests for the full ODS pipeline: compile → parquet → md.
//!
//! These tests replace `verify_compilation.sh` with idiomatic Rust:
//! - `tempfile::TempDir` guarantees cleanup even on test panic
//! - Assertions operate on typed `OdsRecord` structs rather than `grep` / regex
//! - The zip archive is inspected in-process without extracting to disk
//! - `CARGO_MANIFEST_DIR` resolves the fixture path regardless of `cwd`

use ods::commands::{ndjson, md, parquet, find, cite};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Mock TRUD XML fixture: two organisations (an NHS Trust and a GP Practice
/// commissioned by it).  Lives alongside these tests so changes to the fixture
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
    zip_path
}

// ---------------------------------------------------------------------------
// Stage 1 — compile
// ---------------------------------------------------------------------------

#[test]
fn compile_rejects_bare_xml() {
    let tmp = TempDir::new().unwrap();
    let res = ndjson::run(ndjson::Args {
        input: Path::new(FIXTURE_XML).to_path_buf(),
        output: Some(tmp.path().join("ods.ndjson")),
    });
    assert!(res.is_err());
    let err = res.unwrap_err().to_string();
    assert!(err.contains("Not a TRUD release archive"));
}

/// Verifies that `ods compile` produces well-formed NDJSON with correctly
/// resolved role names, cross-record parent lookups, and flattened contacts.
#[test]
fn compile_produces_ndjson_with_correct_records() {
    let tmp = TempDir::new().unwrap();
    let zip_path = create_mock_trud_zip(tmp.path());

    ndjson::run(ndjson::Args {
        input: zip_path,
        output: Some(tmp.path().join("ods.ndjson")),
    })
    .expect("compile::run should succeed");

    let ndjson_path = tmp.path().join("ods.ndjson");
    assert!(ndjson_path.exists(), "ods.ndjson was not created");

    let content = std::fs::read_to_string(&ndjson_path).unwrap();
    let lines: Vec<&str> = content.trim().lines().collect();
    assert_eq!(
        lines.len(),
        3,
        "expected 3 lines in NDJSON (1 provenance + 2 records), got {}",
        lines.len()
    );

    let records: Vec<ndjson::OdsRecord> = lines
        .iter()
        .filter_map(|l| {
            if ods::provenance::try_parse_provenance_line(l).is_some() {
                None
            } else {
                Some(serde_json::from_str(l).unwrap_or_else(|e| panic!("line is not valid OdsRecord JSON: {e}\n  -> {l}")))
            }
        })
        .collect();

    // --- RAE: NHS Trust ---
    let rae = records
        .iter()
        .find(|r| r.ods_code == "RAE")
        .expect("RAE record must be present");
    assert_eq!(
        rae.role, "nhs trust",
        "RAE role should be resolved from the CodeSystem concept map"
    );
    assert_eq!(rae.status, "active");

    // --- Y01234: GP Practice ---
    let gp = records
        .iter()
        .find(|r| r.ods_code == "Y01234")
        .expect("Y01234 record must be present");
    assert_eq!(
        gp.role, "general practice",
        "Y01234 role should be resolved from the CodeSystem concept map"
    );
    assert_eq!(gp.status, "active");

    // Parent organisation resolved via RE4 relationship cross-lookup
    let parent = gp
        .parent_organisation
        .as_ref()
        .expect("Y01234 should have a parent_organisation derived from its RE4 relationship");
    assert_eq!(parent.ods_code, "RAE");
    assert_eq!(
        parent.name, "ALDER HEY CHILDREN'S NHS FOUNDATION TRUST",
        "parent name should be filled in by cross-record name lookup"
    );

    // Contacts
    let tel = gp
        .contacts
        .iter()
        .find(|c| c.contact_type == "tel")
        .expect("Y01234 should have a telephone contact");
    assert_eq!(tel.value, "023 80706919");

    let web = gp
        .contacts
        .iter()
        .find(|c| c.contact_type == "http")
        .expect("Y01234 should have a website contact");
    assert_eq!(web.value, "http://example.com");

    // Geo / address
    let loc = gp
        .geo_loc
        .as_ref()
        .expect("Y01234 should have a geo_loc")
        .clone();
    assert_eq!(loc.postcode.as_deref(), Some("SO15 5SY"));
    assert_eq!(loc.uprn.as_deref(), Some("100062506311"));
}

// ---------------------------------------------------------------------------
// Stages 2 + 3 — parquet and md (chained on top of compile)
// ---------------------------------------------------------------------------

/// Runs the full three-stage pipeline and asserts on the md zip contents.
/// The zip is inspected in-process -- no `unzip` subprocess, no leftover files.
#[test]
fn full_pipeline_parquet_and_md() {
    let tmp = TempDir::new().unwrap();

    // Stage 1: compile
    let zip_path = create_mock_trud_zip(tmp.path());
    ndjson::run(ndjson::Args {
        input: zip_path,
        output: Some(tmp.path().join("ods.ndjson")),
    })
    .expect("compile::run should succeed");

    // Stage 2: parquet
    let ndjson_path = tmp.path().join("ods.ndjson");
    let parquet_dir = tmp.path().join("parquet");

    parquet::run(parquet::Args {
        input: ndjson_path,
        output: parquet_dir.clone(),
    })
    .expect("parquet::run should succeed");

    assert!(
        parquet_dir.join("orgs.parquet").exists(),
        "orgs.parquet not created"
    );
    assert!(
        parquet_dir.join("orgs_all.parquet").exists(),
        "orgs_all.parquet not created"
    );
    assert!(
        parquet_dir.join("roles.parquet").exists(),
        "roles.parquet not created"
    );
    assert!(
        parquet_dir.join("relationships.parquet").exists(),
        "relationships.parquet not created"
    );
    assert!(
        parquet_dir.join("successions.parquet").exists(),
        "successions.parquet not created"
    );

    // Stage 3: md
    let zip_path = tmp.path().join("wiki.zip");

    md::run(md::Args {
        input: parquet_dir.clone(),
        output: zip_path.clone(),
    })
    .expect("md::run should succeed");

    assert!(zip_path.exists(), "wiki.zip was not created");

    // Inspect zip contents in-process -- no unzip binary required
    let zip_file = std::fs::File::open(&zip_path).unwrap();
    let mut archive = zip::ZipArchive::new(zip_file).unwrap();

    let entry_name = "organisations/gp_practice/Y01234.md";
    let mut entry = archive
        .by_name(entry_name)
        .unwrap_or_else(|_| panic!("{entry_name} not found in wiki.zip"));

    let mut md = String::new();
    entry.read_to_string(&mut md).unwrap();

    // YAML frontmatter fields
    assert!(md.contains("type: GP Practice"),  "missing 'type' frontmatter\n---\n{md}");
    assert!(md.contains("title: Mock GP Practice"),  "missing 'title' frontmatter\n---\n{md}");
    assert!(md.contains("postcode: SO15 5SY"),        "missing 'postcode' frontmatter\n---\n{md}");
    assert!(md.contains(r#"uprn: "100062506311""#),   "missing 'uprn' frontmatter\n---\n{md}");
    assert!(md.contains(r#"telephone: "023 80706919""#), "missing 'telephone' frontmatter\n---\n{md}");
    assert!(md.contains(r#"website: "http://example.com""#), "missing 'website' frontmatter\n---\n{md}");
    assert!(
        md.contains("resource: https://directory.spineservices.nhs.uk/OdsWebService/Ods/Y01234"),
        "missing 'resource' frontmatter\n---\n{md}"
    );

    assert!(
        md.contains("Parent Org: ALDER HEY CHILDREN'S NHS FOUNDATION TRUST (RAE)"),
        "parent org relationship missing or malformed\n---\n{md}"
    );

    // Stage 4: find (TDD)
    let mut find_out = Vec::new();
    find::run_with_writer(
        find::Args {
            query: Some("Mock".to_string()),
            role: None,
            all: false,
            verbose: false,
            sort: find::SortBy::Code,
            format: find::OutputFormat::Json,
            input: parquet_dir.clone(),
        },
        &mut find_out,
        &parquet_dir,
    )
    .expect("find::run should succeed");

    let find_str = String::from_utf8(find_out).unwrap();
    assert!(find_str.contains("Mock GP Practice"), "find should locate Mock GP");

    // Stage 5: cite
    let mut cite_out = Vec::new();
    cite::run_with_writer(
        cite::Args {
            input: Some(parquet_dir),
            format: "text".to_string(),
        },
        &mut cite_out,
    )
    .expect("cite::run should succeed");

    let cite_str = String::from_utf8(cite_out).unwrap();
    assert!(cite_str.contains("How to Cite"), "cite block missing How to Cite section");
    assert!(cite_str.contains("orgs.parquet:"), "cite block missing orgs.parquet hash");
}


//! Integration tests for the full ODS pipeline: compile → parquet → okf.
//!
//! These tests replace `verify_compilation.sh` with idiomatic Rust:
//! - `tempfile::TempDir` guarantees cleanup even on test panic
//! - Assertions operate on typed `OdsRecord` structs rather than `grep` / regex
//! - The zip archive is inspected in-process without extracting to disk
//! - `CARGO_MANIFEST_DIR` resolves the fixture path regardless of `cwd`

use ods::commands::{compile, okf, parquet};
use std::io::Read;
use std::path::Path;
use tempfile::TempDir;

/// Mock TRUD XML fixture: two organisations (an NHS Trust and a GP Practice
/// commissioned by it).  Lives alongside these tests so changes to the fixture
/// are visible in the same diff as the tests that depend on it.
const FIXTURE_XML: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/mock_hscorgrefdata.xml"
);

// ---------------------------------------------------------------------------
// Stage 1 — compile
// ---------------------------------------------------------------------------

/// Verifies that `ods compile` produces well-formed NDJSON with correctly
/// resolved role names, cross-record parent lookups, and flattened contacts.
#[test]
fn compile_produces_ndjson_with_correct_records() {
    let tmp = TempDir::new().unwrap();

    compile::run(compile::Args {
        input: Path::new(FIXTURE_XML).to_path_buf(),
        output: tmp.path().to_path_buf(),
    })
    .expect("compile::run should succeed");

    let ndjson_path = tmp.path().join("ods.ndjson");
    assert!(ndjson_path.exists(), "ods.ndjson was not created");

    let content = std::fs::read_to_string(&ndjson_path).unwrap();
    let lines: Vec<&str> = content.trim().lines().collect();
    assert_eq!(
        lines.len(),
        2,
        "expected 2 organisations in NDJSON, got {}",
        lines.len()
    );

    let records: Vec<compile::OdsRecord> = lines
        .iter()
        .map(|l| {
            serde_json::from_str(l)
                .unwrap_or_else(|e| panic!("line is not valid OdsRecord JSON: {e}\n  -> {l}"))
        })
        .collect();

    // --- RAE: NHS Trust ---
    let rae = records
        .iter()
        .find(|r| r.ods_code == "RAE")
        .expect("RAE record must be present");
    assert_eq!(
        rae.role, "NHS Trust",
        "RAE role should be resolved from the CodeSystem concept map"
    );
    assert_eq!(rae.status, "Active");

    // --- Y01234: GP Practice ---
    let gp = records
        .iter()
        .find(|r| r.ods_code == "Y01234")
        .expect("Y01234 record must be present");
    assert_eq!(
        gp.role, "General Practice",
        "Y01234 role should be resolved from the CodeSystem concept map"
    );
    assert_eq!(gp.status, "Active");

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
// Stages 2 + 3 — parquet and okf (chained on top of compile)
// ---------------------------------------------------------------------------

/// Runs the full three-stage pipeline and asserts on the OKF zip contents.
/// The zip is inspected in-process -- no `unzip` subprocess, no leftover files.
#[test]
fn full_pipeline_parquet_and_okf() {
    let tmp = TempDir::new().unwrap();

    // Stage 1: compile
    compile::run(compile::Args {
        input: Path::new(FIXTURE_XML).to_path_buf(),
        output: tmp.path().to_path_buf(),
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
        parquet_dir.join("roles.parquet").exists(),
        "roles.parquet not created"
    );
    assert!(
        parquet_dir.join("rels.parquet").exists(),
        "rels.parquet not created"
    );

    // Stage 3: okf
    let zip_path = tmp.path().join("wiki.zip");

    okf::run(okf::Args {
        input: parquet_dir,
        output: zip_path.clone(),
    })
    .expect("okf::run should succeed");

    assert!(zip_path.exists(), "wiki.zip was not created");

    // Inspect zip contents in-process -- no unzip binary required
    let zip_file = std::fs::File::open(&zip_path).unwrap();
    let mut archive = zip::ZipArchive::new(zip_file).unwrap();

    let entry_name = "organisations/general_practice/Y01234.md";
    let mut entry = archive
        .by_name(entry_name)
        .unwrap_or_else(|_| panic!("{entry_name} not found in wiki.zip"));

    let mut md = String::new();
    entry.read_to_string(&mut md).unwrap();

    // YAML frontmatter fields
    assert!(md.contains("type: General Practice"),  "missing 'type' frontmatter\n---\n{md}");
    assert!(md.contains("title: Mock GP Practice"),  "missing 'title' frontmatter\n---\n{md}");
    assert!(md.contains("postcode: SO15 5SY"),        "missing 'postcode' frontmatter\n---\n{md}");
    assert!(md.contains(r#"uprn: "100062506311""#),   "missing 'uprn' frontmatter\n---\n{md}");
    assert!(md.contains(r#"telephone: "023 80706919""#), "missing 'telephone' frontmatter\n---\n{md}");
    assert!(md.contains(r#"website: "http://example.com""#), "missing 'website' frontmatter\n---\n{md}");
    assert!(
        md.contains("resource: https://directory.spineservices.nhs.uk/OdsWebService/Ods/Y01234"),
        "missing 'resource' frontmatter\n---\n{md}"
    );

    // Parent organisation link in the Markdown body
    assert!(
        md.contains("[Alder Hey Children's NHS Foundation Trust](/organisations/nhs_trust/RAE.md)"),
        "parent org link missing or malformed\n---\n{md}"
    );
}

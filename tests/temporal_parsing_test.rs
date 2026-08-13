//! Regression guards for ODS temporal data and role/relationship status.
//!
//! ODS expresses dates and status as *child elements*, not attributes:
//!
//! ```xml
//! <Role id="RO180" uniqueRoleId="5785" primaryRole="true">
//!   <Date><Type value="Legal"/><Start value="2006-10-01"/><End value="2013-03-31"/></Date>
//!   <Status value="Inactive"/>
//! </Role>
//! ```
//!
//! The parser previously looked for `type=` and `status=` *attributes*, which
//! never exist, so every date in the source was silently discarded and every
//! role and relationship was reported `active`.

use ods::commands::{ndjson, parquet};
use std::path::Path;
use tempfile::TempDir;

const FIXTURE_XML: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/mock_temporal.xml"
);

fn parse_fixture() -> Vec<ndjson::OdsRecord> {
    let tmp = TempDir::new().unwrap();
    let out = tmp.path().join("ods.ndjson");

    ndjson::run(ndjson::Args {
        input: Path::new(FIXTURE_XML).to_path_buf(),
        output: Some(out.clone()),
    })
    .expect("ndjson::run should succeed");

    std::fs::read_to_string(&out)
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<ndjson::OdsRecord>(l).ok())
        .filter(|r: &ndjson::OdsRecord| !r.ods_code.is_empty())
        .collect()
}

fn record<'a>(records: &'a [ndjson::OdsRecord], code: &str) -> &'a ndjson::OdsRecord {
    records
        .iter()
        .find(|r| r.ods_code == code)
        .unwrap_or_else(|| panic!("fixture record {code} not found"))
}

fn date_of<'a>(dates: &'a [ndjson::OdsDate], kind: &str) -> &'a ndjson::OdsDate {
    dates
        .iter()
        .find(|d| d.date_type.eq_ignore_ascii_case(kind))
        .unwrap_or_else(|| panic!("no {kind} date found in {dates:?}"))
}

/// Organisation-level `<Date>` children must be captured, and Legal and
/// Operational intervals must be kept apart — ODS routinely disagrees between
/// them, which is the whole reason both are published.
#[test]
fn organisation_dates_are_parsed_and_legal_differs_from_operational() {
    let records = parse_fixture();
    let org = record(&records, "TMP01");

    assert!(
        !org.dates.is_empty(),
        "organisation dates were discarded entirely"
    );

    let legal = date_of(&org.dates, "Legal");
    let operational = date_of(&org.dates, "Operational");

    assert_eq!(legal.start.as_deref(), Some("2006-10-01"));
    assert_eq!(legal.end.as_deref(), Some("2013-03-31"));
    assert_eq!(operational.start.as_deref(), Some("2006-10-02"));
    assert_eq!(operational.end.as_deref(), Some("2020-09-30"));

    assert_ne!(
        legal.start, operational.start,
        "Legal and Operational intervals must be captured separately"
    );
}

/// Role-level `<Date>` children must be captured, including open-ended ranges.
#[test]
fn role_dates_are_parsed() {
    let records = parse_fixture();
    let org = record(&records, "TMP01");

    let primary = org
        .roles
        .iter()
        .find(|r| r.id == "RO177")
        .expect("RO177 role missing");

    assert!(!primary.dates.is_empty(), "role dates were discarded");

    let operational = date_of(&primary.dates, "Operational");
    assert_eq!(operational.start.as_deref(), Some("2006-10-01"));
    assert_eq!(
        operational.end, None,
        "an open-ended role must not invent an end date"
    );
}

/// Role status comes from a `<Status>` child element. Previously every role was
/// hardcoded to `active`, so this asserted value was impossible to produce.
#[test]
fn role_status_is_read_from_child_element() {
    let records = parse_fixture();
    let org = record(&records, "TMP02");

    let role = org
        .roles
        .iter()
        .find(|r| r.id == "RO180")
        .expect("RO180 role missing");

    assert!(
        role.status.eq_ignore_ascii_case("inactive"),
        "role status should be Inactive, got {:?}",
        role.status
    );

    let legal = date_of(&role.dates, "Legal");
    let operational = date_of(&role.dates, "Operational");
    assert_eq!(legal.end.as_deref(), Some("2013-03-31"));
    assert_eq!(operational.end.as_deref(), Some("2020-09-30"));
}

/// An inactive role must not drag the organisation inactive with it.
#[test]
fn role_status_does_not_leak_to_organisation() {
    let records = parse_fixture();
    let org = record(&records, "TMP02");

    assert!(
        org.status.eq_ignore_ascii_case("active"),
        "organisation status should stay Active, got {:?}",
        org.status
    );
}

/// Relationships carry the same `<Date>` / `<Status>` child elements as roles
/// and were broken in exactly the same way.
#[test]
fn relationship_dates_and_status_are_parsed() {
    let records = parse_fixture();
    let org = record(&records, "TMP02");

    let rel = org
        .relationships
        .first()
        .expect("TMP02 should have one relationship");

    assert!(
        rel.status.eq_ignore_ascii_case("inactive"),
        "relationship status should be Inactive, got {:?}",
        rel.status
    );

    let operational = date_of(&rel.dates, "Operational");
    assert_eq!(operational.start.as_deref(), Some("2013-04-01"));
    assert_eq!(operational.end.as_deref(), Some("2022-06-30"));
}

/// The same role code can be held twice over non-overlapping periods. They are
/// distinct role *instances*, distinguished by uniqueRoleId — not duplicates.
#[test]
fn repeated_role_codes_are_distinct_instances() {
    let records = parse_fixture();
    let org = record(&records, "TMP03");

    let repeated: Vec<_> = org.roles.iter().filter(|r| r.id == "RO270").collect();
    assert_eq!(
        repeated.len(),
        2,
        "expected two RO270 instances, got {}",
        repeated.len()
    );

    let mut ids: Vec<&str> = repeated.iter().map(|r| r.unique_role_id.as_str()).collect();
    ids.sort();
    assert_eq!(ids, vec!["4002", "4003"], "role instances must be distinguishable");

    let closed = repeated
        .iter()
        .find(|r| r.unique_role_id == "4002")
        .unwrap();
    let current = repeated
        .iter()
        .find(|r| r.unique_role_id == "4003")
        .unwrap();

    assert!(closed.status.eq_ignore_ascii_case("inactive"));
    assert!(current.status.eq_ignore_ascii_case("active"));
    assert_eq!(
        date_of(&closed.dates, "Operational").end.as_deref(),
        Some("2015-12-31")
    );
    assert_eq!(date_of(&current.dates, "Operational").end, None);
}

/// A `<Succ>` carries two sibling elements both named `<Type>`:
/// `<Date><Type value="Legal"/></Date>` (the date's kind) and
/// `<Type>Predecessor</Type>` (the succession's kind). Conflating them would
/// silently corrupt either succession direction or succession dates.
#[test]
fn successor_date_type_is_not_confused_with_succession_type() {
    let records = parse_fixture();
    let org = record(&records, "TMP02");

    let succ = org
        .successors
        .first()
        .expect("TMP02 should have one successor entry");

    assert!(
        succ.succ_type.eq_ignore_ascii_case("predecessor"),
        "succession type should come from <Type>Predecessor</Type>, got {:?}",
        succ.succ_type
    );
    assert_eq!(succ.target.ods_code, "TMP01");

    let legal = date_of(&succ.dates, "Legal");
    assert_eq!(legal.start.as_deref(), Some("2006-10-01"));
    assert_eq!(legal.end, None);
}

/// End-to-end: dates must survive into the Parquet projection, not just NDJSON.
#[test]
fn parquet_projection_populates_date_columns() {
    let tmp = TempDir::new().unwrap();
    let out_dir = tmp.path().join("parquet_out");

    parquet::run(parquet::Args {
        input: Path::new(FIXTURE_XML).to_path_buf(),
        output: out_dir.clone(),
    })
    .expect("parquet::run should succeed");

    assert!(
        non_null_count(&out_dir.join("orgs.parquet"), "operational_start") > 0,
        "orgs.parquet operational_start is entirely null"
    );
    assert!(
        non_null_count(&out_dir.join("orgs.parquet"), "legal_start") > 0,
        "orgs.parquet legal_start is entirely null"
    );
    // Role dates live on the org<->role bridge, not the vocabulary table.
    assert!(
        non_null_count(&out_dir.join("org_roles.parquet"), "operational_start") > 0,
        "org_roles.parquet operational_start is entirely null"
    );
    assert!(
        non_null_count(&out_dir.join("org_roles.parquet"), "operational_end") > 0,
        "org_roles.parquet operational_end is entirely null"
    );
}

/// Counts non-null values in a Parquet column.
fn non_null_count(path: &Path, column: &str) -> usize {
    // `parquet` at module scope is `ods::commands::parquet`, so reach the
    // external crate explicitly.
    use ::parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    use arrow::record_batch::RecordBatch;

    let file = std::fs::File::open(path)
        .unwrap_or_else(|e| panic!("opening {}: {e}", path.display()));
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .unwrap()
        .build()
        .unwrap();

    let mut total = 0;
    for batch in reader {
        let batch: RecordBatch = batch.unwrap();
        let idx = match batch.schema().index_of(column) {
            Ok(i) => i,
            Err(_) => panic!("column {column} not present in {}", path.display()),
        };
        let col = batch.column(idx);
        total += col.len() - col.null_count();
    }
    total
}

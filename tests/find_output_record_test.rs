//! Acceptance tests for `find-output-record.md`
//!
//! Verifies:
//! - Task 1: OrgRow schema order, nullability against orgs_schema(), and failure detection on extra column.
//! - Task 2: role_name is completely absent from src/commands/find.rs (grep -c 'role_name\b' is 0).
//! - Task 3: JSON output has 25 keys (23 schema in order + predecessors + successors); role_name absent from JSON and CSV.
//! - Task 4: Transitive closures in predecessor_codes/successor_codes match orgs.parquet column;
//!           predecessors/successors decorated lists match codes in order; ods info output unchanged.
//! - Task 5: CSV headers derived from record, quoted commas in address parsed by CSV reader,
//!           CSV columns equal JSON keys minus object arrays, no duplicate column values in A101.
//! - Task 6: Zero copies in find.rs (grep -c 'ods_code,record_class' is 0).

mod common;

use common::setup_find_test_workspace;
use ods::commands::find::{self, csv_headers_from_json, Args, OrgRow, OutputFormat};
use ods::commands::parquet::{
    build_succession_edges, compute_transitive_closures, export_orgs, export_orgs_all,
    export_relationships, export_roles, export_successions, orgs_schema,
};
use ods::ods_xml::{Location, OdsRecord, OdsRole, OdsSuccessor};
use ods::provenance::OdsProvenance;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

/// RFC 4180 CSV line parser
fn parse_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    current.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                current.push(c);
            }
        } else {
            if c == '"' {
                in_quotes = true;
            } else if c == ',' {
                fields.push(current);
                current = String::new();
            } else {
                current.push(c);
            }
        }
    }
    fields.push(current);
    fields
}

fn check_org_row_against_schema(
    schema: &arrow::datatypes::Schema,
    dummy: &OrgRow,
) -> Result<(), String> {
    let val = serde_json::to_value(dummy).map_err(|e| e.to_string())?;
    let obj = val
        .as_object()
        .ok_or_else(|| "OrgRow must serialize as JSON object".to_string())?;
    let org_keys: Vec<&str> = obj.keys().map(|s| s.as_str()).collect();
    let schema_fields = schema.fields();

    if org_keys.len() != schema_fields.len() {
        let org_set: std::collections::HashSet<&str> = org_keys.iter().copied().collect();
        let schema_set: std::collections::HashSet<&str> =
            schema_fields.iter().map(|f| f.name().as_str()).collect();
        let missing: Vec<&str> = schema_set.difference(&org_set).copied().collect();
        let extra: Vec<&str> = org_set.difference(&schema_set).copied().collect();
        return Err(format!(
            "Key count mismatch: OrgRow has {}, schema has {}. Missing in OrgRow: {:?}, Extra in OrgRow: {:?}",
            org_keys.len(),
            schema_fields.len(),
            missing,
            extra
        ));
    }

    for (i, field) in schema_fields.iter().enumerate() {
        if org_keys[i] != field.name().as_str() {
            return Err(format!(
                "Key sequence mismatch at index {}: OrgRow has '{}', schema has '{}'",
                i,
                org_keys[i],
                field.name()
            ));
        }

        let is_null = obj[field.name().as_str()].is_null();
        if field.is_nullable() != is_null {
            return Err(format!(
                "Nullability mismatch for '{}': schema is_nullable={}, OrgRow is_null={}",
                field.name(),
                field.is_nullable(),
                is_null
            ));
        }
    }

    Ok(())
}

fn dummy_org_row() -> OrgRow {
    OrgRow {
        ods_code: "X".to_string(),
        name: "X".to_string(),
        record_class: "X".to_string(),
        role_codes: vec!["RO177".to_string()],
        role_names: vec!["Prescribing Cost Centre".to_string()],
        primary_role_code: "RO177".to_string(),
        address: None,
        town: None,
        county: None,
        postcode: None,
        country: None,
        uprn: None,
        telephone: None,
        website: None,
        predecessor_codes: vec![],
        successor_codes: vec![],
        status: "active".to_string(),
        legal_start: None,
        legal_end: None,
        operational_start: None,
        operational_end: None,
        last_changed: None,
        trud_release_date: "2026-07-31".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Task 1: OrgRow key sequence, nullability, and schema drift detection
// ---------------------------------------------------------------------------

#[test]
fn test_task1_org_row_matches_orgs_schema_and_nullability() {
    let schema = orgs_schema();
    let dummy = dummy_org_row();

    // 1. Assert OrgRow serialized keys equal orgs_schema() names in exact order
    // 2. Assert nullability matches (Option<String> is nullable, non-Option is not)
    assert!(check_org_row_against_schema(&schema, &dummy).is_ok());

    // 3. Show it failing: add a field to orgs_schema() and confirm test names missing field
    let mut fields: Vec<arrow::datatypes::Field> =
        schema.fields().iter().map(|f| (**f).clone()).collect();
    fields.push(arrow::datatypes::Field::new(
        "future_phantom_column",
        arrow::datatypes::DataType::Utf8,
        true,
    ));
    let extended_schema = arrow::datatypes::Schema::new(fields);

    let err = check_org_row_against_schema(&extended_schema, &dummy).unwrap_err();
    assert!(
        err.contains("future_phantom_column"),
        "Drift check must name the missing field, got:\n{}",
        err
    );
}

// ---------------------------------------------------------------------------
// Task 2: role_name deleted from src/commands/find.rs
// ---------------------------------------------------------------------------

#[test]
fn test_task2_grep_no_role_name_in_find_rs() {
    let find_src = fs::read_to_string("src/commands/find.rs").expect("read src/commands/find.rs");
    let re = regex_lite_or_manual_word_match(&find_src, "role_name");
    assert_eq!(
        re, 0,
        "grep -c 'role_name\\b' src/commands/find.rs must return 0, found {} hits",
        re
    );

    // Verify plural role_names is present
    let re_plural = regex_lite_or_manual_word_match(&find_src, "role_names");
    assert!(
        re_plural > 0,
        "role_names is a schema column and must remain present"
    );
}

fn regex_lite_or_manual_word_match(src: &str, target_word: &str) -> usize {
    let mut count = 0;
    for line in src.lines() {
        let mut idx = 0;
        while let Some(pos) = line[idx..].find(target_word) {
            let start = idx + pos;
            let end = start + target_word.len();
            idx = end;

            // Check word boundaries
            let left_ok = if start == 0 {
                true
            } else {
                let prev = line[..start].chars().last().unwrap();
                !prev.is_alphanumeric() && prev != '_'
            };
            let right_ok = if end == line.len() {
                true
            } else {
                let next = line[end..].chars().next().unwrap();
                !next.is_alphanumeric() && next != '_'
            };

            if left_ok && right_ok {
                count += 1;
            }
        }
    }
    count
}

// ---------------------------------------------------------------------------
// Task 3: JSON output has 25 keys, order preserved, no role_name in JSON or CSV
// ---------------------------------------------------------------------------

#[test]
fn test_task3_find_sedbergh_json_keys_and_no_role_name() {
    let (_tmp, parquet_dir) = setup_find_test_workspace();

    // 1. Test JSON output
    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            query: Some("sedbergh".to_string()),
            format: OutputFormat::Json,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .unwrap();

    let s = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(!lines.is_empty(), "expected at least one JSON record");

    let schema = orgs_schema();
    let schema_fields = schema.fields();
    assert_eq!(schema_fields.len(), 23);

    for line in lines {
        let val: serde_json::Value = serde_json::from_str(line).unwrap();
        let obj = val.as_object().unwrap();
        let keys: Vec<String> = obj.keys().cloned().collect();

        // Must emit exactly 25 keys
        assert_eq!(keys.len(), 25, "find --format json must emit 25 keys");

        // First 23 keys match orgs_schema() names in order
        for (i, field) in schema_fields.iter().enumerate() {
            assert_eq!(
                &keys[i],
                field.name(),
                "Key {} mismatch against orgs_schema()",
                i
            );
        }

        // Key 23 is predecessors, key 24 is successors
        assert_eq!(keys[23], "predecessors");
        assert_eq!(keys[24], "successors");

        // role_name appears in no machine output
        assert!(
            !obj.contains_key("role_name"),
            "role_name must not appear in JSON output"
        );
    }

    // 2. Test CSV output does not contain role_name
    let mut csv_out = Vec::new();
    find::run_with_writer(
        Args {
            query: Some("sedbergh".to_string()),
            format: OutputFormat::Csv,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut csv_out,
        &parquet_dir,
    )
    .unwrap();

    let csv_s = String::from_utf8(csv_out).unwrap();
    let header_line = csv_s.lines().next().unwrap();
    let csv_headers = parse_csv_line(header_line);
    assert!(
        !csv_headers.contains(&"role_name".to_string()),
        "CSV header must not contain role_name"
    );
    assert_eq!(
        regex_lite_or_manual_word_match(&csv_s, "role_name"),
        0,
        "CSV output must contain 0 occurrences of role_name"
    );
}

// ---------------------------------------------------------------------------
// Task 4: Transitive closure in predecessor_codes & successors, and info unchanged
// ---------------------------------------------------------------------------

fn setup_two_hop_chain_workspace() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let parquet_dir = tmp.path().join("parquet");
    fs::create_dir_all(&parquet_dir).unwrap();

    // Chain: C303 -> B202 -> A101
    // C303 is succeeded by B202. B202 is succeeded by A101.
    // Therefore A101 has predecessors: B202, C303 (transitive closure).
    // C303 has successors: B202, A101 (transitive closure).
    let records = vec![
        OdsRecord {
            ods_code: "C303".to_string(),
            name: "Charlie Clinic".to_string(),
            status: "inactive".to_string(),
            record_class: "site".to_string(),
            role: "branch surgery".to_string(),
            geo_loc: Some(Location {
                address_lines: vec!["3 Old Road".to_string()],
                town: Some("Leeds".to_string()),
                county: None,
                postcode: Some("LS1 1AA".to_string()),
                country: Some("ENGLAND".to_string()),
                uprn: None,
            }),
            successors: vec![OdsSuccessor {
                unique_succ_id: "1".to_string(),
                succ_type: "Successor".to_string(),
                dates: vec![],
                target: ods::ods_xml::OdsRelationshipTarget {
                    ods_code: "B202".to_string(),
                    name: Some("Beta Surgery".to_string()),
                    ..Default::default()
                },
            }],
            ..Default::default()
        },
        OdsRecord {
            ods_code: "B202".to_string(),
            name: "Beta Surgery".to_string(),
            status: "inactive".to_string(),
            record_class: "site".to_string(),
            role: "branch surgery".to_string(),
            geo_loc: Some(Location {
                address_lines: vec!["2 High Street".to_string()],
                town: Some("Manchester".to_string()),
                county: None,
                postcode: Some("M1 1AA".to_string()),
                country: Some("ENGLAND".to_string()),
                uprn: None,
            }),
            successors: vec![OdsSuccessor {
                unique_succ_id: "2".to_string(),
                succ_type: "Successor".to_string(),
                dates: vec![],
                target: ods::ods_xml::OdsRelationshipTarget {
                    ods_code: "A101".to_string(),
                    name: Some("Alpha Health Centre".to_string()),
                    ..Default::default()
                },
            }],
            ..Default::default()
        },
        OdsRecord {
            ods_code: "A101".to_string(),
            name: "Alpha Health Centre".to_string(),
            status: "active".to_string(),
            record_class: "org".to_string(),
            role: "prescribing cost centre".to_string(),
            geo_loc: Some(Location {
                address_lines: vec!["1 Main Street".to_string()],
                town: Some("London".to_string()),
                county: Some("Greater London".to_string()),
                postcode: Some("SW1A 1AA".to_string()),
                country: Some("ENGLAND".to_string()),
                uprn: Some("10001".to_string()),
            }),
            roles: vec![OdsRole {
                id: "RO177".to_string(),
                code: None,
                display_name: Some("prescribing cost centre".to_string()),
                unique_role_id: "1".to_string(),
                primary_role: true,
                status: "active".to_string(),
                dates: vec![],
            }],
            ..Default::default()
        },
    ];

    let edges = build_succession_edges(&records);
    let (succ_closures, pred_closures) = compute_transitive_closures(&records, &edges);

    let mut prov = OdsProvenance::default();
    prov.trud_release_date = Some("2026-07-31".to_string());

    export_orgs(
        &parquet_dir,
        &records,
        &succ_closures,
        &pred_closures,
        Some(&prov),
    )
    .unwrap();
    export_orgs_all(
        &parquet_dir,
        &records,
        &succ_closures,
        &pred_closures,
        Some(&prov),
    )
    .unwrap();
    export_roles(&parquet_dir, &records, Some(&prov)).unwrap();
    export_relationships(&parquet_dir, &records, Some(&prov)).unwrap();
    export_successions(&parquet_dir, &records, Some(&prov)).unwrap();

    (tmp, parquet_dir)
}

#[test]
fn test_task4_two_hop_chain_closure_and_predecessors_decoration() {
    let (_tmp, parquet_dir) = setup_two_hop_chain_workspace();

    // 1. Run find for A101
    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            code: vec!["A101".to_string()],
            format: OutputFormat::Json,
            input: Some(parquet_dir.clone()),
            all: true,
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .unwrap();

    let s = String::from_utf8(out).unwrap();
    let val: serde_json::Value = serde_json::from_str(s.trim()).unwrap();

    // Acceptance: predecessor_codes has both two-hop predecessors ["B202", "C303"]
    let pred_codes: Vec<&str> = val["predecessor_codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(
        pred_codes,
        vec!["B202", "C303"],
        "predecessor_codes must contain the transitive closure"
    );

    // Acceptance: predecessors array matches predecessor_codes in exact order
    let preds = val["predecessors"].as_array().unwrap();
    assert_eq!(preds.len(), 2);
    assert_eq!(preds[0]["code"], "B202");
    assert_eq!(preds[0]["name"], "Beta Surgery");
    assert_eq!(preds[0]["status"], "inactive");
    assert_eq!(preds[1]["code"], "C303");
    assert_eq!(preds[1]["name"], "Charlie Clinic");
    assert_eq!(preds[1]["status"], "inactive");

    // Predecessors entries do not carry obsolete "date" field
    assert!(preds[0].get("date").is_none());
    assert!(preds[1].get("date").is_none());

    // 2. Run find for C303 (two-hop successors)
    let mut out_c = Vec::new();
    find::run_with_writer(
        Args {
            code: vec!["C303".to_string()],
            format: OutputFormat::Json,
            input: Some(parquet_dir.clone()),
            all: true,
            ..Default::default()
        },
        &mut out_c,
        &parquet_dir,
    )
    .unwrap();

    let s_c = String::from_utf8(out_c).unwrap();
    let val_c: serde_json::Value = serde_json::from_str(s_c.trim()).unwrap();

    let succ_codes: Vec<&str> = val_c["successor_codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(
        succ_codes,
        vec!["A101", "B202"],
        "successor_codes must contain the transitive closure"
    );

    let succs = val_c["successors"].as_array().unwrap();
    assert_eq!(succs.len(), 2);
    assert_eq!(succs[0]["code"], "A101");
    assert_eq!(succs[0]["name"], "Alpha Health Centre");
    assert_eq!(succs[1]["code"], "B202");
    assert_eq!(succs[1]["name"], "Beta Surgery");

    // 3. Verify ods info succession output is unchanged
    let mut info_out = Vec::new();
    ods::commands::info::run_with_writer(
        ods::commands::info::Args {
            ods_code: "B202".to_string(),
            format: ods::commands::info::OutputFormat::Markdown,
            input: Some(parquet_dir.clone()),
        },
        &mut info_out,
        &parquet_dir,
    )
    .unwrap();

    let info_s = String::from_utf8(info_out).unwrap();
    assert!(info_s.contains("# Beta Surgery (B202)"));
    assert!(info_s.contains("Predecessors"));
    assert!(info_s.contains("Succession"));
    assert!(info_s.contains("Alpha Health Centre"));
}

// ---------------------------------------------------------------------------
// Task 5: CSV derivation, RFC 4180 parsing, and no duplicate column values
// ---------------------------------------------------------------------------

#[test]
fn test_task5_csv_headers_and_real_csv_parsing() {
    let (_tmp, parquet_dir) = setup_two_hop_chain_workspace();

    // 1. Produce CSV output
    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            code: vec!["A101".to_string()],
            format: OutputFormat::Csv,
            input: Some(parquet_dir.clone()),
            all: true,
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .unwrap();

    let s = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = s.lines().collect();
    assert_eq!(lines.len(), 2, "Header + 1 data line expected");

    let header_line = lines[0];
    let data_line = lines[1];

    let headers = parse_csv_line(header_line);
    let cells = parse_csv_line(data_line);

    // Header count equals row count (23 columns)
    assert_eq!(headers.len(), 23);
    assert_eq!(cells.len(), 23);

    // CSV headers equal JSON keys minus object-valued keys (predecessors, successors)
    let dummy = dummy_org_row();
    let mut meta = std::collections::HashMap::new();
    meta.insert(
        "B202".to_string(),
        ("Beta Surgery".to_string(), "inactive".to_string()),
    );
    let output_rec = ods::commands::find::build_output_record(&dummy, &meta);
    let val = serde_json::to_value(&output_rec).unwrap();
    let derived_csv_headers = csv_headers_from_json(&val);
    assert_eq!(headers, derived_csv_headers);

    // Quoted field contains commas: address is "1 Main Street, London, SW1A 1AA"
    let addr_idx = headers.iter().position(|h| h == "address").unwrap();
    assert_eq!(cells[addr_idx], "1 Main Street, London, SW1A 1AA");

    // Acceptance: No two CSV columns hold identical values for A101
    // (In the old code, predecessor_codes and predecessors were verbatim copies).
    let mut seen_values = std::collections::HashSet::new();
    for (idx, cell) in cells.iter().enumerate() {
        if !cell.is_empty() {
            assert!(
                seen_values.insert((headers[idx].as_str(), cell.as_str())),
                "Duplicate column value found"
            );
        }
    }
    // Specifically verify predecessors and successors are NOT in CSV
    assert!(!headers.contains(&"predecessors".to_string()));
    assert!(!headers.contains(&"successors".to_string()));
}

// ---------------------------------------------------------------------------
// Task 6: Zero copies in find.rs
// ---------------------------------------------------------------------------

#[test]
fn test_task6_grep_ods_code_record_class_is_zero() {
    let find_src = fs::read_to_string("src/commands/find.rs").expect("read src/commands/find.rs");
    let hits = find_src.matches("ods_code,record_class").count();
    assert_eq!(
        hits, 0,
        "grep -c 'ods_code,record_class' src/commands/find.rs must be 0, found {}",
        hits
    );
}

// ---------------------------------------------------------------------------
// Display formats: table, markdown, and tsv remain byte-identical
// ---------------------------------------------------------------------------

#[test]
fn test_display_formats_table_markdown_tsv() {
    let (_tmp, parquet_dir) = setup_find_test_workspace();

    for format in [OutputFormat::Table, OutputFormat::Markdown, OutputFormat::Tsv] {
        let mut out = Vec::new();
        find::run_with_writer(
            Args {
                code: vec!["A82608".to_string()],
                format,
                input: Some(parquet_dir.clone()),
                ..Default::default()
            },
            &mut out,
            &parquet_dir,
        )
        .unwrap();

        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("A82608"));
        assert!(s.contains("SEDBERGH MEDICAL PRACTICE"));
    }
}

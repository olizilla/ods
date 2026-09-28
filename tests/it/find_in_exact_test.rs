use crate::common;

use std::path::PathBuf;
use tempfile::TempDir;

use ods::commands::find::{self, Args, OutputFormat, SortBy};
use ods::commands::parquet::{
    build_succession_edges, compute_transitive_closures, export_orgs,
    export_relationships, export_roles, export_successions,
};
use ods::ods_xml::{Location, OdsRecord, OdsRole};



#[allow(clippy::too_many_arguments)]
fn make_test_record(
    ods_code: &str,
    name: &str,
    town: Option<&str>,
    county: Option<&str>,
    country: Option<&str>,
    postcode: Option<&str>,
    role_code: &str,
    role_name: &str,
) -> OdsRecord {
    OdsRecord {
        ods_code: ods_code.to_string(),
        name: name.to_string(),
        status: "active".to_string(),
        record_class: "org".to_string(),
        role: role_name.to_string(),
        geo_loc: Some(Location {
            address_lines: vec!["1 Test Street".to_string()],
            town: town.map(|s| s.to_string()),
            county: county.map(|s| s.to_string()),
            country: country.map(|s| s.to_string()),
            postcode: postcode.map(|s| s.to_string()),
            uprn: None,
        }),
        roles: vec![OdsRole {
            id: role_code.to_string(),
            code: Some(role_code.to_string()),
            display_name: Some(role_name.to_string()),
            unique_role_id: format!("U_{}", ods_code),
            primary_role: true,
            status: "active".to_string(),
            dates: vec![],
        }],
        ..Default::default()
    }
}

fn setup_exact_in_workspace() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().expect("create temp dir");
    let parquet_dir = tmp.path().join("parquet");
    std::fs::create_dir_all(&parquet_dir).expect("create parquet dir");

    let records = vec![
        // 1. LONDON / LONDONDERRY
        make_test_record("L1", "ST BARTS LONDON", Some("LONDON"), Some("GREATER LONDON"), Some("ENGLAND"), Some("EC1A 7BE"), "RO76", "GP Practice"),
        make_test_record("L2", "LONDONDERRY HOSPITAL", Some("LONDONDERRY"), Some("COUNTY LONDONDERRY"), Some("NORTHERN IRELAND"), Some("BT48 7SX"), "RO76", "GP Practice"),
        make_test_record("L3", "CITY SURGERY LONDON", Some("LONDON"), Some("LONDON"), Some("ENGLAND"), Some("EC2 1AA"), "RO76", "GP Practice"),
        make_test_record("L4", "LONDON PHARMACY", Some("LONDON"), Some("GREATER LONDON"), Some("ENGLAND"), Some("WC1 1AA"), "RO182", "Pharmacy"),
        make_test_record("L5", "WESTMINSTER HEALTH", Some("WESTMINSTER"), Some("GREATER LONDON"), Some("ENGLAND"), Some("SW1A 1AA"), "RO76", "GP Practice"),

        // 2. NEWCASTLE
        make_test_record("N1", "NEWCASTLE GP PRACTICE", Some("NEWCASTLE"), Some("STAFFORDSHIRE"), Some("ENGLAND"), Some("ST5 1AA"), "RO76", "GP Practice"),
        make_test_record("N2", "NEWCASTLE CLINIC", Some("NEWCASTLE"), Some("TYNE AND WEAR"), Some("ENGLAND"), Some("NE1 1AA"), "RO182", "Pharmacy"),
        make_test_record("N3", "FREEMAN HOSPITAL", Some("NEWCASTLE-UPON-TYNE"), Some("TYNE AND WEAR"), Some("ENGLAND"), Some("NE7 7DN"), "RO197", "Hospital"),
        make_test_record("N4", "ROYAL VICTORIA INFIRMARY", Some("NEWCASTLE UPON TYNE"), Some("TYNE AND WEAR"), Some("ENGLAND"), Some("NE1 4LP"), "RO197", "Hospital"),
        make_test_record("N5", "LYME SURGERY", Some("NEWCASTLE UNDER LYME"), Some("STAFFORDSHIRE"), Some("ENGLAND"), Some("ST5 2BB"), "RO76", "GP Practice"),
        make_test_record("N6", "EMLYN HEALTH", Some("NEWCASTLE EMLYN"), Some("DYFED"), Some("WALES"), Some("SA38 9AA"), "RO76", "GP Practice"),
        make_test_record("N7", "BORDER CLINIC", Some("NEWCASTLETON"), Some("ROXBURGHSHIRE"), Some("SCOTLAND"), Some("TD9 0QD"), "RO76", "GP Practice"),

        // 3. DURHAM
        make_test_record("D1", "DURHAM HEALTH CENTRE", Some("DURHAM"), Some("COUNTY DURHAM"), Some("ENGLAND"), Some("DH1 3YG"), "RO76", "GP Practice"),
        make_test_record("D2", "DARLINGTON MEMORIAL", Some("DARLINGTON"), Some("DURHAM"), Some("ENGLAND"), Some("DL3 6HX"), "RO197", "Hospital"),
        make_test_record("D3", "DURHAM COUNTY CLINIC", Some("DURHAM"), Some("DURHAM"), Some("ENGLAND"), Some("DH1 5XZ"), "RO182", "Pharmacy"),

        // 4. ISLE OF MAN (exercises multi-field matching 4 ways: county, country; town, county, country; country; county)
        make_test_record("I1", "DOUGLAS CLINIC", Some("REGION"), Some("ISLE OF MAN"), Some("ISLE OF MAN"), Some("IM1 1AA"), "RO182", "Pharmacy"),
        make_test_record("I2", "RAMSEY SURGERY", Some("ISLE OF MAN"), Some("ISLE OF MAN"), Some("ISLE OF MAN"), Some("IM2 2AA"), "RO76", "GP Practice"),
        make_test_record("I3", "PEEL HEALTH", Some("PEEL"), Some("REGION"), Some("ISLE OF MAN"), Some("IM3 3AA"), "RO182", "Pharmacy"),
        make_test_record("I4", "CASTLETOWN MEDICAL", Some("CASTLETOWN"), Some("ISLE OF MAN"), Some("UK"), Some("IM4 4AA"), "RO76", "GP Practice"),

        // 5. SEDBERGH
        make_test_record("S1", "SEDBERGH MEDICAL PRACTICE", Some("SEDBERGH"), Some("CUMBRIA"), Some("ENGLAND"), Some("LA10 5DL"), "RO76", "GP Practice"),
    ];

    let edges = build_succession_edges(&records);
    let (succ_closures, pred_closures) = compute_transitive_closures(&records, &edges);
    let prov = ods::provenance::fixture_embedded("2026-08-28");

    export_orgs(&parquet_dir, &records, &succ_closures, &pred_closures, Some(&prov), "2026-08-28").unwrap();
    export_roles(&parquet_dir, &records, Some(&prov), "2026-08-28").unwrap();
    export_relationships(&parquet_dir, &records, Some(&prov), "2026-08-28").unwrap();
    export_successions(&parquet_dir, &records, Some(&prov), "2026-08-28").unwrap();

    (tmp, parquet_dir)
}

// ===========================================================================
// Unconditional Synthetic Acceptance Tests (Runs in CI)
// ===========================================================================

#[test]
fn test_synthetic_in_repeatable_and_union() {
    let (_tmp, parquet_dir) = setup_exact_in_workspace();

    // newcastle alone -> N1, N2 (2 rows)
    let mut out_nc = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["newcastle".to_string()],
            format: OutputFormat::Json,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out_nc,
        &parquet_dir,
    )
    .expect("find --in newcastle");
    let count_nc = String::from_utf8(out_nc).unwrap().lines().count();
    assert_eq!(count_nc, 2);

    // newcastle upon tyne alone -> N3, N4 (2 rows)
    let mut out_nut = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["newcastle upon tyne".to_string()],
            format: OutputFormat::Json,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out_nut,
        &parquet_dir,
    )
    .expect("find --in 'newcastle upon tyne'");
    let count_nut = String::from_utf8(out_nut).unwrap().lines().count();
    assert_eq!(count_nut, 2);

    // Union: --in newcastle --in "newcastle upon tyne" -> 4 rows
    let mut out_both = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["newcastle".to_string(), "newcastle upon tyne".to_string()],
            format: OutputFormat::Json,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out_both,
        &parquet_dir,
    )
    .expect("find --in newcastle --in 'newcastle upon tyne'");
    let count_both = String::from_utf8(out_both).unwrap().lines().count();
    assert_eq!(count_both, 4);

    // Duplicate value changes no count
    let mut out_dup = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["newcastle".to_string(), "newcastle".to_string()],
            format: OutputFormat::Json,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out_dup,
        &parquet_dir,
    )
    .expect("find --in newcastle --in newcastle");
    let count_dup = String::from_utf8(out_dup).unwrap().lines().count();
    assert_eq!(count_dup, 2);

    // Location and role filters operate independently and predictably
    let mut out_london = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["london".to_string()],
            format: OutputFormat::Json,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out_london,
        &parquet_dir,
    )
    .expect("find --in london");
    let count_london = String::from_utf8(out_london).unwrap().lines().count();
    assert_eq!(count_london, 3);

    let mut out_ro76 = Vec::new();
    find::run_with_writer(
        Args {
            role: vec!["RO76".to_string()],
            format: OutputFormat::Json,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out_ro76,
        &parquet_dir,
    )
    .expect("find --role RO76");
    let count_ro76 = String::from_utf8(out_ro76).unwrap().lines().count();
    assert_eq!(count_ro76, 12);
}

#[test]
fn test_synthetic_min_length_check() {
    let (_tmp, parquet_dir) = setup_exact_in_workspace();

    let mut out = Vec::new();
    let err = find::run_with_writer(
        Args {
            location: vec!["ab".to_string()],
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .unwrap_err();

    assert!(err.to_string().contains("✖ Location query 'ab' is too short (minimum 3 characters)"));
}

#[test]
fn test_synthetic_exact_match_london_no_londonderry() {
    let (_tmp, parquet_dir) = setup_exact_in_workspace();

    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            gp: true,
            location: vec!["london".to_string()],
            format: OutputFormat::Json,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .expect("find --gp --in london");

    let text = String::from_utf8(out).unwrap();
    assert!(!text.contains("LONDONDERRY"));
    assert!(text.contains("ST BARTS LONDON"));
    assert!(text.contains("CITY SURGERY LONDON"));
}

#[test]
fn test_synthetic_exact_match_newcastle_no_newcastleton() {
    let (_tmp, parquet_dir) = setup_exact_in_workspace();

    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["newcastle".to_string()],
            format: OutputFormat::Json,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .expect("find --in newcastle");

    let text = String::from_utf8(out).unwrap();
    assert!(!text.contains("BORDER CLINIC"));
    assert!(!text.contains("NEWCASTLETON"));
    assert!(text.contains("NEWCASTLE GP PRACTICE"));
    assert!(text.contains("NEWCASTLE CLINIC"));

    // Verify matched column is town, not postcode
    let mut out_table = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["newcastle".to_string()],
            format: OutputFormat::Table,
            input: Some(parquet_dir.clone()),
            plain: true,
            ..Default::default()
        },
        &mut out_table,
        &parquet_dir,
    )
    .expect("find --in newcastle table");
    let cells = get_matched_cells(&String::from_utf8(out_table).unwrap());
    for cell in cells {
        assert_eq!(cell, "town");
    }
}

#[test]
fn test_synthetic_hyphenated_and_spaced_names_identical() {
    let (_tmp, parquet_dir) = setup_exact_in_workspace();

    let mut out_spaced = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["newcastle upon tyne".to_string()],
            format: OutputFormat::Json,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out_spaced,
        &parquet_dir,
    )
    .expect("find spaced");

    let mut out_hyphen = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["newcastle-upon-tyne".to_string()],
            format: OutputFormat::Json,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out_hyphen,
        &parquet_dir,
    )
    .expect("find hyphenated");

    let text_spaced = String::from_utf8(out_spaced).unwrap();
    let text_hyphen = String::from_utf8(out_hyphen).unwrap();
    assert_eq!(text_spaced, text_hyphen);
    assert_eq!(text_spaced.lines().count(), 2);
}

#[test]
fn test_synthetic_monotonicity() {
    let (_tmp, parquet_dir) = setup_exact_in_workspace();

    let mut out_london = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["london".to_string()],
            format: OutputFormat::Json,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out_london,
        &parquet_dir,
    )
    .expect("find --in london");
    let count_london = String::from_utf8(out_london).unwrap().lines().count();

    let mut out_gp_london = Vec::new();
    find::run_with_writer(
        Args {
            gp: true,
            location: vec!["london".to_string()],
            format: OutputFormat::Json,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out_gp_london,
        &parquet_dir,
    )
    .expect("find --gp --in london");
    let count_gp_london = String::from_utf8(out_gp_london).unwrap().lines().count();

    assert!(count_gp_london <= count_london);
    assert_eq!(count_london, 3);
    assert_eq!(count_gp_london, 2);
}

fn get_matched_cells(table_str: &str) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();
    let mut current_row: Option<String> = None;

    for l in table_str.lines() {
        // A data row always has column dividers; the footer row never does, whatever
        // words it uses (`open`, `legally closed`, `record(s)`, ...).
        if l.starts_with("│ ") && !l.contains("ODS Code") && l.contains('┆') {
            let cols: Vec<&str> = l.split('┆').collect();
            let first_col = cols.first().map(|c| c.trim_start_matches('│').trim()).unwrap_or("");
            let last_col = cols.last().map(|c| c.trim().trim_end_matches('│').trim()).unwrap_or("");

            if !first_col.is_empty() {
                if let Some(prev) = current_row.take() {
                    rows.push(prev);
                }
                current_row = Some(last_col.to_string());
            } else if let Some(ref mut cur) = current_row {
                if !last_col.is_empty() {
                    if !cur.is_empty() && !cur.ends_with(' ') {
                        cur.push(' ');
                    }
                    cur.push_str(last_col);
                }
            }
        }
    }
    if let Some(prev) = current_row {
        rows.push(prev);
    }
    rows
}

#[test]
fn test_synthetic_matched_column_isle_of_man_4_way() {
    let (_tmp, parquet_dir) = setup_exact_in_workspace();

    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["isle of man".to_string()],
            format: OutputFormat::Table,
            input: Some(parquet_dir.clone()),
            plain: true,
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .expect("find --in 'isle of man'");

    let text = String::from_utf8(out).unwrap();
    let cells = get_matched_cells(&text);
    assert!(text.lines().any(|l| l.contains("ODS Code") && l.contains("Matched")));
    assert!(cells.contains(&"county, country".to_string()));
    assert!(cells.contains(&"town, county, country".to_string()));
    assert!(cells.contains(&"country".to_string()));
    assert!(cells.contains(&"county".to_string()));
}

#[test]
fn test_synthetic_matched_column_london_town_county() {
    let (_tmp, parquet_dir) = setup_exact_in_workspace();

    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["london".to_string()],
            format: OutputFormat::Table,
            input: Some(parquet_dir.clone()),
            plain: true,
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .expect("find --in london");

    let text = String::from_utf8(out).unwrap();
    let cells = get_matched_cells(&text);
    assert!(cells.contains(&"town, county".to_string()));
}

#[test]
fn test_synthetic_matched_column_durham() {
    let (_tmp, parquet_dir) = setup_exact_in_workspace();

    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["durham".to_string()],
            format: OutputFormat::Table,
            input: Some(parquet_dir.clone()),
            plain: true,
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .expect("find --in durham");

    let text = String::from_utf8(out).unwrap();
    // Verify Matched header column exists
    assert!(text.contains("Matched"));
    let header_line = text.lines().find(|l| l.contains("ODS Code")).unwrap();
    assert!(header_line.contains("Matched"));

    let cells = get_matched_cells(&text);
    assert!(cells.contains(&"town".to_string()));
    assert!(cells.contains(&"county".to_string()));
    assert!(cells.contains(&"town, county".to_string()));
}

#[test]
fn test_synthetic_matched_column_sedbergh() {
    let (_tmp, parquet_dir) = setup_exact_in_workspace();

    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["sedbergh".to_string()],
            format: OutputFormat::Table,
            input: Some(parquet_dir.clone()),
            plain: true,
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .expect("find --in sedbergh");

    let text = String::from_utf8(out).unwrap();
    let cells = get_matched_cells(&text);
    assert_eq!(cells, vec!["town".to_string()]);
}

#[test]
fn test_synthetic_json_has_no_matched_key() {
    let (_tmp, parquet_dir) = setup_exact_in_workspace();

    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["durham".to_string()],
            format: OutputFormat::Json,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .expect("find --in durham --format json");

    let text = String::from_utf8(out).unwrap();
    for line in text.lines() {
        let v: serde_json::Value = serde_json::from_str(line).expect("valid json");
        assert!(v.get("matched").is_none());
        assert!(v.get("matched_fields").is_none());
    }
}

#[test]
fn test_synthetic_also_line_for_newcastle() {
    let (_tmp, parquet_dir) = setup_exact_in_workspace();

    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["newcastle".to_string()],
            format: OutputFormat::Table,
            input: Some(parquet_dir.clone()),
            plain: true,
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .expect("find --in newcastle");

    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("* Also: \"NEWCASTLE UPON TYNE\" (2)"));
    assert!(!text.contains("newcastleton"));
}

#[test]
fn test_synthetic_also_line_absent_for_sedbergh_and_durham() {
    let (_tmp, parquet_dir) = setup_exact_in_workspace();

    // Verify also line is absent for sedbergh, durham, london
    for loc in &["sedbergh", "durham", "london"] {
        let mut out = Vec::new();
        find::run_with_writer(
            Args {
                location: vec![loc.to_string()],
                format: OutputFormat::Table,
                input: Some(parquet_dir.clone()),
                plain: true,
                ..Default::default()
            },
            &mut out,
            &parquet_dir,
        )
        .expect("find should succeed");

        let text = String::from_utf8(out).unwrap();
        assert!(!text.contains("* Also:"));
    }
}

#[test]
fn test_synthetic_zero_matches_newcastel_suggestions() {
    let (_tmp, parquet_dir) = setup_exact_in_workspace();

    let mut out = Vec::new();
    let err = find::run_with_writer(
        Args {
            location: vec!["newcastel".to_string()],
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .unwrap_err();

    let err_msg = err.to_string();
    assert!(err_msg.contains("✖ No location matches 'newcastel'"));
    assert!(err_msg.contains("Did you mean: NEWCASTLE"));
}

#[test]
fn test_synthetic_zero_matches_multiple_unmatched() {
    let (_tmp, parquet_dir) = setup_exact_in_workspace();

    let mut out = Vec::new();
    let err = find::run_with_writer(
        Args {
            location: vec!["foo_place".to_string(), "bar_place".to_string()],
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .unwrap_err();

    let err_msg = err.to_string();
    assert!(err_msg.contains("✖ No location matches 'foo_place'"));
    assert!(err_msg.contains("✖ No location matches 'bar_place'"));
}


// ---------------------------------------------------------------------------
// Task 7: Help text
// ---------------------------------------------------------------------------

#[test]
fn test_find_in_split_district_json_output_cleanliness() {
    let (_tmp, parquet_dir) = setup_exact_in_workspace();

    let mut out_stdout = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["SW1".to_string()],
            format: OutputFormat::Json,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out_stdout,
        &parquet_dir,
    )
    .expect("find --in SW1 --format json");

    let text = String::from_utf8(out_stdout).unwrap();
    // Must return rows
    assert!(!text.trim().is_empty(), "JSON output should not be empty");
    for line in text.lines() {
        assert!(!line.starts_with('*'), "No * notice line on stdout in json format");
        let _: serde_json::Value = serde_json::from_str(line).expect("valid json row");
    }
}

#[test]
fn test_in_examples() {
    let (_tmp, parquet_dir) = common::setup_find_test_workspace();

    // Table of examples:
    // (query_value, expected_matches: Vec<(ods_code, matched_fields)>)
    let cases = vec![
        // LA1 against LA10: LA1 must not match LA10
        ("LA1", vec![]),
        // LA10 matches all 14 Sedbergh records via postcode
        (
            "LA10",
            vec![
                ("8GJ58", "postcode"),
                ("A82608", "postcode"),
                ("A82608001", "postcode"),
                ("D2E8H", "postcode"),
                ("EE112233", "postcode"),
                ("EE112331", "postcode"),
                ("EE112451", "postcode"),
                ("EE137269", "postcode"),
                ("FLG02", "postcode"),
                ("RNN88", "postcode"),
                ("RW5OX", "postcode"),
                ("RX796", "postcode"),
                ("V25604", "postcode"),
                ("VN6C2", "postcode"),
            ],
        ),
        // EC1 reaching EC1A (EC1A01)
        ("EC1", vec![("EC1A01", "postcode")]),
        // E1 reaching E1 1AA (E101) and E1W (E1W01)
        ("E1", vec![("E101", "postcode"), ("E1W01", "postcode")]),
        // E1W reaching E1W01
        ("E1W", vec![("E1W01", "postcode")]),
        // 2-character district N1 accepted and reaching N101, but NOT N1101
        ("N1", vec![("N101", "postcode")]),
        // Whole postcode with space
        (
            "LA10 5DL",
            vec![
                ("A82608", "postcode"),
                ("A82608001", "postcode"),
            ],
        ),
        // Whole postcode without space
        (
            "LA105DL",
            vec![
                ("A82608", "postcode"),
                ("A82608001", "postcode"),
            ],
        ),
    ];

    for (val, expected) in cases {
        let mut out = Vec::new();
        let res = find::run_with_writer(
            Args {
                location: vec![val.to_string()],
                format: OutputFormat::Table,
                input: Some(parquet_dir.clone()),
                plain: true,
                sort: Some(SortBy::Code),
                ..Default::default()
            },
            &mut out,
            &parquet_dir,
        );

        let mut actual: Vec<(String, String)> = Vec::new();
        if let Ok(()) = res {
            let text = String::from_utf8(out).unwrap();

            // Parse rows from table
            for line in text.lines() {
                if line.starts_with("│ ") && !line.contains("ODS Code") && !line.contains("record") {
                    let parts: Vec<&str> = line.split('┆').collect();
                    if parts.len() >= 6 {
                        let code = parts[0].trim_start_matches('│').trim().to_string();
                        let matched = parts[5].trim_end_matches('│').trim().to_string();
                        if !code.is_empty() {
                            actual.push((code, matched));
                        }
                    }
                }
            }
            actual.sort_by(|a, b| a.0.cmp(&b.0));
        } else if !expected.is_empty() {
            panic!("find --in {} failed unexpectedly: {}", val, res.unwrap_err());
        }

        let mut expected_sorted: Vec<(String, String)> = expected
            .into_iter()
            .map(|(c, m)| (c.to_string(), m.to_string()))
            .collect();
        expected_sorted.sort_by(|a, b| a.0.cmp(&b.0));

        assert_eq!(
            actual, expected_sorted,
            "Mismatch for --in {}:\nGot: {:?}\nExpected: {:?}",
            val, actual, expected_sorted
        );
    }
}


use std::collections::HashMap;
use std::path::PathBuf;
use tempfile::TempDir;

use ods::commands::find::{self, Args, OutputFormat};
use ods::commands::parquet::{
    build_succession_edges, compute_transitive_closures, export_orgs, export_orgs_all,
    export_relationships, export_roles, export_successions,
};
use ods::ods_xml::{Location, OdsRecord, OdsRole};
use ods::provenance::OdsProvenance;

fn get_release_dir() -> Option<PathBuf> {
    let p = PathBuf::from("ods_data/releases/2026-08-28");
    if p.join("orgs.parquet").exists() {
        Some(p)
    } else {
        None
    }
}

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
    let prov = OdsProvenance {
        trud_release_date: Some("2026-08-28".to_string()),
        ..Default::default()
    };

    export_orgs(&parquet_dir, &records, &succ_closures, &pred_closures, Some(&prov)).unwrap();
    export_orgs_all(&parquet_dir, &records, &succ_closures, &pred_closures, Some(&prov)).unwrap();
    export_roles(&parquet_dir, &records, Some(&prov)).unwrap();
    export_relationships(&parquet_dir, &records, Some(&prov)).unwrap();
    export_successions(&parquet_dir, &records, Some(&prov)).unwrap();

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
    table_str
        .lines()
        .filter(|l| l.starts_with("│ ") && !l.contains("ODS Code") && !l.contains("record"))
        .filter_map(|l| l.split('┆').next_back())
        .map(|cell| cell.trim().trim_end_matches('│').trim().to_string())
        .collect()
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
    assert!(text.lines().any(|l| l.contains("ODS Code") && l.contains("Matched")));
    let cells = get_matched_cells(&text);
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
    assert!(text.contains("* Also: \"newcastle upon tyne\" (2)"));
    assert!(!text.contains("newcastleton"));
}

#[test]
fn test_synthetic_also_line_absent_for_sedbergh_and_durham() {
    let (_tmp, parquet_dir) = setup_exact_in_workspace();

    for loc in &["sedbergh", "durham"] {
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
    assert!(err_msg.contains("Did you mean: newcastle"));
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

#[test]
fn test_synthetic_no_search_line() {
    let (_tmp, parquet_dir) = setup_exact_in_workspace();

    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            query: Some("sedbergh".to_string()),
            gp: true,
            location: vec!["cumbria".to_string()],
            input: Some(parquet_dir.clone()),
            plain: true,
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .expect("find sedbergh --gp --in cumbria");

    let text = String::from_utf8(out).unwrap();
    assert!(!text.contains("* Search:"));
}

// ===========================================================================
// Full Release Tests (Requires ods_data/releases/2026-08-28)
// Run with: cargo test --test find_in_exact_test -- --ignored
// ===========================================================================

#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_task1_in_repeatable_and_union() {
    let release_dir = get_release_dir().expect("requires ods_data/releases/2026-08-28");

    // newcastle alone
    let mut out_nc = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["newcastle".to_string()],
            format: OutputFormat::Json,
            input: Some(release_dir.clone()),
            ..Default::default()
        },
        &mut out_nc,
        &release_dir,
    )
    .expect("find --in newcastle");
    let count_nc = String::from_utf8(out_nc).unwrap().lines().count();
    assert_eq!(count_nc, 318);

    // newcastle upon tyne alone
    let mut out_nut = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["newcastle upon tyne".to_string()],
            format: OutputFormat::Json,
            input: Some(release_dir.clone()),
            ..Default::default()
        },
        &mut out_nut,
        &release_dir,
    )
    .expect("find --in 'newcastle upon tyne'");
    let count_nut = String::from_utf8(out_nut).unwrap().lines().count();
    assert_eq!(count_nut, 1561);

    // Union: --in newcastle --in "newcastle upon tyne"
    let mut out_both = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["newcastle".to_string(), "newcastle upon tyne".to_string()],
            format: OutputFormat::Json,
            input: Some(release_dir.clone()),
            ..Default::default()
        },
        &mut out_both,
        &release_dir,
    )
    .expect("find --in newcastle --in 'newcastle upon tyne'");
    let count_both = String::from_utf8(out_both).unwrap().lines().count();
    assert_eq!(count_both, count_nc + count_nut); // 318 + 1561 = 1879

    // Duplicate value changes no count
    let mut out_dup = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["newcastle".to_string(), "newcastle".to_string()],
            format: OutputFormat::Json,
            input: Some(release_dir.clone()),
            ..Default::default()
        },
        &mut out_dup,
        &release_dir,
    )
    .expect("find --in newcastle --in newcastle");
    let count_dup = String::from_utf8(out_dup).unwrap().lines().count();
    assert_eq!(count_dup, count_nc);
}

// ---------------------------------------------------------------------------
// Task 2: Exact match on normalised value
// ---------------------------------------------------------------------------

#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_task2_exact_match_london_no_londonderry() {
    let release_dir = get_release_dir().expect("requires ods_data/releases/2026-08-28");

    // ods find --gp --in london returns no rows with town LONDONDERRY
    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            gp: true,
            location: vec!["london".to_string()],
            format: OutputFormat::Json,
            input: Some(release_dir.clone()),
            ..Default::default()
        },
        &mut out,
        &release_dir,
    )
    .expect("find --gp --in london");

    let text = String::from_utf8(out).unwrap();
    for line in text.lines() {
        assert!(
            !line.to_uppercase().contains("\"TOWN\":\"LONDONDERRY\""),
            "Expected no LONDONDERRY rows, but found: {}",
            line
        );
    }
}

#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_task2_newcastle_no_ne_postcodes() {
    let release_dir = get_release_dir().expect("requires ods_data/releases/2026-08-28");

    // At baseline, --in newcastle matched 1,559 rows via postcode prefix "ne".
    // Today, only 318 rows match, none via postcode (313 ST, 3 BT, and 2 Tyne and Wear where town was entered NEWCASTLE).
    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["newcastle".to_string()],
            format: OutputFormat::Json,
            input: Some(release_dir.clone()),
            ..Default::default()
        },
        &mut out,
        &release_dir,
    )
    .expect("find --in newcastle");

    let text = String::from_utf8(out).unwrap();
    let mut ne_postcodes = 0;
    for line in text.lines() {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        if let Some(postcode) = v["postcode"].as_str() {
            if postcode.starts_with("NE") {
                ne_postcodes += 1;
            }
        }
    }
    // Baseline was 1,559 NE postcodes. Now only 2 (where town was explicitly entered as NEWCASTLE).
    assert_eq!(ne_postcodes, 2);

    // Verify in table format that every single row matched on "town" and none on "postcode"
    let mut out_table = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["newcastle".to_string()],
            format: OutputFormat::Table,
            input: Some(release_dir.clone()),
            plain: true,
            ..Default::default()
        },
        &mut out_table,
        &release_dir,
    )
    .expect("find --in newcastle table");

    let table_text = String::from_utf8(out_table).unwrap();
    for line in table_text.lines() {
        if line.starts_with("│ ") && !line.contains("ODS Code") && !line.contains("records") {
            let last_cell = line.split('┆').next_back().unwrap().trim().trim_end_matches('│').trim();
            if !last_cell.is_empty() {
                assert_eq!(last_cell, "town");
            }
        }
    }
}

#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_task2_hyphenated_and_spaced_names_identical() {
    let release_dir = get_release_dir().expect("requires ods_data/releases/2026-08-28");

    let mut out_hyphen = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["newcastle-under-lyme".to_string()],
            format: OutputFormat::Json,
            input: Some(release_dir.clone()),
            ..Default::default()
        },
        &mut out_hyphen,
        &release_dir,
    )
    .expect("find --in newcastle-under-lyme");

    let mut out_space = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["newcastle under lyme".to_string()],
            format: OutputFormat::Json,
            input: Some(release_dir.clone()),
            ..Default::default()
        },
        &mut out_space,
        &release_dir,
    )
    .expect("find --in newcastle under lyme");

    let text_hyphen = String::from_utf8(out_hyphen).unwrap();
    let text_space = String::from_utf8(out_space).unwrap();
    assert_eq!(text_hyphen, text_space);
    assert_eq!(text_hyphen.lines().count(), 33);
}

// ---------------------------------------------------------------------------
// Task 3: Return every match (delete level collapse) & monotonicity property
// ---------------------------------------------------------------------------

#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_task3_durham_returns_all_674_matches() {
    let release_dir = get_release_dir().expect("requires ods_data/releases/2026-08-28");

    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["durham".to_string()],
            format: OutputFormat::Json,
            input: Some(release_dir.clone()),
            ..Default::default()
        },
        &mut out,
        &release_dir,
    )
    .expect("find --in durham");

    let text = String::from_utf8(out).unwrap();
    assert_eq!(text.lines().count(), 674);
}

#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_task3_monotonicity_property_adding_filter_never_increases_count() {
    let release_dir = get_release_dir().expect("requires ods_data/releases/2026-08-28");

    // ods find --in london
    let mut out_london = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["london".to_string()],
            format: OutputFormat::Json,
            input: Some(release_dir.clone()),
            ..Default::default()
        },
        &mut out_london,
        &release_dir,
    )
    .expect("find --in london");
    let count_london = String::from_utf8(out_london).unwrap().lines().count();

    // ods find --gp --in london
    let mut out_gp_london = Vec::new();
    find::run_with_writer(
        Args {
            gp: true,
            location: vec!["london".to_string()],
            format: OutputFormat::Json,
            input: Some(release_dir.clone()),
            ..Default::default()
        },
        &mut out_gp_london,
        &release_dir,
    )
    .expect("find --gp --in london");
    let count_gp_london = String::from_utf8(out_gp_london).unwrap().lines().count();

    // Monotonicity property: adding --gp filter must never increase row count
    assert!(
        count_gp_london <= count_london,
        "Monotonicity violated: --gp --in london ({}) > --in london ({})",
        count_gp_london,
        count_london
    );
    assert_eq!(count_london, 18374);
}

// ---------------------------------------------------------------------------
// Task 4: Name the fields each row matched on (Matched column)
// ---------------------------------------------------------------------------

#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_task4_matched_column_durham() {
    let release_dir = get_release_dir().expect("requires ods_data/releases/2026-08-28");

    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["durham".to_string()],
            format: OutputFormat::Table,
            input: Some(release_dir.clone()),
            plain: true,
            ..Default::default()
        },
        &mut out,
        &release_dir,
    )
    .expect("find --in durham");

    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("Matched"));

    // Header has Matched
    let header_line = text.lines().find(|l| l.contains("ODS Code")).unwrap();
    assert!(header_line.ends_with("Matched │"));

    let mut county_count = 0;
    let mut town_count = 0;
    let mut both_count = 0;

    for line in text.lines() {
        if line.starts_with("│ ") && !line.contains("ODS Code") && !line.contains("records") {
            let last_cell = line.split('┆').next_back().unwrap().trim().trim_end_matches('│').trim();
            if last_cell == "county" {
                county_count += 1;
            } else if last_cell == "town" {
                town_count += 1;
            } else if last_cell.contains("county") && last_cell.contains("town") {
                both_count += 1;
            }
        }
    }

    assert_eq!(county_count, 92);
    assert_eq!(town_count, 582);
    assert_eq!(both_count, 0);
    assert_eq!(county_count + town_count, 674);
}

#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_task4_matched_column_isle_of_man() {
    let release_dir = get_release_dir().expect("requires ods_data/releases/2026-08-28");

    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["isle of man".to_string()],
            format: OutputFormat::Table,
            input: Some(release_dir.clone()),
            plain: true,
            ..Default::default()
        },
        &mut out,
        &release_dir,
    )
    .expect("find --in 'isle of man'");

    let text = String::from_utf8(out).unwrap();
    let mut group_counts: HashMap<String, usize> = HashMap::new();

    for line in text.lines() {
        if line.starts_with("│ ") && !line.contains("ODS Code") && !line.contains("records") {
            let last_cell = line.split('┆').next_back().unwrap().trim().trim_end_matches('│').trim();
            if !last_cell.is_empty() {
                *group_counts.entry(last_cell.to_string()).or_default() += 1;
            }
        }
    }

    assert_eq!(group_counts.get("county, country"), Some(&139));
    assert_eq!(group_counts.get("town, county, country"), Some(&55));
    assert_eq!(group_counts.get("country"), Some(&6));
    assert_eq!(group_counts.get("county"), Some(&1));
    assert_eq!(group_counts.values().sum::<usize>(), 201);
}

#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_task4_matched_column_london_single_town_county_row() {
    let release_dir = get_release_dir().expect("requires ods_data/releases/2026-08-28");

    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["london".to_string()],
            format: OutputFormat::Table,
            input: Some(release_dir.clone()),
            plain: true,
            ..Default::default()
        },
        &mut out,
        &release_dir,
    )
    .expect("find --in london");

    let text = String::from_utf8(out).unwrap();
    let mut town_county_count = 0;

    for line in text.lines() {
        if line.starts_with("│ ") && !line.contains("ODS Code") && !line.contains("records") {
            let last_cell = line.split('┆').next_back().unwrap().trim().trim_end_matches('│').trim();
            if last_cell == "town, county" {
                town_county_count += 1;
            }
        }
    }

    assert_eq!(town_county_count, 1);
}

#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_task4_matched_column_sedbergh() {
    let release_dir = get_release_dir().expect("requires ods_data/releases/2026-08-28");

    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["sedbergh".to_string()],
            format: OutputFormat::Table,
            input: Some(release_dir.clone()),
            plain: true,
            ..Default::default()
        },
        &mut out,
        &release_dir,
    )
    .expect("find --in sedbergh");

    let text = String::from_utf8(out).unwrap();
    let mut row_count = 0;

    for line in text.lines() {
        if line.starts_with("│ ") && !line.contains("ODS Code") && !line.contains("records") {
            let last_cell = line.split('┆').next_back().unwrap().trim().trim_end_matches('│').trim();
            assert_eq!(last_cell, "town");
            row_count += 1;
        }
    }

    assert_eq!(row_count, 14);
}

#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_task4_json_has_no_matched_key() {
    let release_dir = get_release_dir().expect("requires ods_data/releases/2026-08-28");

    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["durham".to_string()],
            format: OutputFormat::Json,
            input: Some(release_dir.clone()),
            ..Default::default()
        },
        &mut out,
        &release_dir,
    )
    .expect("find --in durham --format json");

    let text = String::from_utf8(out).unwrap();
    let first_line = text.lines().next().unwrap();
    let v: serde_json::Value = serde_json::from_str(first_line).unwrap();
    let obj = v.as_object().unwrap();

    assert!(!obj.contains_key("matched"));
    assert!(!obj.contains_key("matched_fields"));
    assert_eq!(obj.len(), 25);
}

// ---------------------------------------------------------------------------
// Task 5: Offer near-misses
// ---------------------------------------------------------------------------

#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_task5_also_line_for_newcastle() {
    let release_dir = get_release_dir().expect("requires ods_data/releases/2026-08-28");

    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            location: vec!["newcastle".to_string()],
            format: OutputFormat::Table,
            input: Some(release_dir.clone()),
            plain: true,
            ..Default::default()
        },
        &mut out,
        &release_dir,
    )
    .expect("find --in newcastle");

    let text = String::from_utf8(out).unwrap();
    let also_line = text.lines().find(|l| l.starts_with("* Also:")).expect("expected * Also: line");
    assert_eq!(
        also_line,
        "* Also: \"newcastle upon tyne\" (1561) · \"newcastle under lyme\" (33) · \"newcastle emlyn\" (15) · +1 more"
    );
    assert!(!also_line.contains("newcastleton"));
}

#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_task5_also_line_absent_for_sedbergh_durham_london() {
    let release_dir = get_release_dir().expect("requires ods_data/releases/2026-08-28");

    for loc in ["sedbergh", "durham", "london"] {
        let mut out = Vec::new();
        find::run_with_writer(
            Args {
                location: vec![loc.to_string()],
                format: OutputFormat::Table,
                input: Some(release_dir.clone()),
                plain: true,
                ..Default::default()
            },
            &mut out,
            &release_dir,
        )
        .expect("find should succeed");

        let text = String::from_utf8(out).unwrap();
        assert!(
            !text.contains("* Also:"),
            "Expected no * Also: line for {}, got:\n{}",
            loc,
            text
        );
    }
}

#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_task5_zero_matches_newcastel_suggestions() {
    let release_dir = get_release_dir().expect("requires ods_data/releases/2026-08-28");

    let mut out = Vec::new();
    let err = find::run_with_writer(
        Args {
            location: vec!["newcastel".to_string()],
            input: Some(release_dir.clone()),
            ..Default::default()
        },
        &mut out,
        &release_dir,
    )
    .unwrap_err();

    let err_msg = err.to_string();
    assert!(err_msg.contains("✖ No location matches 'newcastel'"));
    assert!(err_msg.contains("Did you mean: newcastle · newcastle upon tyne · newcastle emlyn"));
}

// ---------------------------------------------------------------------------
// Task 6: Delete search line
// ---------------------------------------------------------------------------

#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_task6_no_search_line() {
    let release_dir = get_release_dir().expect("requires ods_data/releases/2026-08-28");

    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            query: Some("sedbergh".to_string()),
            gp: true,
            location: vec!["cumbria".to_string()],
            input: Some(release_dir.clone()),
            plain: true,
            ..Default::default()
        },
        &mut out,
        &release_dir,
    )
    .expect("find sedbergh --gp --in cumbria");

    let text = String::from_utf8(out).unwrap();
    assert!(!text.contains("* Search:"));
    assert!(text.contains("* Source: releases/2026-08-28/orgs.parquet"));
    assert!(text.contains("1 active record"));
}

// ---------------------------------------------------------------------------
// Task 7: Help text
// ---------------------------------------------------------------------------

#[test]
fn test_task7_after_help_verbatim() {
    use clap::CommandFactory;

    let mut cmd = Args::command();
    let long_help = cmd.render_long_help().to_string();

    let expected_after_help = "Filters combine with AND. Repeated or comma-separated values within one flag
combine with OR:

  ods find --gp --in cumbria           GP practices AND in Cumbria
  ods find --in durham --in cumbria    in Durham OR in Cumbria

--gp and --dentist add role codes to --role rather than filtering separately.
--all reads orgs_all.parquet, which is why it adds a Status column.

--in matches a whole country, county or town, ignoring case and punctuation, or
a postcode prefix. Town and county are as ODS records them: postal and historic,
not administrative. London-area practices are recorded under MIDDLESEX (196 GP
practices, none with town LONDON), ESSEX, KENT and SURREY, so no --in value
selects a metropolitan area.";

    assert!(long_help.contains(expected_after_help));

    // -h (short help) should not contain after_help
    let mut cmd_short = Args::command();
    let short_help = cmd_short.render_help().to_string();
    assert!(!short_help.contains("Filters combine with AND"));
}

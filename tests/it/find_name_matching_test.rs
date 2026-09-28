use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

use ods::commands::find::{self, Args, OutputFormat};
use ods::commands::parquet::{
    build_succession_edges, compute_transitive_closures, export_orgs,
    export_relationships, export_roles, export_successions,
};
use ods::ods_xml::{Location, OdsRecord, OdsRole};

fn get_release_dir() -> Option<PathBuf> {
    let p = PathBuf::from("ods_data/releases/2026-08-28");
    if p.join("orgs.parquet").exists() {
        Some(p)
    } else {
        None
    }
}

fn duckdb_available() -> bool {
    Command::new("duckdb")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn require_duckdb() -> bool {
    let avail = duckdb_available();
    if !avail {
        if std::env::var("CI").as_deref() == Ok("true") {
            panic!("duckdb CLI is required in CI, but was not found on PATH");
        } else {
            eprintln!("Skipping test: duckdb CLI not found on PATH");
            return false;
        }
    }
    true
}

fn make_test_record(ods_code: &str, name: &str, role_code: &str, role_name: &str) -> OdsRecord {
    OdsRecord {
        ods_code: ods_code.to_string(),
        name: name.to_string(),
        status: "active".to_string(),
        record_class: "org".to_string(),
        role: role_name.to_string(),
        geo_loc: Some(Location {
            address_lines: vec!["1 Test Street".to_string()],
            town: Some("LONDON".to_string()),
            county: Some("GREATER LONDON".to_string()),
            country: Some("ENGLAND".to_string()),
            postcode: Some("SW1A 1AA".to_string()),
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

fn setup_name_matching_workspace() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().expect("create temp dir");
    let parquet_dir = tmp.path().join("parquet");
    std::fs::create_dir_all(&parquet_dir).expect("create parquet dir");

    let records = vec![
        make_test_record("A3RE", "L'ARCHE", "RO104", "Social Care Provider"),
        make_test_record("AGX8", "L'ARCHE", "RO104", "Social Care Provider"),
        make_test_record("VM2JG", "GLASFRYN (L'ARCHE)", "RO104", "Social Care Provider"),
        make_test_record("VN0KQ", "L'ARCHE LIVERPOOL", "RO104", "Social Care Provider"),
        make_test_record("8C872", "LARCHES ROAD", "RO157", "Non-NHS Organisation"),
        make_test_record("C7FV", "CEDARS & LARCHES CARE LTD", "RO104", "Social Care Provider"),
        make_test_record("CC01", "CHRIST CHURCH CLINIC", "RO76", "GP Practice"),
        make_test_record("CC02", "CHRISTCHURCH PHARMACY", "RO182", "Pharmacy"),
        make_test_record("HC01", "HOMECARE PLUS", "RO182", "Pharmacy"),
        make_test_record("HC02", "HOME CARE ASSIST", "RO182", "Pharmacy"),
        make_test_record("DN01", "DAYNIGHT CHEMIST", "RO182", "Pharmacy"),
        make_test_record("DN02", "DAY NIGHT PHARMACY", "RO182", "Pharmacy"),
        make_test_record("SM01", "ST MARYS NH", "RO182", "Pharmacy"),
        make_test_record("SM02", "ST MARY'S CLINIC", "RO182", "Pharmacy"),
        make_test_record("SM03", "ST MARY STREET SURGERY", "RO76", "GP Practice"),
        make_test_record("SM04", "TEDBURN ST MARY SCHOOL", "RO221", "School"),
        make_test_record("A82608", "SEDBERGH MEDICAL PRACTICE", "RO76", "GP Practice"),
    ];

    let edges = build_succession_edges(&records);
    let (succ_closures, pred_closures) = compute_transitive_closures(&records, &edges);
    let prov = ods::provenance::fixture_embedded("2026-08-28");

    export_orgs(&parquet_dir, &records, &succ_closures, &pred_closures, Some(&prov)).expect("export orgs");
    export_roles(&parquet_dir, &records, Some(&prov)).expect("export roles");
    export_relationships(&parquet_dir, &records, Some(&prov)).expect("export relationships");
    export_successions(&parquet_dir, &records, Some(&prov)).expect("export successions");

    (tmp, parquet_dir)
}

fn run_find_csv(args: Args, parquet_dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            format: OutputFormat::Csv,
            input: Some(parquet_dir.to_path_buf()),
            ..args
        },
        &mut out,
        parquet_dir,
    )
    .expect("run_with_writer csv");

    let s = String::from_utf8(out).expect("utf8 csv");
    s.lines()
        .skip(1)
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.split(',').next().unwrap_or("").to_string())
        .collect()
}

fn run_find_sql(args: Args, parquet_dir: &Path) -> String {
    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            sql: true,
            input: Some(parquet_dir.to_path_buf()),
            ..args
        },
        &mut out,
        parquet_dir,
    )
    .expect("run_with_writer sql");

    String::from_utf8(out).expect("utf8 sql")
}

fn run_duckdb_csv_codes(sql: &str) -> Vec<String> {
    let output = Command::new("duckdb")
        .args(["-csv", "-c", sql])
        .output()
        .expect("failed to execute duckdb");

    assert!(
        output.status.success(),
        "duckdb failed on SQL:\n{}\nStderr: {}",
        sql,
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("duckdb stdout utf8");
    let mut lines = stdout.lines();
    let header = lines.next().unwrap_or("");
    let ods_col_idx = header
        .split(',')
        .position(|col| col.trim().trim_matches('"') == "ods_code")
        .unwrap_or(0);

    lines
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| l.split(',').nth(ods_col_idx).map(|s| s.trim().trim_matches('"').to_string()))
        .collect()
}


// =========================================================================
// Synthetic Acceptance Tests (Tasks 2-5, run unconditionally in CI in <0.2s)
// =========================================================================


#[test]
fn test_find_sql_flag_duckdb_equivalence_on_synthetic_data() {
    let (_tmp, parquet_dir) = setup_name_matching_workspace();

    let sql = run_find_sql(
        Args {
            query: Some("st marys".to_string()),
            ..Default::default()
        },
        &parquet_dir,
    );
    assert!(sql.contains("AND regexp_replace(name, '[^A-Z0-9]', '', 'g') LIKE '%STMARYS%'"));
    assert!(sql.contains("ORDER BY (name LIKE '%ST MARYS%') DESC, ods_code;"));

    if require_duckdb() {
        let codes_find = run_find_csv(
            Args {
                query: Some("l'arche".to_string()),
                ..Default::default()
            },
            &parquet_dir,
        );
        let sql_larche = run_find_sql(
            Args {
                query: Some("l'arche".to_string()),
                ..Default::default()
            },
            &parquet_dir,
        );
        let codes_duck = run_duckdb_csv_codes(&sql_larche);
        assert_eq!(codes_find, codes_duck, "DuckDB output must match ods find order identically");
    }
}


// =========================================================================
// Full Release Acceptance Tests (Requires ods_data/releases/2026-08-28)
// =========================================================================

#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_squash_agrees_with_duckdb_sql_on_all_release_names_on_release_data() {
    if !require_duckdb() {
        return;
    }
    let release_dir = get_release_dir().expect("requires ods_data/releases/2026-08-28");
    let parquet_path = release_dir.join("orgs.parquet");

    let sql = format!(
        "SELECT count(*) FROM '{}' WHERE regexp_replace(name, '[^A-Z0-9]', '', 'g') != regexp_replace(upper(name), '[^A-Z0-9]', '', 'g');",
        parquet_path.display()
    );
    let count = run_duckdb_csv_codes(&sql);
    assert_eq!(count[0], "0", "SQL expression must match uppercase alphanumeric cleaning");

    let sql_all = format!("SELECT count(*) FROM '{}';", parquet_path.display());
    let total_all = run_duckdb_csv_codes(&sql_all);
    assert_eq!(total_all[0], "370917", "orgs.parquet holds every organisation of both XML files");
}


#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_name_matching_ignores_spacing_punctuation_and_matches_row_counts_on_release_data() {
    let release_dir = get_release_dir().expect("requires ods_data/releases/2026-08-28");

    let queries_and_targets = [
        ("christchurch", 153),
        ("christ church", 153),
        ("homecare", 2879),
        ("daynight", 32),
        ("l'arche", 20),
        ("st marys", 566),
        ("sedbergh", 6),
    ];

    for (q, expected) in queries_and_targets {
        let codes = run_find_csv(
            Args {
                query: Some(q.to_string()),
                ..Default::default()
            },
            &release_dir,
        );
        assert_eq!(
            codes.len(),
            expected,
            "Query '{}' expected {} matches, got {}",
            q,
            expected,
            codes.len()
        );
    }

    // Identical row sets for christchurch vs christ church
    let cc1: HashSet<String> = run_find_csv(
        Args {
            query: Some("christchurch".to_string()),
            ..Default::default()
        },
        &release_dir,
    )
    .into_iter()
    .collect();

    let cc2: HashSet<String> = run_find_csv(
        Args {
            query: Some("christ church".to_string()),
            ..Default::default()
        },
        &release_dir,
    )
    .into_iter()
    .collect();

    assert_eq!(cc1, cc2, "christchurch and christ church must return identical row sets");

    let hc1: HashSet<String> = run_find_csv(
        Args {
            query: Some("homecare".to_string()),
            ..Default::default()
        },
        &release_dir,
    )
    .into_iter()
    .collect();

    let hc2: HashSet<String> = run_find_csv(
        Args {
            query: Some("home care".to_string()),
            ..Default::default()
        },
        &release_dir,
    )
    .into_iter()
    .collect();

    assert_eq!(hc1, hc2, "homecare and home care must return identical row sets");

    let dn1: HashSet<String> = run_find_csv(
        Args {
            query: Some("daynight".to_string()),
            ..Default::default()
        },
        &release_dir,
    )
    .into_iter()
    .collect();

    let dn2: HashSet<String> = run_find_csv(
        Args {
            query: Some("day night".to_string()),
            ..Default::default()
        },
        &release_dir,
    )
    .into_iter()
    .collect();

    assert_eq!(dn1, dn2, "daynight and day night must return identical row sets");

    // Monotonicity check
    assert!(cc1.len() >= 77, "No query returns fewer rows than before");
}


#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_ordering_positions_exact_matches_first_on_release_data() {
    let release_dir = get_release_dir().expect("requires ods_data/releases/2026-08-28");

    // l'arche: first 11 have L'ARCHE, last 9 have LARCHES
    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            query: Some("l'arche".to_string()),
            format: OutputFormat::Csv,
            input: Some(release_dir.clone()),
            ..Default::default()
        },
        &mut out,
        &release_dir,
    )
    .unwrap();
    let s = String::from_utf8(out).unwrap();
    let rows: Vec<&str> = s.lines().skip(1).filter(|l| !l.is_empty()).collect();
    assert_eq!(rows.len(), 20);

    for (i, row) in rows.iter().enumerate() {
        let name = row.split(',').nth(1).unwrap();
        if i < 11 {
            assert!(
                name.contains("L'ARCHE"),
                "Row {} should contain L'ARCHE, got: {}",
                i + 1,
                name
            );
        } else {
            assert!(
                name.contains("LARCHES"),
                "Row {} should contain LARCHES, got: {}",
                i + 1,
                name
            );
        }
    }

    // st marys: ST MARY STREET SURGERY at 461 of 566, TEDBURN ST MARY SCHOOL at 363
    let mut out_sm = Vec::new();
    find::run_with_writer(
        Args {
            query: Some("st marys".to_string()),
            format: OutputFormat::Csv,
            input: Some(release_dir.clone()),
            ..Default::default()
        },
        &mut out_sm,
        &release_dir,
    )
    .unwrap();
    let sm_rows: Vec<String> = String::from_utf8(out_sm).unwrap().lines().skip(1).map(|l| l.to_string()).collect();
    assert_eq!(sm_rows.len(), 566);
    let pos_surgery = sm_rows.iter().position(|r| r.contains("ST MARY STREET SURGERY")).map(|p| p + 1);
    assert_eq!(pos_surgery, Some(461), "ST MARY STREET SURGERY must be at pos 461 of 566 for 'st marys'");
    let pos_school = sm_rows.iter().position(|r| r.contains("TEDBURN ST MARY SCHOOL")).map(|p| p + 1);
    assert_eq!(pos_school, Some(363), "TEDBURN ST MARY SCHOOL must be at pos 363 of 566 for 'st marys'");

    // st mary's: ST MARY STREET SURGERY at 517 of 566, TEDBURN ST MARY SCHOOL at 502
    let mut out_sma = Vec::new();
    find::run_with_writer(
        Args {
            query: Some("st mary's".to_string()),
            format: OutputFormat::Csv,
            input: Some(release_dir.clone()),
            ..Default::default()
        },
        &mut out_sma,
        &release_dir,
    )
    .unwrap();
    let sma_rows: Vec<String> = String::from_utf8(out_sma).unwrap().lines().skip(1).map(|l| l.to_string()).collect();
    assert_eq!(sma_rows.len(), 566);
    let pos_surgery_a = sma_rows.iter().position(|r| r.contains("ST MARY STREET SURGERY")).map(|p| p + 1);
    assert_eq!(pos_surgery_a, Some(517), "ST MARY STREET SURGERY must be at pos 517 of 566 for 'st mary\\'s'");
    let pos_school_a = sma_rows.iter().position(|r| r.contains("TEDBURN ST MARY SCHOOL")).map(|p| p + 1);
    assert_eq!(pos_school_a, Some(502), "TEDBURN ST MARY SCHOOL must be at pos 502 of 566 for 'st mary\\'s'");
}


#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_sql_duckdb_equivalence_four_queries_on_release_data() {
    if !require_duckdb() {
        return;
    }
    let release_dir = get_release_dir().expect("requires ods_data/releases/2026-08-28");

    let test_queries = ["sedbergh", "l'arche", "st marys", "christchurch"];
    for q in test_queries {
        let codes_find = run_find_csv(
            Args {
                query: Some(q.to_string()),
                ..Default::default()
            },
            &release_dir,
        );

        let sql = run_find_sql(
            Args {
                query: Some(q.to_string()),
                ..Default::default()
            },
            &release_dir,
        );

        let codes_duck = run_duckdb_csv_codes(&sql);
        assert_eq!(
            codes_find, codes_duck,
            "Query '{}' emitted SQL run in DuckDB must match find CSV sequence in exact order",
            q
        );
    }
}


#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_st_marys_sql_output_format_on_release_data() {
    let release_dir = get_release_dir().expect("requires ods_data/releases/2026-08-28");

    let sql = run_find_sql(
        Args {
            query: Some("st marys".to_string()),
            ..Default::default()
        },
        &release_dir,
    );

    let expected_suffix = "WHERE status = 'active'\n  AND (legal_end IS NULL OR legal_end > trud_release_date)\n  AND regexp_replace(name, '[^A-Z0-9]', '', 'g') LIKE '%STMARYS%'\nORDER BY (name LIKE '%ST MARYS%') DESC, ods_code;";
    assert!(
        sql.contains(expected_suffix),
        "SQL output must contain expected clause and order by, got:\n{}",
        sql
    );
}

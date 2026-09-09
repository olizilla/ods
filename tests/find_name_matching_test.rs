use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

use ods::commands::find::{self, squash_for_matching, Args, OutputFormat};
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
    let prov = OdsProvenance {
        trud_release_date: Some("2026-08-28".to_string()),
        ..Default::default()
    };

    export_orgs(&parquet_dir, &records, &succ_closures, &pred_closures, Some(&prov)).expect("export orgs");
    export_orgs_all(&parquet_dir, &records, &succ_closures, &pred_closures, Some(&prov)).expect("export orgs_all");
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
// Task 1 — Symbol counts and squash fold invariants (runs unconditionally in CI)
// =========================================================================

#[test]
fn test_task1_symbol_counts() {
    let find_src = std::fs::read_to_string("src/commands/find.rs").expect("read find.rs");
    let norm_count = find_src.matches("normalize_for_matching").count();
    let squash_count = find_src.matches("squash_for_matching").count();
    let rank_count = find_src.matches("compute_name_rank").count();

    assert_eq!(
        norm_count, 7,
        "grep -c 'normalize_for_matching' src/commands/find.rs must return 7, got {}",
        norm_count
    );
    assert_eq!(
        squash_count, 3,
        "grep -c 'squash_for_matching' src/commands/find.rs must return 3, got {}",
        squash_count
    );
    assert_eq!(
        rank_count, 0,
        "grep -rn 'compute_name_rank' src/ must return nothing, got {}",
        rank_count
    );
}

#[test]
fn test_task1_squash_unit_invariants() {
    assert_eq!(squash_for_matching("Christchurch"), "CHRISTCHURCH");
    assert_eq!(squash_for_matching("Christ Church"), "CHRISTCHURCH");
    assert_eq!(squash_for_matching("L'Arche"), "LARCHE");
    assert_eq!(squash_for_matching("St Mary's"), "STMARYS");
    assert_eq!(squash_for_matching("Day & Night Pharmacy"), "DAYNIGHTPHARMACY");
}

// =========================================================================
// Synthetic Acceptance Tests (Tasks 2-5, run unconditionally in CI in <0.2s)
// =========================================================================

#[test]
fn test_synthetic_task2_name_matching_ignores_spacing_and_punctuation() {
    let (_tmp, parquet_dir) = setup_name_matching_workspace();

    let codes_cc1 = run_find_csv(
        Args {
            query: Some("christchurch".to_string()),
            ..Default::default()
        },
        &parquet_dir,
    );
    let codes_cc2 = run_find_csv(
        Args {
            query: Some("christ church".to_string()),
            ..Default::default()
        },
        &parquet_dir,
    );
    let set_cc1: HashSet<_> = codes_cc1.into_iter().collect();
    let set_cc2: HashSet<_> = codes_cc2.into_iter().collect();
    assert_eq!(set_cc1.len(), 2);
    assert_eq!(set_cc1, set_cc2);

    let codes_hc1 = run_find_csv(
        Args {
            query: Some("homecare".to_string()),
            ..Default::default()
        },
        &parquet_dir,
    );
    let codes_hc2 = run_find_csv(
        Args {
            query: Some("home care".to_string()),
            ..Default::default()
        },
        &parquet_dir,
    );
    let set_hc1: HashSet<_> = codes_hc1.into_iter().collect();
    let set_hc2: HashSet<_> = codes_hc2.into_iter().collect();
    assert_eq!(set_hc1.len(), 2);
    assert_eq!(set_hc1, set_hc2);

    let codes_dn1 = run_find_csv(
        Args {
            query: Some("daynight".to_string()),
            ..Default::default()
        },
        &parquet_dir,
    );
    let codes_dn2 = run_find_csv(
        Args {
            query: Some("day night".to_string()),
            ..Default::default()
        },
        &parquet_dir,
    );
    let set_dn1: HashSet<_> = codes_dn1.into_iter().collect();
    let set_dn2: HashSet<_> = codes_dn2.into_iter().collect();
    assert_eq!(set_dn1.len(), 2);
    assert_eq!(set_dn1, set_dn2);
}

#[test]
fn test_synthetic_task3_ordering_exact_matches_first() {
    let (_tmp, parquet_dir) = setup_name_matching_workspace();

    // l'arche: A3RE, AGX8, VM2JG, VN0KQ contain L'ARCHE; 8C872 and C7FV contain LARCHES
    let codes = run_find_csv(
        Args {
            query: Some("l'arche".to_string()),
            ..Default::default()
        },
        &parquet_dir,
    );
    assert_eq!(codes.len(), 6);
    let exact_set: HashSet<&str> = ["A3RE", "AGX8", "VM2JG", "VN0KQ"].into_iter().collect();
    for code in &codes[..4] {
        assert!(exact_set.contains(code.as_str()), "Top 4 must be exact matches, got {}", code);
    }
    assert_eq!(codes[4], "8C872");
    assert_eq!(codes[5], "C7FV");

    // st marys vs st mary's
    let codes_plain = run_find_csv(
        Args {
            query: Some("st marys".to_string()),
            ..Default::default()
        },
        &parquet_dir,
    );
    assert_eq!(codes_plain[0], "SM01", "st marys must rank SM01 (ST MARYS NH) first");

    let codes_apostrophe = run_find_csv(
        Args {
            query: Some("st mary's".to_string()),
            ..Default::default()
        },
        &parquet_dir,
    );
    assert_eq!(codes_apostrophe[0], "SM02", "st mary's must rank SM02 (ST MARY'S CLINIC) first");
}

#[test]
fn test_synthetic_task4_sql_output_and_duckdb_equivalence() {
    let (_tmp, parquet_dir) = setup_name_matching_workspace();

    let sql = run_find_sql(
        Args {
            query: Some("st marys".to_string()),
            ..Default::default()
        },
        &parquet_dir,
    );
    assert!(sql.contains("WHERE regexp_replace(name, '[^A-Z0-9]', '', 'g') LIKE '%STMARYS%'"));
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

#[test]
fn test_task5_help_text_contains_rule_verbatim() {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ods"));
    cmd.args(["find", "--help"]);
    let output = cmd.output().expect("execute ods find --help");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");

    assert!(
        stdout.contains("Spaces and punctuation are ignored. Results order exact matches first."),
        "Help text missing first rule line:\n{}",
        stdout
    );
    assert!(
        stdout.contains("\"healthcare\" and \"health care\" return the same result set in different orders."),
        "Help text missing second rule line:\n{}",
        stdout
    );
}

// =========================================================================
// Full Release Acceptance Tests (Requires ods_data/releases/2026-08-28)
// =========================================================================

#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_task1_squash_agrees_with_duckdb_sql_on_all_release_names() {
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

    let all_path = release_dir.join("orgs_all.parquet");
    let sql_all = format!("SELECT count(*) FROM '{}';", all_path.display());
    let total_all = run_duckdb_csv_codes(&sql_all);
    assert_eq!(total_all[0], "306201", "orgs_all.parquet has 306,201 rows");
}

#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_task1_in_london_and_role_ro76_unaffected() {
    let release_dir = get_release_dir().expect("requires ods_data/releases/2026-08-28");

    let codes_london = run_find_csv(
        Args {
            location: vec!["london".to_string()],
            ..Default::default()
        },
        &release_dir,
    );
    assert_eq!(codes_london.len(), 18374, "ods find --in london count must be 18,374");

    let codes_ro76 = run_find_csv(
        Args {
            role: vec!["RO76".to_string()],
            ..Default::default()
        },
        &release_dir,
    );
    assert_eq!(codes_ro76.len(), 7566, "ods find --role RO76 count must be 7,566");
}

#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_task2_acceptance_seven_counts_and_identical_row_sets() {
    let release_dir = get_release_dir().expect("requires ods_data/releases/2026-08-28");

    let queries_and_targets = [
        ("christchurch", 161),
        ("christ church", 161),
        ("homecare", 2880),
        ("daynight", 32),
        ("l'arche", 20),
        ("st marys", 577),
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

    // Monotonicity check
    assert!(cc1.len() >= 77, "No query returns fewer rows than before");
}

#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_task2_shown_failing_normalize_for_matching() {
    use ods::commands::find::normalize_for_matching;
    let release_dir = get_release_dir().expect("requires ods_data/releases/2026-08-28");

    let file = std::fs::File::open(release_dir.join("orgs.parquet")).unwrap();
    let builder = parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let reader = builder.build().unwrap();

    let norm_q = normalize_for_matching("christchurch");
    let mut old_count = 0;
    for batch in reader {
        let b = batch.unwrap();
        let name_arr = b.column(1).as_any().downcast_ref::<arrow::array::StringArray>().unwrap();
        for i in 0..b.num_rows() {
            if normalize_for_matching(name_arr.value(i)).contains(&norm_q) {
                old_count += 1;
            }
        }
    }
    assert_eq!(old_count, 77, "Demonstrate failure: normalize_for_matching returns 77 rather than 161");
}

#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_task3_ordering_positions_larche_and_st_marys() {
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

    // st marys: ST MARY STREET SURGERY at 465 of 577, TEDBURN ST MARY SCHOOL at 367
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
    assert_eq!(sm_rows.len(), 577);
    let pos_surgery = sm_rows.iter().position(|r| r.contains("ST MARY STREET SURGERY")).map(|p| p + 1);
    assert_eq!(pos_surgery, Some(465), "ST MARY STREET SURGERY must be at pos 465 of 577 for 'st marys'");
    let pos_school = sm_rows.iter().position(|r| r.contains("TEDBURN ST MARY SCHOOL")).map(|p| p + 1);
    assert_eq!(pos_school, Some(367), "TEDBURN ST MARY SCHOOL must be at pos 367 of 577 for 'st marys'");

    // st mary's: ST MARY STREET SURGERY at 527 of 577, TEDBURN ST MARY SCHOOL at 512
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
    assert_eq!(sma_rows.len(), 577);
    let pos_surgery_a = sma_rows.iter().position(|r| r.contains("ST MARY STREET SURGERY")).map(|p| p + 1);
    assert_eq!(pos_surgery_a, Some(527), "ST MARY STREET SURGERY must be at pos 527 of 577 for 'st mary\\'s'");
    let pos_school_a = sma_rows.iter().position(|r| r.contains("TEDBURN ST MARY SCHOOL")).map(|p| p + 1);
    assert_eq!(pos_school_a, Some(512), "TEDBURN ST MARY SCHOOL must be at pos 512 of 577 for 'st mary\\'s'");
}

#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_task3_shown_failing_drop_ordering_key() {
    let release_dir = get_release_dir().expect("requires ods_data/releases/2026-08-28");

    let codes_sorted = run_find_csv(
        Args {
            query: Some("l'arche".to_string()),
            sort: Some(ods::commands::find::SortBy::Code),
            ..Default::default()
        },
        &release_dir,
    );
    // When sorted strictly by code without exact match ranking:
    // 8C872 (LARCHES ROAD) is at index 0 (1st) rather than index 11 (12th)!
    assert_eq!(codes_sorted[0], "8C872", "Dropping ranking key puts 8C872 (LARCHES ROAD) first");
}

#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_task4_sql_duckdb_equivalence_four_queries() {
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
fn test_task4_shown_failing_drop_order_by_from_sql() {
    if !require_duckdb() {
        return;
    }
    let release_dir = get_release_dir().expect("requires ods_data/releases/2026-08-28");

    // Deliberately broken query: drop ORDER BY (name LIKE '%L''ARCHE%') DESC
    let broken_sql = format!(
        "SELECT * FROM '{}/orgs.parquet' WHERE regexp_replace(name, '[^A-Z0-9]', '', 'g') LIKE '%LARCHE%' ORDER BY ods_code;",
        release_dir.display()
    );
    let duck_codes = run_duckdb_csv_codes(&broken_sql);

    // In correct find output: row 3 is VM2JG (GLASFRYN (L'ARCHE)).
    // In broken DuckDB output without name LIKE ranking: row 1 is 8C872, row 2 is A3RE, row 3 is AGX8!
    assert_ne!(
        duck_codes[2], "VM2JG",
        "Dropping ORDER BY key causes l'arche to fail at row 3 (got AGX8 instead of VM2JG)"
    );
}

#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_task4_st_marys_sql_output_format() {
    let release_dir = get_release_dir().expect("requires ods_data/releases/2026-08-28");

    let sql = run_find_sql(
        Args {
            query: Some("st marys".to_string()),
            ..Default::default()
        },
        &release_dir,
    );

    let expected_suffix = "WHERE regexp_replace(name, '[^A-Z0-9]', '', 'g') LIKE '%STMARYS%'\nORDER BY (name LIKE '%ST MARYS%') DESC, ods_code;";
    assert!(
        sql.contains(expected_suffix),
        "SQL output must contain expected clause and order by, got:\n{}",
        sql
    );
}

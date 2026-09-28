use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

use ods::commands::find::{self, Args, OutputFormat, SortBy};
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
        if std::env::var("CI").as_deref() == Ok("true") || std::env::var("CI").as_deref() == Ok("1") {
            panic!("duckdb CLI is required in CI, but was not found on PATH");
        } else {
            eprintln!("Skipping test: duckdb CLI not found on PATH");
            return false;
        }
    }
    true
}

#[allow(clippy::too_many_arguments)]
fn make_test_record(
    ods_code: &str,
    name: &str,
    town: Option<&str>,
    county: Option<&str>,
    country: Option<&str>,
    postcode: Option<&str>,
    roles: Vec<(&str, &str)>, // (code, name)
    status: &str,
) -> OdsRecord {
    let ods_roles = roles
        .iter()
        .map(|(code, name)| OdsRole {
            id: code.to_string(),
            code: Some(code.to_string()),
            display_name: Some(name.to_string()),
            unique_role_id: format!("U_{}_{}", ods_code, code),
            primary_role: true,
            status: status.to_string(),
            dates: vec![],
        })
        .collect();

    let primary_role_name = roles.first().map(|(_, n)| *n).unwrap_or("Organisation");

    OdsRecord {
        ods_code: ods_code.to_string(),
        name: name.to_string(),
        status: status.to_string(),
        record_class: "org".to_string(),
        role: primary_role_name.to_string(),
        geo_loc: Some(Location {
            address_lines: vec!["1 Test Street".to_string()],
            town: town.map(|s| s.to_string()),
            county: county.map(|s| s.to_string()),
            country: country.map(|s| s.to_string()),
            postcode: postcode.map(|s| s.to_string()),
            uprn: None,
        }),
        roles: ods_roles,
        ..Default::default()
    }
}

fn setup_sql_workspace() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().expect("create temp dir");
    let parquet_dir = tmp.path().join("parquet");
    std::fs::create_dir_all(&parquet_dir).expect("create parquet dir");

    let records = vec![
        make_test_record(
            "A82608",
            "SEDBERGH MEDICAL PRACTICE",
            Some("SEDBERGH"),
            Some("CUMBRIA"),
            Some("ENGLAND"),
            Some("LA10 5DL"),
            vec![("RO76", "GP Practice"), ("RO177", "Prescribing Cost Centre")],
            "active",
        ),
        make_test_record(
            "8GJ58",
            "PARKER M JUNE (ACUPUNCURIST)",
            Some("SEDBERGH"),
            Some("CUMBRIA"),
            Some("ENGLAND"),
            Some("LA10 5AU"),
            vec![("RO157", "Non-NHS Organisation")],
            "active",
        ),
        make_test_record(
            "CC01",
            "CHRIST CHURCH CLINIC",
            Some("DURHAM"),
            Some("COUNTY DURHAM"),
            Some("ENGLAND"),
            Some("DH1 3YG"),
            vec![("RO76", "GP Practice")],
            "active",
        ),
        make_test_record(
            "CC02",
            "CHRISTCHURCH PHARMACY",
            Some("CHRISTCHURCH"),
            Some("DORSET"),
            Some("ENGLAND"),
            Some("BH23 1AA"),
            vec![("RO182", "Pharmacy")],
            "active",
        ),
        make_test_record(
            "SM01",
            "ST MARYS NH",
            Some("DURHAM"),
            Some("DURHAM"),
            Some("ENGLAND"),
            Some("DH1 5XZ"),
            vec![("RO76", "GP Practice")],
            "active",
        ),
        make_test_record(
            "SM02",
            "ST MARY'S CLINIC",
            Some("NEWCASTLE"),
            Some("TYNE AND WEAR"),
            Some("ENGLAND"),
            Some("NE1 1AA"),
            vec![("RO110", "Dental Practice")],
            "active",
        ),
        make_test_record(
            "SM03",
            "ST MARY'S CLINIC",
            Some("DURHAM"),
            Some("COUNTY DURHAM"),
            Some("ENGLAND"),
            Some("DH1 3YG"),
            vec![("RO110", "Dental Practice")],
            "active",
        ),
        make_test_record(
            "DN01",
            "DENTAL SMILES",
            Some("DOUGLAS"),
            Some("ISLE OF MAN"),
            Some("ISLE OF MAN"),
            Some("IM1 1AA"),
            vec![("RO65", "Private Dental Practice")],
            "active",
        ),
        make_test_record(
            "INACT1",
            "SEDBERGH OLD SURGERY",
            Some("SEDBERGH"),
            Some("CUMBRIA"),
            Some("ENGLAND"),
            Some("LA10 5ZZ"),
            vec![("RO76", "GP Practice")],
            "inactive",
        ),
    ];

    let edges = build_succession_edges(&records);
    let (succ_closures, pred_closures) = compute_transitive_closures(&records, &edges);
    let prov = ods::provenance::fixture_embedded("2026-08-28");

    export_orgs(&parquet_dir, &records, &succ_closures, &pred_closures, Some(&prov), "2026-08-28")
        .expect("export orgs");
    export_roles(&parquet_dir, &records, Some(&prov), "2026-08-28").expect("export roles");
    export_relationships(&parquet_dir, &records, Some(&prov), "2026-08-28").expect("export relationships");
    export_successions(&parquet_dir, &records, Some(&prov), "2026-08-28").expect("export successions");

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

    let stdout = String::from_utf8(out).expect("utf8 csv");
    stdout
        .lines()
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

fn run_find_sql_err(args: Args, parquet_dir: &Path) -> (String, String) {
    let mut out = Vec::new();
    let res = find::run_with_writer(
        Args {
            sql: true,
            input: Some(parquet_dir.to_path_buf()),
            ..args
        },
        &mut out,
        parquet_dir,
    );
    let stdout = String::from_utf8(out).unwrap_or_default();
    let stderr = match res {
        Ok(_) => String::new(),
        Err(e) => format!("{:#}", e),
    };
    (stdout, stderr)
}

fn run_duckdb_csv_codes(sql: &str) -> Vec<String> {
    let output = Command::new("duckdb")
        .args(["-csv", "-c", sql])
        .output()
        .expect("run duckdb");
    assert!(
        output.status.success(),
        "duckdb failed on SQL:\n{}\nstderr:\n{}",
        sql,
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("duckdb utf8");
    let mut lines = stdout.lines();
    let header = match lines.next() {
        Some(h) => h,
        None => return Vec::new(),
    };
    let code_idx = header
        .split(',')
        .position(|col| col == "ods_code")
        .unwrap_or(0);

    lines
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.split(',').nth(code_idx).unwrap_or("").to_string())
        .collect()
}

// -----------------------------------------------------------------------------
// Task 1: --sql validates like the reader
// -----------------------------------------------------------------------------

#[test]
fn test_sql_flag_validation_errors_match_query_reader() {
    let (_tmp, parquet_dir) = setup_sql_workspace();

    // 1. --role GP errors on both
    let (_, sql_err) = run_find_sql_err(
        Args {
            role: vec!["GP".to_string()],
            ..Default::default()
        },
        &parquet_dir,
    );
    let mut reader_out = Vec::new();
    let reader_res = find::run_with_writer(
        Args {
            role: vec!["GP".to_string()],
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut reader_out,
        &parquet_dir,
    );
    let reader_err = format!("{:#}", reader_res.unwrap_err());

    assert!(sql_err.contains("✖ No role named 'GP'"));
    assert!(sql_err.contains("Did you mean:"));
    assert!(sql_err.contains("Or use codes:"));
    assert_eq!(sql_err, reader_err);

    // 2. --in ab errors on both
    let (_, sql_in_err) = run_find_sql_err(
        Args {
            location: vec!["ab".to_string()],
            ..Default::default()
        },
        &parquet_dir,
    );
    let mut reader_in_out = Vec::new();
    let reader_in_res = find::run_with_writer(
        Args {
            location: vec!["ab".to_string()],
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut reader_in_out,
        &parquet_dir,
    );
    let reader_in_err = format!("{:#}", reader_in_res.unwrap_err());

    assert!(sql_in_err.contains("✖ Location query 'ab' is too short (minimum 3 characters)"));
    assert_eq!(sql_in_err, reader_in_err);

    // 3. --role "GP Practice" resolves to RO76
    let sql_gp_name = run_find_sql(
        Args {
            role: vec!["GP Practice".to_string()],
            ..Default::default()
        },
        &parquet_dir,
    );
    assert!(
        sql_gp_name.contains("list_has_any(role_codes, ['RO76'])"),
        "sql should contain resolved RO76, got:\n{}",
        sql_gp_name
    );
}

#[test]
fn test_find_sql_output_hygiene_and_isolation() {
    let (_tmp, parquet_dir) = setup_sql_workspace();

    // sedbergh --gp --sql | grep -c '^[*!]' returns 0
    let sql = run_find_sql(
        Args {
            query: Some("sedbergh".to_string()),
            gp: true,
            ..Default::default()
        },
        &parquet_dir,
    );
    let notice_lines = sql.lines().filter(|l| l.starts_with('*') || l.starts_with('!')).count();
    assert_eq!(notice_lines, 0, "No notice lines should appear in SQL output");

    // --format json with --sql prints the query, not JSON
    let sql_json = run_find_sql(
        Args {
            query: Some("sedbergh".to_string()),
            format: OutputFormat::Json,
            ..Default::default()
        },
        &parquet_dir,
    );
    assert!(sql_json.contains("SELECT *\nFROM"));
    assert!(!sql_json.trim_start().starts_with('{') && !sql_json.trim_start().starts_with('['));

    // bare ods find --sql prints unfiltered query and exits 0
    let sql_bare = run_find_sql(Args::default(), &parquet_dir);
    assert!(sql_bare.contains("SELECT *\nFROM"));
    assert!(sql_bare.contains("ORDER BY ods_code;"));
    assert!(sql_bare.contains(
        "WHERE status = 'active'\n  AND (legal_end IS NULL OR legal_end > publication_date)\nORDER BY ods_code;"
    ));
    assert!(!sql_bare.contains("No search filters given"));

    // ods find --sql reads no Parquet file (works on empty/non-existent parquet dir)
    let non_existent = parquet_dir.join("does_not_exist");
    let sql_no_file = run_find_sql(
        Args {
            query: Some("sedbergh".to_string()),
            ..Default::default()
        },
        &non_existent,
    );
    assert!(sql_no_file.contains("SELECT *"));
    assert!(sql_no_file.contains("regexp_replace(name, '[^A-Z0-9]', '', 'g') LIKE '%SEDBERGH%'"));
}

// -----------------------------------------------------------------------------
// Task 2: --code, --role and role shortcuts
// -----------------------------------------------------------------------------

#[test]
fn test_task2_clause_presence() {
    let (_tmp, parquet_dir) = setup_sql_workspace();

    // --code A82608
    let sql_code = run_find_sql(
        Args {
            code: vec!["A82608".to_string()],
            ..Default::default()
        },
        &parquet_dir,
    );
    assert!(sql_code.contains(
        "WHERE status = 'active'\n  AND (legal_end IS NULL OR legal_end > publication_date)\n  AND ods_code IN ('A82608')\nORDER BY ods_code;"
    ));

    // --code A82608,8GJ58
    let sql_multi_code = run_find_sql(
        Args {
            code: vec!["A82608".to_string(), "8GJ58".to_string()],
            ..Default::default()
        },
        &parquet_dir,
    );
    assert!(sql_multi_code.contains("AND ods_code IN ('A82608', '8GJ58')"));

    // --dentist
    let sql_dentist = run_find_sql(
        Args {
            dentist: true,
            ..Default::default()
        },
        &parquet_dir,
    );
    assert!(sql_dentist.contains("AND list_has_any(role_codes, ['RO110', 'RO65'])"));

    // sedbergh --gp
    let sql_sedbergh_gp = run_find_sql(
        Args {
            query: Some("sedbergh".to_string()),
            gp: true,
            ..Default::default()
        },
        &parquet_dir,
    );
    assert!(sql_sedbergh_gp.contains("AND regexp_replace(name, '[^A-Z0-9]', '', 'g') LIKE '%SEDBERGH%'"));
    assert!(sql_sedbergh_gp.contains("AND list_has_any(role_codes, ['RO76', 'RO227', 'RO315'])"));

    // --gp --all
    let sql_gp_all = run_find_sql(
        Args {
            gp: true,
            all: true,
            ..Default::default()
        },
        &parquet_dir,
    );
    assert!(sql_gp_all.contains("orgs.parquet"));
    assert!(!sql_gp_all.contains("status = 'active'"), "--all drops the status filter");
    assert!(sql_gp_all.contains("WHERE list_has_any(role_codes, ['RO76', 'RO227', 'RO315'])"));

    // Postcode clause shapes:
    // 1. outward(postcode) = 'SW1A' (sub-district)
    let sql_sw1a = run_find_sql(
        Args {
            location: vec!["SW1A".to_string()],
            ..Default::default()
        },
        &parquet_dir,
    );
    assert!(sql_sw1a.contains("OR outward(postcode) = 'SW1A')"));
    assert!(!sql_sw1a.contains("regexp_matches"));

    // 2. district(postcode) = 'SW1' (district)
    let sql_sw1 = run_find_sql(
        Args {
            location: vec!["SW1".to_string()],
            ..Default::default()
        },
        &parquet_dir,
    );
    assert!(sql_sw1.contains("OR district(postcode) = 'SW1')"));
    assert!(!sql_sw1.contains("regexp_matches"));
    assert!(!sql_sw1.contains("IN ("));

    // 3. postcode = 'LA10 5DL' (full postcode)
    let sql_full = run_find_sql(
        Args {
            location: vec!["LA105DL".to_string()],
            ..Default::default()
        },
        &parquet_dir,
    );
    assert!(sql_full.contains("OR postcode = 'LA10 5DL')"));
    assert!(!sql_full.contains("regexp_matches"));
    assert!(!sql_full.contains("inward"));
    assert!(!sql_full.contains("upper("));
}

#[test]
fn test_sql_flag_code_apostrophe_escaped() {
    let (_tmp, parquet_dir) = setup_sql_workspace();

    // --code "x'y"
    let sql_quote = run_find_sql(
        Args {
            code: vec!["x'y".to_string()],
            ..Default::default()
        },
        &parquet_dir,
    );
    assert!(sql_quote.contains("ods_code IN ('X''Y')"));

    if require_duckdb() {
        let codes = run_duckdb_csv_codes(&sql_quote);
        assert_eq!(codes.len(), 0);
    }
}

// -----------------------------------------------------------------------------
// Task 3: --in and the two macros
// -----------------------------------------------------------------------------

#[test]
fn test_sql_flag_macro_emission_rules_and_block_format() {
    let (_tmp, parquet_dir) = setup_sql_workspace();

    // --code A82608 --sql emits 0 macros
    let sql_code = run_find_sql(
        Args {
            code: vec!["A82608".to_string()],
            ..Default::default()
        },
        &parquet_dir,
    );
    assert_eq!(sql_code.lines().filter(|l| l.contains("MACRO")).count(), 0);

    // sedbergh --gp --sql emits 0 macros
    let sql_sedbergh_gp = run_find_sql(
        Args {
            query: Some("sedbergh".to_string()),
            gp: true,
            ..Default::default()
        },
        &parquet_dir,
    );
    assert_eq!(sql_sedbergh_gp.lines().filter(|l| l.contains("MACRO")).count(), 0);

    // --in durham --sql emits outward and district macros, and no inward macro, norm with DURHAM
    let sql_durham = run_find_sql(
        Args {
            location: vec!["durham".to_string()],
            ..Default::default()
        },
        &parquet_dir,
    );
    assert!(sql_durham.contains("CREATE OR REPLACE TEMP MACRO norm(s) AS"));
    assert!(sql_durham.contains("CREATE OR REPLACE TEMP MACRO outward(p) AS"));
    assert!(sql_durham.contains("CREATE OR REPLACE TEMP MACRO district(p) AS"));
    assert!(!sql_durham.contains("CREATE OR REPLACE TEMP MACRO inward(p) AS"));
    assert!(!sql_durham.contains("norm_postcode"));

    let expected_durham_block = "\
AND (   norm(town)    = 'DURHAM'\n \
       OR norm(county)  = 'DURHAM'\n \
       OR norm(country) = 'DURHAM')";
    assert!(
        sql_durham.contains(expected_durham_block),
        "Expected block format:\n{}\nGot:\n{}",
        expected_durham_block,
        sql_durham
    );

    // --in LA105DL --sql emits same 3 macros, no inward
    let sql_full_pc = run_find_sql(
        Args {
            location: vec!["LA105DL".to_string()],
            ..Default::default()
        },
        &parquet_dir,
    );
    assert!(sql_full_pc.contains("CREATE OR REPLACE TEMP MACRO outward(p) AS"));
    assert!(sql_full_pc.contains("CREATE OR REPLACE TEMP MACRO district(p) AS"));
    assert!(!sql_full_pc.contains("CREATE OR REPLACE TEMP MACRO inward(p) AS"));
}

// -----------------------------------------------------------------------------
// Task 4: Equivalence matrix and failure demonstrations
// -----------------------------------------------------------------------------

#[test]
fn test_task4_synthetic_ordered_equivalence_matrix() {
    if !require_duckdb() {
        return;
    }
    let (_tmp, parquet_dir) = setup_sql_workspace();

    let cases: Vec<(&str, Args)> = vec![
        ("bare query sedbergh", Args { query: Some("sedbergh".to_string()), ..Default::default() }),
        ("query st marys", Args { query: Some("st marys".to_string()), ..Default::default() }),
        ("query st mary's", Args { query: Some("st mary's".to_string()), ..Default::default() }),
        ("single code A82608", Args { code: vec!["A82608".to_string()], ..Default::default() }),
        ("multi code A82608,8GJ58", Args { code: vec!["A82608".to_string(), "8GJ58".to_string()], ..Default::default() }),
        ("role code RO76", Args { role: vec!["RO76".to_string()], ..Default::default() }),
        ("role name GP Practice", Args { role: vec!["GP Practice".to_string()], ..Default::default() }),
        ("alias gp", Args { gp: true, ..Default::default() }),
        ("alias dentist", Args { dentist: true, ..Default::default() }),
        ("in durham", Args { location: vec!["durham".to_string()], ..Default::default() }),
        ("in isle of man", Args { location: vec!["isle of man".to_string()], ..Default::default() }),
        ("in la10", Args { location: vec!["la10".to_string()], ..Default::default() }),
        ("repeated in durham cumbria", Args { location: vec!["durham".to_string(), "cumbria".to_string()], ..Default::default() }),
        ("all in durham", Args { all: true, location: vec!["durham".to_string()], ..Default::default() }),
        ("sort name", Args { sort: Some(SortBy::Name), query: Some("sedbergh".to_string()), ..Default::default() }),
        ("sort postcode", Args { sort: Some(SortBy::Postcode), query: Some("sedbergh".to_string()), ..Default::default() }),
        ("sort name ties", Args { sort: Some(SortBy::Name), query: Some("st".to_string()), ..Default::default() }),
        ("sort postcode ties", Args { sort: Some(SortBy::Postcode), location: vec!["durham".to_string()], ..Default::default() }),
        ("sort code", Args { sort: Some(SortBy::Code), query: Some("sedbergh".to_string()), ..Default::default() }),
        ("query sedbergh gp", Args { query: Some("sedbergh".to_string()), gp: true, ..Default::default() }),
        ("in durham gp", Args { location: vec!["durham".to_string()], gp: true, ..Default::default() }),
        ("query medical in durham", Args { query: Some("medical".to_string()), location: vec!["durham".to_string()], ..Default::default() }),
        ("bare sql unfiltered", Args::default()),
    ];

    for (desc, args) in cases {
        let csv_codes = run_find_csv(args.clone(), &parquet_dir);
        let sql = run_find_sql(args, &parquet_dir);
        let duck_codes = run_duckdb_csv_codes(&sql);

        assert_eq!(
            csv_codes, duck_codes,
            "Ordered equivalence mismatch for case '{}':\nods find csv: {:?}\nduckdb sql: {:?}",
            desc, csv_codes, duck_codes
        );
    }
}


#[test]
fn test_sql_flag_duckdb_missing_behavior() {
    // Check duckdb missing handling
    fn check_duckdb_path(is_avail: bool, is_ci: bool) -> Result<bool, &'static str> {
        if !is_avail {
            if is_ci {
                return Err("duckdb CLI is required in CI, but was not found on PATH");
            } else {
                return Ok(false); // skip
            }
        }
        Ok(true)
    }

    assert_eq!(check_duckdb_path(false, false), Ok(false)); // skips locally
    assert_eq!(check_duckdb_path(false, true), Err("duckdb CLI is required in CI, but was not found on PATH")); // fails in CI
    assert_eq!(check_duckdb_path(true, true), Ok(true));
}

// -----------------------------------------------------------------------------
// -----------------------------------------------------------------------------
// Full Release Tests (requires release 2026-08-28)
// -----------------------------------------------------------------------------

#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_sql_flag_release_filter_counts_match_on_release_data() {
    if !require_duckdb() {
        return;
    }
    let Some(release_dir) = get_release_dir() else {
        return;
    };

    let cases = vec![
        (Args { code: vec!["A82608".to_string()], ..Default::default() }, 1),
        (Args { dentist: true, ..Default::default() }, 9796),
        (Args { query: Some("sedbergh".to_string()), gp: true, ..Default::default() }, 1),
        (Args { gp: true, all: true, ..Default::default() }, 10524),
    ];

    for (args, expected_rows) in cases {
        let sql = run_find_sql(args, &release_dir);
        let codes = run_duckdb_csv_codes(&sql);
        assert_eq!(codes.len(), expected_rows);
    }
}

#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_sql_flag_macro_emission_counts_match_on_release_data() {
    if !require_duckdb() {
        return;
    }
    let Some(release_dir) = get_release_dir() else {
        return;
    };

    let cases = vec![
        (Args { location: vec!["durham".to_string()], ..Default::default() }, 674),
        (Args { location: vec!["isle of man".to_string()], ..Default::default() }, 201),
        (Args { location: vec!["la10".to_string()], ..Default::default() }, 14),
        (Args { location: vec!["durham".to_string()], gp: true, ..Default::default() }, 15),
        (Args { location: vec!["durham".to_string(), "cumbria".to_string()], ..Default::default() }, 1631),
    ];

    for (args, expected_rows) in cases {
        let sql = run_find_sql(args, &release_dir);
        let codes = run_duckdb_csv_codes(&sql);
        assert_eq!(codes.len(), expected_rows);
    }
}

#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_release_task4_equivalence_matrix() {
    if !require_duckdb() {
        return;
    }
    let Some(release_dir) = get_release_dir() else {
        return;
    };

    let cases: Vec<(&str, Args)> = vec![
        ("sedbergh", Args { query: Some("sedbergh".to_string()), ..Default::default() }),
        ("christchurch", Args { query: Some("christchurch".to_string()), ..Default::default() }),
        ("st marys", Args { query: Some("st marys".to_string()), ..Default::default() }),
        ("st mary's", Args { query: Some("st mary's".to_string()), ..Default::default() }),
        ("code A82608", Args { code: vec!["A82608".to_string()], ..Default::default() }),
        ("role RO76", Args { role: vec!["RO76".to_string()], ..Default::default() }),
        ("role GP Practice", Args { role: vec!["GP Practice".to_string()], ..Default::default() }),
        ("gp", Args { gp: true, ..Default::default() }),
        ("dentist", Args { dentist: true, ..Default::default() }),
        ("in durham", Args { location: vec!["durham".to_string()], ..Default::default() }),
        ("in isle of man", Args { location: vec!["isle of man".to_string()], ..Default::default() }),
        ("in la10", Args { location: vec!["la10".to_string()], ..Default::default() }),
        ("in durham in cumbria", Args { location: vec!["durham".to_string(), "cumbria".to_string()], ..Default::default() }),
        ("sort name", Args { sort: Some(SortBy::Name), query: Some("sedbergh".to_string()), ..Default::default() }),
        ("sort postcode", Args { sort: Some(SortBy::Postcode), query: Some("sedbergh".to_string()), ..Default::default() }),
        ("st marys sort name", Args { sort: Some(SortBy::Name), query: Some("st marys".to_string()), ..Default::default() }),
        ("st marys sort postcode", Args { sort: Some(SortBy::Postcode), query: Some("st marys".to_string()), ..Default::default() }),
        ("in durham sort name", Args { sort: Some(SortBy::Name), location: vec!["durham".to_string()], ..Default::default() }),
        ("in durham sort postcode", Args { sort: Some(SortBy::Postcode), location: vec!["durham".to_string()], ..Default::default() }),
        ("sedbergh gp", Args { query: Some("sedbergh".to_string()), gp: true, ..Default::default() }),
        ("in durham gp", Args { location: vec!["durham".to_string()], gp: true, ..Default::default() }),
        ("in W1", Args { location: vec!["W1".to_string()], ..Default::default() }),
        ("in EC1", Args { location: vec!["EC1".to_string()], ..Default::default() }),
        ("in E1", Args { location: vec!["E1".to_string()], ..Default::default() }),
        ("in N1", Args { location: vec!["N1".to_string()], ..Default::default() }),
        ("in SE1", Args { location: vec!["SE1".to_string()], ..Default::default() }),
        ("in SW1", Args { location: vec!["SW1".to_string()], ..Default::default() }),
        ("in LA1", Args { location: vec!["LA1".to_string()], ..Default::default() }),
        ("in 'W1,N1C'", Args { location: vec!["W1".to_string(), "N1C".to_string()], ..Default::default() }),
        ("in N1C", Args { location: vec!["N1C".to_string()], ..Default::default() }),
        ("in E1W", Args { location: vec!["E1W".to_string()], ..Default::default() }),
        ("in LA10", Args { location: vec!["LA10".to_string()], ..Default::default() }),
        ("in SW1A", Args { location: vec!["SW1A".to_string()], ..Default::default() }),
        ("in LA105DL", Args { location: vec!["LA105DL".to_string()], ..Default::default() }),
        ("in LA10 5DL", Args { location: vec!["LA10 5DL".to_string()], ..Default::default() }),
    ];

    for (desc, args) in cases {
        let csv_codes = run_find_csv(args.clone(), &release_dir);
        let sql = run_find_sql(args, &release_dir);
        let duck_codes = run_duckdb_csv_codes(&sql);

        assert_eq!(
            csv_codes, duck_codes,
            "Full release equivalence mismatch for case '{}': len {} vs {}",
            desc, csv_codes.len(), duck_codes.len()
        );
    }
}

#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_sql_flag_list_has_any_agrees_with_or_chain_on_release_data() {
    if !require_duckdb() {
        return;
    }
    let Some(release_dir) = get_release_dir() else {
        return;
    };

    let sql = format!(
        "SELECT count(*) \
         FROM '{}' \
         WHERE list_has_any(role_codes, ['RO76', 'RO227', 'RO315']) != \
               (list_contains(role_codes, 'RO76') OR list_contains(role_codes, 'RO227') OR list_contains(role_codes, 'RO315'));",
        release_dir.join("orgs.parquet").display()
    );

    let output = Command::new("duckdb")
        .args(["-csv", "-noheader", "-c", &sql])
        .output()
        .expect("run duckdb");
    assert!(output.status.success());
    let count: i64 = String::from_utf8(output.stdout)
        .expect("utf8")
        .trim()
        .parse()
        .expect("parse count");
    assert_eq!(count, 0, "list_has_any and OR list_contains disagree on rows");
}

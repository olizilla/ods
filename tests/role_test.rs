mod common;

use common::setup_find_test_workspace;
use ods::commands::role::{self, Args, OutputFormat};
use std::process::Command;

fn ods_binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ods"))
}

#[test]
fn test_role_lists_all_sorted_by_holders_desc() {
    let (_tmp, parquet_dir) = setup_find_test_workspace();

    let mut out = Vec::new();
    role::run_with_writer(
        Args {
            format: OutputFormat::Csv,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .expect("ods role should succeed");

    let s = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = s.lines().collect();

    let expected_count = ods::roles::role_names().names.len();
    assert_eq!(lines[0], "role_code,role_name,holders");
    assert_eq!(
        lines.len(),
        expected_count + 1,
        "ods role must list all curated roles"
    );

    // Verify descending order of holder counts
    let mut prev_count = usize::MAX;
    for line in &lines[1..] {
        let parts: Vec<&str> = line.split(',').collect();
        assert_eq!(parts.len(), 3, "CSV line must have 3 columns: {line}");
        let count: usize = parts[2].parse().unwrap();
        assert!(
            count <= prev_count,
            "Holder count must be sorted descending: prev {prev_count}, current {count}"
        );
        prev_count = count;
    }
}

#[test]
fn test_role_search_substring_filter() {
    let (_tmp, parquet_dir) = setup_find_test_workspace();

    let mut out = Vec::new();
    role::run_with_writer(
        Args {
            query: Some("dental".to_string()),
            format: OutputFormat::Csv,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .expect("ods role dental should succeed");

    let s = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = s.lines().collect();
    assert_eq!(lines[0], "role_code,role_name,holders");

    // Exactly 2 dental roles: RO110 and RO65
    assert_eq!(lines.len(), 3);
    assert!(lines[1].starts_with("RO110,General Dental Practice"));
    assert!(lines[2].starts_with("RO65,Private Dental Practice"));
}

#[test]
fn test_role_codes_flag_outputs_comma_separated() {
    let (_tmp, parquet_dir) = setup_find_test_workspace();

    // 1. Search term 'dental' with --codes
    let mut out = Vec::new();
    role::run_with_writer(
        Args {
            query: Some("dental".to_string()),
            codes: true,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .expect("ods role dental --codes should succeed");

    let s = String::from_utf8(out).unwrap();
    assert_eq!(s.trim(), "RO110,RO65");

    // 2. --codes wins over --format json
    let mut out_json = Vec::new();
    role::run_with_writer(
        Args {
            query: Some("dental".to_string()),
            codes: true,
            format: OutputFormat::Json,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out_json,
        &parquet_dir,
    )
    .expect("ods role dental --codes --format json should succeed");

    let s_json = String::from_utf8(out_json).unwrap();
    assert_eq!(
        s_json.trim(),
        "RO110,RO65",
        "--codes must win over --format json"
    );
}

#[test]
fn test_role_codes_all_when_no_query() {
    let (_tmp, parquet_dir) = setup_find_test_workspace();

    let mut out = Vec::new();
    role::run_with_writer(
        Args {
            codes: true,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .expect("ods role --codes should succeed");

    let s = String::from_utf8(out).unwrap();
    let codes: Vec<&str> = s.trim().split(',').collect();
    let expected_count = ods::roles::role_names().names.len();
    assert_eq!(
        codes.len(),
        expected_count,
        "ods role --codes without query must emit all curated role codes"
    );
}

#[test]
fn test_role_formats_table_markdown_csv_json() {
    let (_tmp, parquet_dir) = setup_find_test_workspace();

    // 1. Table format (default)
    let mut out_table = Vec::new();
    role::run_with_writer(
        Args {
            query: Some("dental".to_string()),
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out_table,
        &parquet_dir,
    )
    .unwrap();
    let s_table = String::from_utf8(out_table).unwrap();
    assert!(s_table.contains("Code"));
    assert!(s_table.contains("Name"));
    assert!(s_table.contains("Holders"));
    assert!(s_table.contains("RO110"));
    assert!(s_table.contains("General Dental Practice"));

    // 2. Markdown format
    let mut out_md = Vec::new();
    role::run_with_writer(
        Args {
            query: Some("dental".to_string()),
            format: OutputFormat::Markdown,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out_md,
        &parquet_dir,
    )
    .unwrap();
    let s_md = String::from_utf8(out_md).unwrap();
    assert!(s_md
        .lines()
        .any(|l| l.contains("Code") && l.contains("Name") && l.contains("Holders")));
    assert!(s_md.contains("RO110"));
    assert!(s_md.contains("General Dental Practice"));

    // 3. JSON format
    let mut out_json = Vec::new();
    role::run_with_writer(
        Args {
            query: Some("dental".to_string()),
            format: OutputFormat::Json,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out_json,
        &parquet_dir,
    )
    .unwrap();
    let val: serde_json::Value = serde_json::from_slice(&out_json).expect("valid JSON array");
    let arr = val.as_array().expect("array of objects");
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["role_code"], "RO110");
    assert_eq!(arr[0]["role_name"], "General Dental Practice");
    assert!(arr[0]["holders"].as_u64().unwrap() > 0);
}




fn count_role_codes_in_parquet(path: &std::path::Path) -> std::collections::HashMap<String, usize> {
    use arrow::array::{Array, ListArray, StringArray};
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    use std::fs::File;

    let file = File::open(path).expect("open parquet file");
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).expect("builder");
    let reader = builder.build().expect("reader");

    let mut counts = std::collections::HashMap::new();
    for batch in reader {
        let batch = batch.expect("batch");
        let schema = batch.schema();
        let idx = schema.index_of("role_codes").expect("role_codes col");
        let list_arr = batch
            .column(idx)
            .as_any()
            .downcast_ref::<ListArray>()
            .expect("ListArray");

        for i in 0..batch.num_rows() {
            if list_arr.is_valid(i) {
                let val_arr = list_arr.value(i);
                if let Some(str_arr) = val_arr.as_any().downcast_ref::<StringArray>() {
                    for j in 0..str_arr.len() {
                        if str_arr.is_valid(j) {
                            *counts.entry(str_arr.value(j).to_string()).or_insert(0) += 1;
                        }
                    }
                }
            }
        }
    }
    counts
}

#[test]
fn test_role_holders_counts_active_orgs_holding_role_actively() {
    let (_tmp, parquet_dir) = setup_find_test_workspace();

    // 1. Direct query of orgs.parquet vs orgs_all.parquet in test workspace
    let orgs_counts = count_role_codes_in_parquet(&parquet_dir.join("orgs.parquet"));
    let orgs_all_counts = count_role_codes_in_parquet(&parquet_dir.join("orgs_all.parquet"));

    let mut out = Vec::new();
    role::run_with_writer(
        Args {
            format: OutputFormat::Csv,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .expect("ods role --format csv should succeed");

    let s = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = s.lines().collect();
    assert_eq!(lines[0], "role_code,role_name,holders");

    let mut csv_map = std::collections::HashMap::new();
    for line in &lines[1..] {
        let parts: Vec<&str> = line.split(',').collect();
        let code = parts[0];
        let holders: usize = parts[2].parse().unwrap();
        let expected_orgs = orgs_counts.get(code).copied().unwrap_or(0);
        assert_eq!(
            holders, expected_orgs,
            "role {} holders must equal count from orgs.parquet",
            code
        );
        csv_map.insert(code.to_string(), holders);
    }

    // In the test fixture, RO76 has active holders in orgs.parquet and inactive holders in orgs_all.parquet.
    // Explicitly pin the synthetic fixture values (14 active vs 17 all) so this verification runs on CI.
    let ro76_active = orgs_counts.get("RO76").copied().unwrap_or(0);
    let ro76_all = orgs_all_counts.get("RO76").copied().unwrap_or(0);
    assert_eq!(ro76_active, 14, "synthetic fixture has 14 active RO76 holders in orgs.parquet");
    assert_eq!(ro76_all, 17, "synthetic fixture has 17 total RO76 records in orgs_all.parquet");
    assert_ne!(
        ro76_active, ro76_all,
        "RO76 fixture must have different counts between active and all orgs"
    );
    assert_eq!(
        csv_map.get("RO76"),
        Some(&14),
        "ods role must report 14 active RO76 holders (from orgs.parquet), not 17 (from orgs_all.parquet)"
    );
}

fn get_release_dir() -> Option<std::path::PathBuf> {
    let p = std::path::PathBuf::from("ods_data/releases/2026-08-28");
    if p.join("orgs.parquet").exists() {
        Some(p)
    } else {
        None
    }
}

#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_role_holders_counts_on_release_data() {
    let release_dir = get_release_dir().expect("requires ods_data/releases/2026-08-28");
    let mut out_real = Vec::new();
    role::run_with_writer(
        Args {
            format: OutputFormat::Csv,
            input: Some(release_dir.clone()),
            ..Default::default()
        },
        &mut out_real,
        &release_dir,
    )
    .expect("ods role on real data should succeed");

    let s_real = String::from_utf8(out_real).unwrap();
    let mut real_map = std::collections::HashMap::new();
    for line in s_real.lines().skip(1) {
        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() == 3 {
            real_map.insert(parts[0].to_string(), parts[2].parse::<usize>().unwrap());
        }
    }

    assert_eq!(
        real_map.get("RO198"),
        Some(&38256),
        "RO198 must have 38,256 active holders in 2026-08-28 release (not 49,575)"
    );
    assert_eq!(
        real_map.get("RO101"),
        Some(&31738),
        "RO101 must have 31,738 active holders in 2026-08-28 release (not 50,114)"
    );
}

#[test]
fn test_role_piped_output_has_no_ansi_escapes() {
    let (_tmp, parquet_dir) = setup_find_test_workspace();
    let output = ods_binary()
        .arg("role")
        .arg("dental")
        .arg("-i")
        .arg(&parquet_dir)
        .output()
        .expect("run ods role");
    assert!(
        output.status.success(),
        "status: {:?}, stderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("\x1b["),
        "piped output must not contain ANSI escape sequences, got:\n{}",
        stdout
    );
}


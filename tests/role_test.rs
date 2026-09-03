mod common;

use common::setup_find_test_workspace;
use ods::commands::role::{self, Args, OutputFormat};

#[test]
fn test_role_lists_all_205_sorted_by_holders_desc() {
    let (_tmp, parquet_dir) = setup_find_test_workspace();

    let mut out = Vec::new();
    role::run_with_writer(
        Args {
            query: None,
            codes: false,
            format: OutputFormat::Csv,
            input: Some(parquet_dir.clone()),
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
            codes: false,
            format: OutputFormat::Csv,
            input: Some(parquet_dir.clone()),
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
            format: OutputFormat::Table,
            input: Some(parquet_dir.clone()),
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
            query: None,
            codes: true,
            format: OutputFormat::Table,
            input: Some(parquet_dir.clone()),
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
            codes: false,
            format: OutputFormat::Table,
            input: Some(parquet_dir.clone()),
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
            codes: false,
            format: OutputFormat::Markdown,
            input: Some(parquet_dir.clone()),
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
            codes: false,
            format: OutputFormat::Json,
            input: Some(parquet_dir.clone()),
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

#[test]
fn test_role_zero_match_suggestions() {
    let (_tmp, parquet_dir) = setup_find_test_workspace();

    let mut out = Vec::new();
    let err = role::run_with_writer(
        Args {
            query: Some("general practice".to_string()),
            codes: false,
            format: OutputFormat::Table,
            input: Some(parquet_dir.clone()),
        },
        &mut out,
        &parquet_dir,
    )
    .unwrap_err();

    let err_msg = err.to_string();
    assert!(err_msg.contains("✖ No role matches 'general practice'"));
    assert!(err_msg.contains("Did you mean:"));
    assert!(err_msg.contains("GP Practice"));
}

#[test]
fn test_role_csv_escaping_guards() {
    let (_tmp, parquet_dir) = setup_find_test_workspace();

    let mut out_csv = Vec::new();
    role::run_with_writer(
        Args {
            query: None,
            codes: false,
            format: OutputFormat::Csv,
            input: Some(parquet_dir.clone()),
        },
        &mut out_csv,
        &parquet_dir,
    )
    .unwrap();

    let s_csv = String::from_utf8(out_csv).unwrap();
    let lines: Vec<&str> = s_csv.lines().collect();
    assert_eq!(lines[0], "role_code,role_name,holders");

    let expected_count = ods::roles::role_names().names.len();
    assert_eq!(lines.len(), expected_count + 1);

    for line in &lines[1..] {
        // Must start with RO
        assert!(
            line.starts_with("RO"),
            "CSV row must start with role code: {line}"
        );
        // Must end with a parseable integer holders count
        let last_comma = line
            .rfind(',')
            .expect("CSV line must contain at least one comma");
        let holders_str = &line[last_comma + 1..];
        let _: usize = holders_str.parse().expect("holders must be integer");
    }
}

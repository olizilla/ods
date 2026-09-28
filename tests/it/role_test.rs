use crate::common;

use common::{ods_binary, setup_find_test_workspace};
use ods::commands::role::{self, Args, OutputFormat};

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




fn count_role_codes_in_parquet(path: &std::path::Path, active_only: bool) -> std::collections::HashMap<String, usize> {
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
        let status_arr = batch
            .column(schema.index_of("status").expect("status col"))
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("status StringArray");

        for i in 0..batch.num_rows() {
            if active_only && status_arr.value(i) != "active" {
                continue;
            }
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

fn role_holders_csv(parquet_dir: &std::path::Path, all: bool) -> std::collections::HashMap<String, usize> {
    let mut out = Vec::new();
    role::run_with_writer(
        Args {
            format: OutputFormat::Csv,
            input: Some(parquet_dir.to_path_buf()),
            all,
            ..Default::default()
        },
        &mut out,
        parquet_dir,
    )
    .expect("ods role --format csv should succeed");

    let s = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = s.lines().collect();
    assert_eq!(lines[0], "role_code,role_name,holders");
    lines[1..]
        .iter()
        .map(|line| {
            let parts: Vec<&str> = line.split(',').collect();
            (parts[0].to_string(), parts[2].parse().unwrap())
        })
        .collect()
}

#[test]
fn role_counts_active_organisations_and_all_adds_the_rest() {
    let (_tmp, parquet_dir) = setup_find_test_workspace();
    let orgs = parquet_dir.join("orgs.parquet");

    let active_counts = count_role_codes_in_parquet(&orgs, true);
    let all_counts = count_role_codes_in_parquet(&orgs, false);

    // In the synthetic fixture RO76 has 14 active holders and 17 holders in all.
    assert_eq!(active_counts.get("RO76"), Some(&14));
    assert_eq!(all_counts.get("RO76"), Some(&17));

    let default_csv = role_holders_csv(&parquet_dir, false);
    for (code, holders) in &default_csv {
        assert_eq!(
            *holders,
            active_counts.get(code.as_str()).copied().unwrap_or(0),
            "role {code} holders must count active organisations only"
        );
    }
    assert_eq!(default_csv.get("RO76"), Some(&14));

    let all_csv = role_holders_csv(&parquet_dir, true);
    for (code, holders) in &all_csv {
        assert_eq!(
            *holders,
            all_counts.get(code.as_str()).copied().unwrap_or(0),
            "role {code} holders with --all must count every organisation"
        );
    }
    assert_eq!(all_csv.get("RO76"), Some(&17));
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
        Some(&35116),
        "RO198 (NHS Trust Site) drops from 38,256 active to 35,116 open once legally-closed \
         holders count as closed, not just not-yet-closed-in-every-sense"
    );
    assert_eq!(
        real_map.get("RO101"),
        Some(&31738),
        "RO101 (Social Care Site) is unchanged at 31,738: none of its holders are legally closed"
    );
    assert_eq!(
        real_map.get("RO76"),
        Some(&7566),
        "RO76 (GP Practice) is unchanged at 7,566: none of its holders are legally closed"
    );
    assert_eq!(
        real_map.get("RO197"),
        Some(&202),
        "RO197 (NHS Trust) drops from 247 active to 202 open, matching `ods find --role RO197`"
    );
}

/// `--all` is already the unfiltered table (`include_inactive: true`), a code path the open
/// filter never reaches, so it must count exactly what it counted before this brief.
#[test]
#[ignore = "requires full release 2026-08-28"]
fn test_role_all_is_unchanged_by_the_open_filter() {
    let release_dir = get_release_dir().expect("requires ods_data/releases/2026-08-28");
    let mut out = Vec::new();
    role::run_with_writer(
        Args {
            format: OutputFormat::Csv,
            all: true,
            input: Some(release_dir.clone()),
            ..Default::default()
        },
        &mut out,
        &release_dir,
    )
    .expect("ods role --all on real data should succeed");

    let s = String::from_utf8(out).unwrap();
    let mut counts = std::collections::HashMap::new();
    for line in s.lines().skip(1) {
        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() == 3 {
            counts.insert(parts[0].to_string(), parts[2].parse::<usize>().unwrap());
        }
    }

    for (code, expected) in [("RO198", 61592), ("RO76", 10112), ("RO197", 689), ("RO227", 1390), ("RO315", 321)] {
        assert_eq!(
            counts.get(code),
            Some(&expected),
            "{code} with --all must be unchanged from today"
        );
    }
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


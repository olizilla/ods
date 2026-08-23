use ods::commands::find::{self, Args, OutputFormat, SortBy};
use std::path::PathBuf;

#[test]
fn test_find_table_inactive_successor_display() {
    let parquet_dir = PathBuf::from("./ods_data/current");
    if !parquet_dir.join("orgs_all.parquet").exists() {
        eprintln!("Skipping test: ./ods_data/current/orgs_all.parquet missing");
        return;
    }

    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            query: Some("CLWYD".to_string()),
            code: Vec::new(),
            location: None,
            role: Vec::new(),
            all: true,
            verbose: false,
            sort: Some(SortBy::Code),
            format: OutputFormat::Table,
            input: parquet_dir.clone(),
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .expect("find::run_with_writer should succeed");

    let output_str = String::from_utf8(out).expect("valid UTF-8");
    assert!(
        output_str.contains("CLWYD → CONWY UA +4"),
        "Table view of CLWYD must show 'CLWYD → CONWY UA +4', got:\n{}",
        output_str
    );
}

#[test]
fn test_find_exact_code_matches_as_table() {
    let parquet_dir = PathBuf::from("./ods_data/current");
    if !parquet_dir.join("orgs.parquet").exists() {
        eprintln!("Skipping test: ./ods_data/current/orgs.parquet missing");
        return;
    }

    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            query: None,
            code: vec!["A82608".to_string()],
            location: None,
            role: Vec::new(),
            all: false,
            verbose: false,
            sort: Some(SortBy::Code),
            format: OutputFormat::Table,
            input: parquet_dir.clone(),
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .expect("find::run_with_writer should succeed");

    let output_str = String::from_utf8(out).expect("valid UTF-8");
    // find always returns table with column headers
    assert!(
        output_str.contains("ODS Code") && output_str.contains("Name"),
        "find must render table headers, got:\n{}",
        output_str
    );
    assert!(
        output_str.contains("A82608") && output_str.contains("SEDBERGH MEDICAL PRACTICE"),
        "find table must include A82608, got:\n{}",
        output_str
    );
}

#[test]
fn test_find_json_successor_codes_array() {
    let parquet_dir = PathBuf::from("./ods_data/current");
    if !parquet_dir.join("orgs_all.parquet").exists() {
        eprintln!("Skipping test: ./ods_data/current/orgs_all.parquet missing");
        return;
    }

    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            query: None,
            code: vec!["001".to_string()],
            location: None,
            role: Vec::new(),
            all: true,
            verbose: false,
            sort: Some(SortBy::Code),
            format: OutputFormat::Json,
            input: parquet_dir.clone(),
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .expect("find::run_with_writer should succeed");

    let output_str = String::from_utf8(out).expect("valid UTF-8");
    let mut found = false;
    for line in output_str.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line.trim()) {
            if json["ods_code"] == "001" {
                found = true;
                let successor_codes = json["successor_codes"].as_array().expect("successor_codes must be an array");
                assert_eq!(
                    successor_codes.len(),
                    5,
                    "successor_codes for 001 must have length 5, got: {:?}",
                    successor_codes
                );

                let expected_codes = vec!["016", "018", "020", "022", "024"];
                let actual_codes: Vec<&str> = successor_codes.iter().filter_map(|v| v.as_str()).collect();
                assert_eq!(actual_codes, expected_codes);
            }
        }
    }
    assert!(found, "Record 001 not found in JSON output:\n{}", output_str);
}

#[test]
fn test_find_csv_successor_codes_semicolon_list() {
    let parquet_dir = PathBuf::from("./ods_data/current");
    if !parquet_dir.join("orgs_all.parquet").exists() {
        eprintln!("Skipping test: ./ods_data/current/orgs_all.parquet missing");
        return;
    }

    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            query: None,
            code: vec!["001".to_string()],
            location: None,
            role: Vec::new(),
            all: true,
            verbose: false,
            sort: Some(SortBy::Code),
            format: OutputFormat::Csv,
            input: parquet_dir.clone(),
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .expect("find::run_with_writer should succeed");

    let output_str = String::from_utf8(out).expect("valid UTF-8");
    let mut found = false;
    for line in output_str.lines() {
        if line.starts_with("001,") {
            found = true;
            assert!(
                line.contains("016; 018; 020; 022; 024") || line.contains("016;018;020;022;024"),
                "CSV row for 001 must contain semicolon-separated successor codes, got line:\n{}",
                line
            );
        }
    }
    assert!(found, "Record 001 row not found in CSV output:\n{}", output_str);
}


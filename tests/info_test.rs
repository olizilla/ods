use ods::commands::info::{self, Args, OutputFormat};
use std::path::PathBuf;

#[test]
fn test_info_renders_full_detail_for_exact_code() {
    let parquet_dir = PathBuf::from("./ods_data/current");
    if !parquet_dir.join("orgs.parquet").exists() && !parquet_dir.join("orgs_all.parquet").exists() {
        eprintln!("Skipping test: parquet files missing in ./ods_data/current");
        return;
    }

    let mut out = Vec::new();
    info::run_with_writer(
        Args {
            ods_code: "A82608".to_string(),
            format: OutputFormat::Markdown,
            input: Some(parquet_dir.clone()),
        },
        &mut out,
        &parquet_dir,
    )
    .expect("info::run_with_writer should succeed for A82608");

    let output_str = String::from_utf8(out).expect("valid UTF-8");
    assert!(
        output_str.contains("# SEDBERGH MEDICAL PRACTICE (A82608)"),
        "Expected header '# SEDBERGH MEDICAL PRACTICE (A82608)', got:\n{}",
        output_str
    );
    assert!(
        output_str.contains("Class:") && output_str.contains("org"),
        "Expected 'Class: org', got:\n{}",
        output_str
    );
    assert!(
        output_str.contains("Status:") && output_str.contains("active"),
        "Expected 'Status: active', got:\n{}",
        output_str
    );
    assert!(
        output_str.contains("Primary Role:") && output_str.contains("Prescribing Cost Centre") && output_str.contains("RO177"),
        "Expected Primary Role with RO177, got:\n{}",
        output_str
    );
    assert!(
        output_str.contains("Other Roles:") && output_str.contains("GP Practice") && output_str.contains("RO76"),
        "Expected Other Roles with GP Practice (RO76), got:\n{}",
        output_str
    );
    assert!(
        output_str.contains("LA10 5DL"),
        "Expected postcode LA10 5DL, got:\n{}",
        output_str
    );
}

#[test]
fn test_info_json_format_and_succession_chain() {
    let parquet_dir = PathBuf::from("./ods_data/current");
    if !parquet_dir.join("orgs_all.parquet").exists() {
        eprintln!("Skipping test: ./ods_data/current/orgs_all.parquet missing");
        return;
    }

    let mut out = Vec::new();
    info::run_with_writer(
        Args {
            ods_code: "0AF".to_string(),
            format: OutputFormat::Json,
            input: Some(parquet_dir.clone()),
        },
        &mut out,
        &parquet_dir,
    )
    .expect("info::run_with_writer should succeed for 0AF with JSON");

    let output_str = String::from_utf8(out).expect("valid UTF-8");
    let json: serde_json::Value = serde_json::from_str(&output_str).expect("valid JSON output");

    assert_eq!(json["ods_code"], "0AF");
    assert_eq!(json["status"], "inactive");

    let succ = json["succession"].as_array().expect("succession must be array");
    assert_eq!(succ.len(), 3, "Expected 3 succession hops for 0AF, got: {:?}", succ);

    assert_eq!(succ[0]["code"], "0CE");
    assert_eq!(succ[0]["depth"], 1);
    assert_eq!(succ[0]["date"], "2012-10-01");

    assert_eq!(succ[1]["code"], "0CY");
    assert_eq!(succ[1]["depth"], 2);
    assert_eq!(succ[1]["date"], "2014-10-01");

    assert_eq!(succ[2]["code"], "YDDTR");
    assert_eq!(succ[2]["depth"], 3);
    assert_eq!(succ[2]["date"], "2016-04-01");
}

#[test]
fn test_info_case_insensitive() {
    let parquet_dir = PathBuf::from("./ods_data/current");
    if !parquet_dir.join("orgs.parquet").exists() && !parquet_dir.join("orgs_all.parquet").exists() {
        eprintln!("Skipping test: parquet files missing in ./ods_data/current");
        return;
    }

    let mut out = Vec::new();
    info::run_with_writer(
        Args {
            ods_code: "a82608".to_string(),
            format: OutputFormat::Markdown,
            input: Some(parquet_dir.clone()),
        },
        &mut out,
        &parquet_dir,
    )
    .expect("info::run_with_writer should succeed for lowercase a82608");

    let output_str = String::from_utf8(out).expect("valid UTF-8");
    assert!(
        output_str.contains("A82608"),
        "Expected A82608 in output, got:\n{}",
        output_str
    );
}

#[test]
fn test_info_nonexistent_code_fails() {
    let parquet_dir = PathBuf::from("./ods_data/current");
    if !parquet_dir.join("orgs.parquet").exists() && !parquet_dir.join("orgs_all.parquet").exists() {
        eprintln!("Skipping test: parquet files missing in ./ods_data/current");
        return;
    }

    let mut out = Vec::new();
    let result = info::run_with_writer(
        Args {
            ods_code: "NONEXISTENT999".to_string(),
            format: OutputFormat::Markdown,
            input: Some(parquet_dir.clone()),
        },
        &mut out,
        &parquet_dir,
    );

    assert!(result.is_err(), "Expected error for nonexistent ODS code");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("NONEXISTENT999"),
        "Expected error message to contain 'NONEXISTENT999', got: {}",
        err_msg
    );
}

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

#[test]
fn test_info_operates_section_in_markdown() {
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
        output_str.contains("## Operates"),
        "Expected '## Operates' section in Markdown output, got:\n{}",
        output_str
    );
    assert!(
        output_str.contains("A82608001") && output_str.contains("DR LUMB W & PARTNER"),
        "Expected 'A82608001 DR LUMB W & PARTNER' in Operates, got:\n{}",
        output_str
    );
}

#[test]
fn test_info_operates_array_in_json() {
    let parquet_dir = PathBuf::from("./ods_data/current");
    if !parquet_dir.join("orgs.parquet").exists() && !parquet_dir.join("orgs_all.parquet").exists() {
        eprintln!("Skipping test: parquet files missing in ./ods_data/current");
        return;
    }

    let mut out = Vec::new();
    info::run_with_writer(
        Args {
            ods_code: "A82608".to_string(),
            format: OutputFormat::Json,
            input: Some(parquet_dir.clone()),
        },
        &mut out,
        &parquet_dir,
    )
    .expect("info::run_with_writer should succeed for A82608 in JSON");

    let output_str = String::from_utf8(out).expect("valid UTF-8");
    let json: serde_json::Value = serde_json::from_str(&output_str).expect("valid JSON");

    let operates = json["operates"].as_array().expect("operates must be an array");
    assert_eq!(operates.len(), 1);
    assert_eq!(operates[0]["code"], "A82608001");
    assert_eq!(operates[0]["name"], "DR LUMB W & PARTNER");
    assert!(operates[0].get("status").is_none(), "status field must be omitted from OperatedEntityJson");
}

#[test]
fn test_info_trust_operates_multiple_entities() {
    let parquet_dir = PathBuf::from("./ods_data/current");
    if !parquet_dir.join("orgs.parquet").exists() && !parquet_dir.join("orgs_all.parquet").exists() {
        eprintln!("Skipping test: parquet files missing in ./ods_data/current");
        return;
    }

    let mut out = Vec::new();
    info::run_with_writer(
        Args {
            ods_code: "RJZ".to_string(),
            format: OutputFormat::Json,
            input: Some(parquet_dir.clone()),
        },
        &mut out,
        &parquet_dir,
    )
    .expect("info::run_with_writer should succeed for RJZ in JSON");

    let output_str = String::from_utf8(out).expect("valid UTF-8");
    let json: serde_json::Value = serde_json::from_str(&output_str).expect("valid JSON");

    let operates = json["operates"].as_array().expect("operates must be an array");
    // Count is active-only (filtering out closed COVID clinics and retired sites)
    assert!(
        operates.len() >= 10,
        "RJZ must operate multiple active hospital sites and clinics (active only), got: {}",
        operates.len()
    );
    assert!(operates.iter().any(|e| e["code"] == "RJZ01"), "Active hospital site RJZ01 must be present");
    assert!(!operates.iter().any(|e| e["code"] == "RJZ06"), "Inactive site RJZ06 must be filtered out");
}

#[test]
fn test_info_entity_without_operates_omits_section() {
    let parquet_dir = PathBuf::from("./ods_data/current");
    if !parquet_dir.join("orgs.parquet").exists() && !parquet_dir.join("orgs_all.parquet").exists() {
        eprintln!("Skipping test: parquet files missing in ./ods_data/current");
        return;
    }

    let mut out = Vec::new();
    info::run_with_writer(
        Args {
            ods_code: "A82608001".to_string(),
            format: OutputFormat::Markdown,
            input: Some(parquet_dir.clone()),
        },
        &mut out,
        &parquet_dir,
    )
    .expect("info::run_with_writer should succeed for A82608001");

    let output_str = String::from_utf8(out).expect("valid UTF-8");
    assert!(
        !output_str.contains("## Operates"),
        "Leaf entity A82608001 must not have '## Operates' section, got:\n{}",
        output_str
    );
}



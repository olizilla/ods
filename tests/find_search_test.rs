use ods::commands::find::{self, Args, OutputFormat, SortBy};
use std::path::PathBuf;

fn get_parquet_dir() -> PathBuf {
    PathBuf::from("./ods_data/current")
}

#[test]
fn test_find_searches_name_only() {
    let parquet_dir = get_parquet_dir();
    if !parquet_dir.join("orgs.parquet").exists() {
        eprintln!("Skipping test: parquet files missing in ./ods_data/current");
        return;
    }

    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            query: Some("Sedbergh".to_string()),
            code: Vec::new(),
            location: None,
            role: Vec::new(),
            all: false,
            verbose: false,
            sort: None,
            format: OutputFormat::Table,
            input: parquet_dir.clone(),
        },
        &mut out,
        &parquet_dir,
    )
    .expect("find::run_with_writer should succeed");

    let output_str = String::from_utf8(out).expect("valid UTF-8");
    // Should match "SEDBERGH MEDICAL PRACTICE"
    assert!(output_str.contains("SEDBERGH MEDICAL PRACTICE"));
}

#[test]
fn test_find_in_location_precedence_county_over_town() {
    let parquet_dir = get_parquet_dir();
    if !parquet_dir.join("orgs.parquet").exists() {
        eprintln!("Skipping test: parquet files missing in ./ods_data/current");
        return;
    }

    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            query: None,
            code: Vec::new(),
            location: Some("Surrey".to_string()),
            role: Vec::new(),
            all: false,
            verbose: false,
            sort: None,
            format: OutputFormat::Table,
            input: parquet_dir.clone(),
        },
        &mut out,
        &parquet_dir,
    )
    .expect("find::run_with_writer should succeed for --in Surrey");

    let output_str = String::from_utf8(out).expect("valid UTF-8");
    assert!(
        output_str.contains("* matched county"),
        "Expected Surrey to match county level, got:\n{}",
        output_str
    );
}

#[test]
fn test_find_in_postcode_stripping_spaces() {
    let parquet_dir = get_parquet_dir();
    if !parquet_dir.join("orgs.parquet").exists() {
        eprintln!("Skipping test: parquet files missing in ./ods_data/current");
        return;
    }

    let mut out_spaced = Vec::new();
    find::run_with_writer(
        Args {
            query: None,
            code: Vec::new(),
            location: Some("LA10 5DL".to_string()),
            role: Vec::new(),
            all: false,
            verbose: false,
            sort: None,
            format: OutputFormat::Table,
            input: parquet_dir.clone(),
        },
        &mut out_spaced,
        &parquet_dir,
    )
    .expect("find::run_with_writer with space in postcode should succeed");

    let mut out_unspaced = Vec::new();
    find::run_with_writer(
        Args {
            query: None,
            code: Vec::new(),
            location: Some("LA105DL".to_string()),
            role: Vec::new(),
            all: false,
            verbose: false,
            sort: None,
            format: OutputFormat::Table,
            input: parquet_dir.clone(),
        },
        &mut out_unspaced,
        &parquet_dir,
    )
    .expect("find::run_with_writer without space in postcode should succeed");

    let s1 = String::from_utf8(out_spaced).unwrap();
    let s2 = String::from_utf8(out_unspaced).unwrap();

    assert!(s1.contains("SEDBERGH MEDICAL PRACTICE"));
    assert!(s2.contains("SEDBERGH MEDICAL PRACTICE"));
    assert_eq!(s1, s2, "Spaced and unspaced postcode queries must produce identical results");
}

#[test]
fn test_find_in_rejects_under_3_chars() {
    let parquet_dir = get_parquet_dir();
    if !parquet_dir.join("orgs.parquet").exists() {
        eprintln!("Skipping test: parquet files missing in ./ods_data/current");
        return;
    }

    let mut out = Vec::new();
    let result = find::run_with_writer(
        Args {
            query: None,
            code: Vec::new(),
            location: Some("M".to_string()),
            role: Vec::new(),
            all: false,
            verbose: false,
            sort: None,
            format: OutputFormat::Table,
            input: parquet_dir.clone(),
        },
        &mut out,
        &parquet_dir,
    );

    assert!(result.is_err(), "Expected error for --in with < 3 chars");
    let err = result.unwrap_err().to_string();
    assert!(err.contains("too short"), "Expected 'too short' in error: {}", err);
}

#[test]
fn test_find_in_three_char_towns_ely_and_ayr() {
    let parquet_dir = get_parquet_dir();
    if !parquet_dir.join("orgs.parquet").exists() {
        eprintln!("Skipping test: parquet files missing in ./ods_data/current");
        return;
    }

    let mut out_ely = Vec::new();
    find::run_with_writer(
        Args {
            query: None,
            code: Vec::new(),
            location: Some("ELY".to_string()),
            role: Vec::new(),
            all: false,
            verbose: false,
            sort: None,
            format: OutputFormat::Table,
            input: parquet_dir.clone(),
        },
        &mut out_ely,
        &parquet_dir,
    )
    .expect("find --in ELY should succeed");

    let s_ely = String::from_utf8(out_ely).unwrap();
    assert!(s_ely.contains("matched town"), "Expected ELY to match town, got:\n{}", s_ely);

    let mut out_ayr = Vec::new();
    find::run_with_writer(
        Args {
            query: None,
            code: Vec::new(),
            location: Some("AYR".to_string()),
            role: Vec::new(),
            all: false,
            verbose: false,
            sort: None,
            format: OutputFormat::Table,
            input: parquet_dir.clone(),
        },
        &mut out_ayr,
        &parquet_dir,
    )
    .expect("find --in AYR should succeed");

    let s_ayr = String::from_utf8(out_ayr).unwrap();
    assert!(
        s_ayr.contains("matched county") || s_ayr.contains("matched town"),
        "Expected AYR to match county or town, got:\n{}",
        s_ayr
    );
    assert!(s_ayr.contains("BOOTS OPTICIANS (AYR)"));
}

#[test]
fn test_find_in_unknown_place_errors() {
    let parquet_dir = get_parquet_dir();
    if !parquet_dir.join("orgs.parquet").exists() {
        eprintln!("Skipping test: parquet files missing in ./ods_data/current");
        return;
    }

    let mut out = Vec::new();
    let result = find::run_with_writer(
        Args {
            query: None,
            code: Vec::new(),
            location: Some("Narnia".to_string()),
            role: Vec::new(),
            all: false,
            verbose: false,
            sort: None,
            format: OutputFormat::Table,
            input: parquet_dir.clone(),
        },
        &mut out,
        &parquet_dir,
    );

    assert!(result.is_err(), "Expected error for unknown location --in Narnia");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("No organisation found in 'Narnia'"),
        "Expected error message for Narnia, got: {}",
        err
    );
}

#[test]
fn test_find_code_flag_exact_lists() {
    let parquet_dir = get_parquet_dir();
    if !parquet_dir.join("orgs.parquet").exists() {
        eprintln!("Skipping test: parquet files missing in ./ods_data/current");
        return;
    }

    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            query: None,
            code: vec!["A82608".to_string(), "RJZ".to_string()],
            location: None,
            role: Vec::new(),
            all: false,
            verbose: false,
            sort: None,
            format: OutputFormat::Table,
            input: parquet_dir.clone(),
        },
        &mut out,
        &parquet_dir,
    )
    .expect("find --code should succeed");

    let s = String::from_utf8(out).unwrap();
    assert!(s.contains("A82608"));
    assert!(s.contains("RJZ"));
    // Prefix A82608001 must NOT match exact --code A82608
    assert!(!s.contains("A82608001"), "Exact code search must not prefix-match A82608001");
}

#[test]
fn test_find_normalization_apostrophes_and_punctuation() {
    let parquet_dir = get_parquet_dir();
    if !parquet_dir.join("orgs.parquet").exists() {
        eprintln!("Skipping test: parquet files missing in ./ods_data/current");
        return;
    }

    let mut out_plain = Vec::new();
    find::run_with_writer(
        Args {
            query: Some("kings college".to_string()),
            code: Vec::new(),
            location: None,
            role: Vec::new(),
            all: false,
            verbose: false,
            sort: None,
            format: OutputFormat::Table,
            input: parquet_dir.clone(),
        },
        &mut out_plain,
        &parquet_dir,
    )
    .expect("kings college query should succeed");

    let mut out_apostrophe = Vec::new();
    find::run_with_writer(
        Args {
            query: Some("king's college".to_string()),
            code: Vec::new(),
            location: None,
            role: Vec::new(),
            all: false,
            verbose: false,
            sort: None,
            format: OutputFormat::Table,
            input: parquet_dir.clone(),
        },
        &mut out_apostrophe,
        &parquet_dir,
    )
    .expect("king's college query should succeed");

    let s1 = String::from_utf8(out_plain).unwrap();
    let s2 = String::from_utf8(out_apostrophe).unwrap();

    assert_eq!(s1, s2, "kings college and king's college queries must produce identical results");
}

#[test]
fn test_find_ranking_exact_prefix_word_substring() {
    let parquet_dir = get_parquet_dir();
    if !parquet_dir.join("orgs.parquet").exists() {
        eprintln!("Skipping test: parquet files missing in ./ods_data/current");
        return;
    }

    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            query: Some("royal free".to_string()),
            code: Vec::new(),
            location: None,
            role: Vec::new(),
            all: false,
            verbose: false,
            sort: None,
            format: OutputFormat::Table,
            input: parquet_dir.clone(),
        },
        &mut out,
        &parquet_dir,
    )
    .expect("royal free query should succeed");

    let s = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = s.lines().filter(|l| l.starts_with('|') && !l.contains("ODS Code") && !l.contains("---")).collect();
    assert!(!lines.is_empty(), "Expected results for royal free");
    // The top line should be a prefix/exact match like ROYAL FREE ...
    assert!(
        lines[0].contains("ROYAL FREE"),
        "Top ranked match must start with ROYAL FREE, got: {}",
        lines[0]
    );
}

#[test]
fn test_find_explicit_sort_overrides_ranking() {
    let parquet_dir = get_parquet_dir();
    if !parquet_dir.join("orgs.parquet").exists() {
        eprintln!("Skipping test: parquet files missing in ./ods_data/current");
        return;
    }

    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            query: Some("royal free".to_string()),
            code: Vec::new(),
            location: None,
            role: Vec::new(),
            all: false,
            verbose: false,
            sort: Some(SortBy::Code),
            format: OutputFormat::Table,
            input: parquet_dir.clone(),
        },
        &mut out,
        &parquet_dir,
    )
    .expect("query with explicit --sort code should succeed");

    let s = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = s.lines().filter(|l| l.starts_with('|') && !l.contains("ODS Code") && !l.contains("---")).collect();
    assert!(lines.len() >= 2);
    // When sorted strictly by code, first column codes must be monotonically increasing
    let code1 = lines[0].split('|').nth(1).unwrap().trim();
    let code2 = lines[1].split('|').nth(1).unwrap().trim();
    assert!(code1 <= code2, "Expected code order {} <= {}", code1, code2);
}

#[test]
fn test_find_role_flag_codes_and_names() {
    let parquet_dir = get_parquet_dir();
    if !parquet_dir.join("orgs.parquet").exists() {
        eprintln!("Skipping test: parquet files missing in ./ods_data/current");
        return;
    }

    let mut out_code = Vec::new();
    find::run_with_writer(
        Args {
            query: None,
            code: Vec::new(),
            location: None,
            role: vec!["RO76".to_string(), "RO227".to_string()],
            all: false,
            verbose: false,
            sort: None,
            format: OutputFormat::Table,
            input: parquet_dir.clone(),
        },
        &mut out_code,
        &parquet_dir,
    )
    .expect("--role with codes should succeed");

    let s_code = String::from_utf8(out_code).unwrap();
    assert!(s_code.contains("GP Practice") || s_code.contains("SURGERY") || s_code.contains("MEDICAL"));

    let mut out_name = Vec::new();
    find::run_with_writer(
        Args {
            query: None,
            code: Vec::new(),
            location: None,
            role: vec!["GP Practice".to_string()],
            all: false,
            verbose: false,
            sort: None,
            format: OutputFormat::Table,
            input: parquet_dir.clone(),
        },
        &mut out_name,
        &parquet_dir,
    )
    .expect("--role with name should succeed");

    let s_name = String::from_utf8(out_name).unwrap();
    assert!(s_name.contains("GP Practice") || s_name.contains("SURGERY") || s_name.contains("MEDICAL"));
}

#[test]
fn test_find_role_typo_gives_suggestions() {
    let parquet_dir = get_parquet_dir();
    if !parquet_dir.join("orgs.parquet").exists() {
        eprintln!("Skipping test: parquet files missing in ./ods_data/current");
        return;
    }

    let mut out = Vec::new();
    let result = find::run_with_writer(
        Args {
            query: None,
            code: Vec::new(),
            location: None,
            role: vec!["General Practice".to_string()],
            all: false,
            verbose: false,
            sort: None,
            format: OutputFormat::Table,
            input: parquet_dir.clone(),
        },
        &mut out,
        &parquet_dir,
    );

    assert!(result.is_err(), "Expected error for invalid role name 'General Practice'");
    let err = result.unwrap_err().to_string();
    assert!(err.contains("No role named 'General Practice'"), "Error must contain 'No role named': {}", err);
    assert!(err.contains("Did you mean:"), "Error must contain suggestions: {}", err);
    assert!(err.contains("GP Practice"), "Suggestions must include GP Practice: {}", err);
    assert!(err.contains("Or use codes:"), "Error must contain code suggestion: {}", err);
}

#[test]
fn test_find_code_hint_for_inactive_code() {
    let parquet_dir = get_parquet_dir();
    if !parquet_dir.join("orgs.parquet").exists() {
        eprintln!("Skipping test: parquet files missing in ./ods_data/current");
        return;
    }

    let mut out = Vec::new();
    find::run_with_writer(
        Args {
            query: Some("FAH".to_string()),
            code: Vec::new(),
            location: None,
            role: Vec::new(),
            all: false,
            verbose: false,
            sort: None,
            format: OutputFormat::Table,
            input: parquet_dir.clone(),
        },
        &mut out,
        &parquet_dir,
    )
    .expect("query FAH should succeed");

    let s = String::from_utf8(out).unwrap();
    assert!(
        s.contains("* 'FAH' is also an ODS code — ods info FAH"),
        "Expected code hint for FAH, got:\n{}",
        s
    );
}

#[test]
fn test_low_signal_roles_whole_set_containment_on_orgs_all() {
    let parquet_dir = get_parquet_dir();
    if !parquet_dir.join("orgs_all.parquet").exists() {
        eprintln!("Skipping test: ./ods_data/current/orgs_all.parquet missing");
        return;
    }

    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    use arrow::array::{Array, ListArray, StringArray};
    use std::fs::File;

    let file = File::open(parquet_dir.join("orgs_all.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let reader = builder.build().unwrap();

    let cfg = ods::roles::role_display_config();
    let low_signal_set: std::collections::HashSet<&str> = cfg.low_signal.keys().map(|s| s.as_str()).collect();

    // Verify each low-signal entry has a reason
    for (code, entry) in &cfg.low_signal {
        assert!(!entry.why.is_empty(), "low-signal role {} must have a recorded reason", code);
    }

    let mut all_low_signal_multi_role_count = 0;
    let mut active_all_low_signal_count = 0;

    for batch in reader {
        let batch = batch.unwrap();
        let schema = batch.schema();
        let status_arr = batch.column(schema.index_of("status").unwrap()).as_any().downcast_ref::<StringArray>().unwrap();
        let role_codes_arr = batch.column(schema.index_of("role_codes").unwrap()).as_any().downcast_ref::<ListArray>().unwrap();

        for i in 0..batch.num_rows() {
            let status = status_arr.value(i);
            let mut role_codes: Vec<String> = Vec::new();
            if role_codes_arr.is_valid(i) {
                let value_arr = role_codes_arr.value(i);
                if let Some(str_arr) = value_arr.as_any().downcast_ref::<StringArray>() {
                    for j in 0..str_arr.len() {
                        if str_arr.is_valid(j) {
                            role_codes.push(str_arr.value(j).to_string());
                        }
                    }
                }
            }

            let is_all_low_signal = !role_codes.is_empty() && role_codes.iter().all(|c| low_signal_set.contains(c.as_str()));

            if is_all_low_signal {
                if role_codes.len() > 1 {
                    all_low_signal_multi_role_count += 1;
                }
                if status.eq_ignore_ascii_case("active") {
                    active_all_low_signal_count += 1;
                }
            }
        }
    }

    assert_eq!(
        active_all_low_signal_count, 0,
        "No active organisation in orgs.parquet/orgs_all.parquet may have its entire role set in low_signal"
    );
    assert_eq!(
        all_low_signal_multi_role_count, 0,
        "No multi-role organisation in orgs_all.parquet may have its entire role set in low_signal"
    );
}

#[test]
fn test_find_roles_display_deemphasis_and_verbose() {
    let parquet_dir = get_parquet_dir();
    if !parquet_dir.join("orgs.parquet").exists() {
        eprintln!("Skipping test: parquet files missing in ./ods_data/current");
        return;
    }

    // 1. Default non-verbose: GP Practice +1 for Sedbergh (RO177 + RO76)
    let mut out_default = Vec::new();
    find::run_with_writer(
        Args {
            query: Some("SEDBERGH MEDICAL PRACTICE".to_string()),
            code: Vec::new(),
            location: None,
            role: Vec::new(),
            all: false,
            verbose: false,
            sort: None,
            format: OutputFormat::Table,
            input: parquet_dir.clone(),
        },
        &mut out_default,
        &parquet_dir,
    ).unwrap();
    let s_default = String::from_utf8(out_default).unwrap();
    assert!(
        s_default.contains("GP Practice +1"),
        "Default table view must show 'GP Practice +1', got:\n{}",
        s_default
    );

    // 2. Verbose: full role set in stored order
    let mut out_verbose = Vec::new();
    find::run_with_writer(
        Args {
            query: Some("SEDBERGH MEDICAL PRACTICE".to_string()),
            code: Vec::new(),
            location: None,
            role: Vec::new(),
            all: false,
            verbose: true,
            sort: None,
            format: OutputFormat::Table,
            input: parquet_dir.clone(),
        },
        &mut out_verbose,
        &parquet_dir,
    ).unwrap();
    let s_verbose = String::from_utf8(out_verbose).unwrap();
    assert!(
        s_verbose.contains("Prescribing Cost Centr..."),
        "Verbose view must show stored order with Prescribing Cost Centre first, got:\n{}",
        s_verbose
    );

    // 3. RJZ King's College Hospital Foundation Trust: NHS Trust, Hospice +1
    let mut out_rjz = Vec::new();
    find::run_with_writer(
        Args {
            query: None,
            code: vec!["RJZ".to_string()],
            location: None,
            role: Vec::new(),
            all: false,
            verbose: false,
            sort: None,
            format: OutputFormat::Table,
            input: parquet_dir.clone(),
        },
        &mut out_rjz,
        &parquet_dir,
    ).unwrap();
    let s_rjz = String::from_utf8(out_rjz).unwrap();
    assert!(
        s_rjz.contains("NHS Trust, Hospice +1"),
        "RJZ table view must show 'NHS Trust, Hospice +1', got:\n{}",
        s_rjz
    );
}


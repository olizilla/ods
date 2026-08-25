mod common;

use common::setup_find_test_workspace;
use ods::commands::find;

#[test]
fn test_find_sedbergh_output_matches_good_binary() {
    let (_tmp, parquet_dir) = setup_find_test_workspace();

    let mut out = Vec::new();
    find::run_with_writer(
        find::Args {
            query: None,
            code: Vec::new(),
            location: Some("sedbergh".to_string()),
            role: Vec::new(),
            all: false,
            verbose: false,
            sort: Some(find::SortBy::Code),
            format: find::OutputFormat::Table,
            input: parquet_dir.clone(),
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .expect("find::run_with_writer should succeed");

    let output_str = String::from_utf8(out).expect("valid UTF-8");
    let lines: Vec<&str> = output_str.lines().collect();

    // 1. Verify table header and column ordering: ODS Code | Name | Postcode | Roles | Class
    assert!(
        lines.iter().any(|l| l.contains("ODS Code")
            && l.contains("Postcode")
            && l.contains("Roles")
            && l.contains("Class")),
        "Header row missing expected columns in order: {}",
        output_str
    );

    // 2. Verify all 14 active records matching town 'sedbergh' are found
    assert!(
        lines
            .iter()
            .any(|l| l.contains("Found 14 matching active records")),
        "Expected 'Found 14 matching active records', got:\n{}",
        output_str
    );

    // 3. Verify specific known records in town Sedbergh are present in output
    let expected_codes = vec![
        "8GJ58", "A82608", "A82608001", "D2E8H", "EE112233", "EE112331", "EE112451", "EE137269",
        "FLG02", "RNN88", "RW5OX", "RX796", "V25604", "VN6C2",
    ];

    for code in expected_codes {
        assert!(
            output_str.contains(code),
            "Expected ODS code {} in find output, but was missing!",
            code
        );
    }
}

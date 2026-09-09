//! Integration tests for `find-table-footer.md`
//!
//! Verifies:
//! - Task 1: Box derivation and display width consistency with unicode_width.
//! - Task 2: Two `*` lines above table (`Search:` and `Source:`).
//! - Task 3: The disagreement notice becomes a `!` line between `* Source:` and the table.
//! - Task 4: Footer content (`{n} active records` vs `{n} records` / `{a} active · {i} inactive`),
//!           parsed data row count verification, singularisation, and zero-result search.
//! - Task 5: Narrow terminal rendering (`COLUMNS=40`) with right-hint overflow to `*` line.
//! - Task 6: Staleness nudge with `!` sigil as final output.

mod common;

use common::setup_find_test_workspace;
use ods::commands::find::render_with_footer;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

fn ods_binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ods"))
}

/// Helper creating a workspace with two releases:
/// - active: 2026-07-31 (current)
/// - older: 2026-06-26
fn setup_two_release_workspace() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let ws_root = tmp.path().join("ods_data");
    fs::create_dir_all(&ws_root).unwrap();

    let (_find_tmp, sample_parquet_dir) = setup_find_test_workspace();

    // 1. Create release 2026-07-31
    let rel_active = ws_root.join("releases").join("2026-07-31");
    fs::create_dir_all(&rel_active).unwrap();
    for entry in fs::read_dir(&sample_parquet_dir).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            fs::copy(entry.path(), rel_active.join(entry.file_name())).unwrap();
        }
    }
    let mut active_prov = ods::provenance::OdsProvenance::load_from_dir(&rel_active).unwrap_or_default();
    active_prov.trud_release_name = Some("Release 7.0.0".to_string());
    active_prov.trud_release_date = Some("2026-07-31".to_string());
    active_prov.trud_release_file = Some("hscorgrefdataxml_data_7.0.0_20260731000001.zip".to_string());
    active_prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());
    active_prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);
    active_prov.publication_date = Some("2026-07-28".to_string());
    active_prov.publication_seq_num = Some("4700".to_string());
    active_prov.publication_type = Some("Full".to_string());
    active_prov.publication_record_count = Some(305541);
    active_prov.dataset_version = Some("1.0.1".to_string());
    fs::write(
        rel_active.join(ods::provenance::PROVENANCE_FILENAME),
        serde_json::to_string_pretty(&active_prov).unwrap(),
    )
    .unwrap();

    // 2. Create release 2026-06-26
    let rel_older = ws_root.join("releases").join("2026-06-26");
    fs::create_dir_all(&rel_older).unwrap();
    for entry in fs::read_dir(&sample_parquet_dir).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            fs::copy(entry.path(), rel_older.join(entry.file_name())).unwrap();
        }
    }
    let mut older_prov = ods::provenance::OdsProvenance::load_from_dir(&rel_older).unwrap_or_default();
    older_prov.trud_release_name = Some("Release 6.9.0".to_string());
    older_prov.trud_release_date = Some("2026-06-26".to_string());
    older_prov.trud_release_file = Some("hscorgrefdataxml_data_6.9.0_20260626000001.zip".to_string());
    older_prov.trud_release_sha256 = Some("7151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());
    older_prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);
    older_prov.publication_date = Some("2026-06-24".to_string());
    older_prov.publication_seq_num = Some("4650".to_string());
    older_prov.publication_type = Some("Full".to_string());
    older_prov.publication_record_count = Some(305000);
    older_prov.dataset_version = Some("1.0.0".to_string());
    fs::write(
        rel_older.join(ods::provenance::PROVENANCE_FILENAME),
        serde_json::to_string_pretty(&older_prov).unwrap(),
    )
    .unwrap();

    // 3. Setup workspace marker and pin current -> 2026-07-31
    let ws = ods::workspace::Workspace::open_or_create(Some(&ws_root)).unwrap();
    ws.set_active("2026-07-31").unwrap();

    (tmp, ws_root)
}

// ---------------------------------------------------------------------------
// Task 1: Box Derivation & Display Width
// ---------------------------------------------------------------------------

#[test]
fn test_task1_render_with_footer_box_geometry() {
    use comfy_table::{presets, ContentArrangement, Table};
    use unicode_width::UnicodeWidthStr;

    let mut table = Table::new();
    table.load_style(presets::UTF8_FULL_CONDENSED);
    table.set_content_arrangement(ContentArrangement::Disabled);
    table.set_header(vec!["ODS Code", "Organisation Name", "Postcode"]);
    table.add_row(vec!["A01", "René & François Hospital", "SW1A 1AA"]);
    table.add_row(vec!["B02", "St Tomás Medical Centre", "LA10 5DL"]);

    let orig_render = table.to_string();
    let orig_bottom = orig_render.lines().last().unwrap();
    let orig_dividers: Vec<usize> = orig_bottom
        .char_indices()
        .filter_map(|(i, c)| if c == '┴' { Some(i) } else { None })
        .collect();

    let left = "2 active records";
    let right = "Use --all to include inactive";
    let rendered = render_with_footer(&table, left, right);

    let lines: Vec<&str> = rendered.table.lines().collect();
    let expected_width = lines[0].width();

    // Every line in result must have identical display width
    for (i, line) in lines.iter().enumerate() {
        assert_eq!(
            line.width(),
            expected_width,
            "Line {} display width mismatch: '{}'",
            i,
            line
        );
    }

    // Merge line is 3rd from bottom
    let merge_line = lines[lines.len() - 3];
    assert!(merge_line.starts_with('├'));
    assert!(merge_line.ends_with('┤'));
    let merge_dividers: Vec<usize> = merge_line
        .char_indices()
        .filter_map(|(i, c)| if c == '┴' { Some(i) } else { None })
        .collect();
    assert_eq!(
        orig_dividers, merge_dividers,
        "Merge line dividers must be in identical positions to original bottom border"
    );

    // Bottom line has all ┴ replaced with ─
    let bottom_line = lines.last().unwrap();
    assert!(bottom_line.starts_with('└'));
    assert!(bottom_line.ends_with('┘'));
    assert!(!bottom_line.contains('┴'), "bottom line must have no ┴ characters");
}

// ---------------------------------------------------------------------------
// Task 2: Two `*` lines above the table
// ---------------------------------------------------------------------------

#[test]
fn test_task2_header_search_and_source_lines() {
    let (_tmp, ws_root) = setup_two_release_workspace();

    // 1. ods find sedbergh
    let out_plain = ods_binary()
        .current_dir(&ws_root)
        .args(["find", "sedbergh"])
        .output()
        .expect("run find");
    let stdout_plain = String::from_utf8_lossy(&out_plain.stdout);
    assert!(!stdout_plain.contains("* Search:"), "* Search: line must be deleted");
    assert!(stdout_plain.contains("* Source: releases/2026-07-31/orgs.parquet"));
    assert!(!stdout_plain.contains("* matched"), "trailing matched line must be deleted");

    // 2. ods find --in sedbergh
    let out_in_town = ods_binary()
        .current_dir(&ws_root)
        .args(["find", "--in", "sedbergh"])
        .output()
        .expect("run find --in");
    let stdout_in_town = String::from_utf8_lossy(&out_in_town.stdout);
    assert!(!stdout_in_town.contains("* Search:"), "* Search: line must be deleted");
    assert!(!stdout_in_town.contains("* matched"), "trailing matched line must be deleted");

    // 3. ods find --in cumbria
    let out_in_county = ods_binary()
        .current_dir(&ws_root)
        .args(["find", "--in", "cumbria"])
        .output()
        .expect("run find --in cumbria");
    let stdout_in_county = String::from_utf8_lossy(&out_in_county.stdout);
    assert!(!stdout_in_county.contains("* Search:"), "* Search: line must be deleted");

    // 4. ods find sedbergh --all (names orgs_all.parquet in Source)
    let out_all = ods_binary()
        .current_dir(&ws_root)
        .args(["find", "sedbergh", "--all"])
        .output()
        .expect("run find --all");
    let stdout_all = String::from_utf8_lossy(&out_all.stdout);
    assert!(stdout_all.contains("* Source: releases/2026-07-31/orgs_all.parquet"));

    // 5. ods find sedbergh --format json (prints neither header line)
    let out_json = ods_binary()
        .current_dir(&ws_root)
        .args(["find", "sedbergh", "--format", "json"])
        .output()
        .expect("run find --format json");
    let stdout_json = String::from_utf8_lossy(&out_json.stdout);
    assert!(!stdout_json.contains("* Search:"));
    assert!(!stdout_json.contains("* Source:"));
}

// ---------------------------------------------------------------------------
// Task 3: The disagreement notice becomes a `!` line
// ---------------------------------------------------------------------------

#[test]
fn test_task3_disagreement_becomes_exclamation_line() {
    let (_tmp, ws_root) = setup_two_release_workspace();
    let older_rel_dir = ws_root.join("releases").join("2026-06-26");

    // Run find from inside older release:
    // ! line must appear directly under * Source: and above the table
    let out_disagree = ods_binary()
        .current_dir(&older_rel_dir)
        .args(["find", "sedbergh"])
        .output()
        .expect("run find from older release");

    assert!(out_disagree.status.success());
    let stdout = String::from_utf8_lossy(&out_disagree.stdout);

    let source_pos = stdout.find("* Source: releases/2026-07-31/orgs.parquet").expect("source line present");
    let disagree_pos = stdout.find("! Run from releases/2026-06-26. Change source with: ods use 2026-06-26").expect("! line present");
    let table_pos = stdout.find("┌").expect("table top border present");

    assert!(
        source_pos < disagree_pos,
        "! line must appear directly under * Source: line"
    );
    assert!(
        disagree_pos < table_pos,
        "! line must appear before the table top border"
    );

    // Run from workspace root: no ! line
    let out_root = ods_binary()
        .current_dir(&ws_root)
        .args(["find", "sedbergh"])
        .output()
        .expect("run find from root");
    let stdout_root = String::from_utf8_lossy(&out_root.stdout);
    assert!(!stdout_root.contains("! Run from releases/"));

    // Verify cite keeps existing two-line stderr form
    let cite_out = ods_binary()
        .current_dir(&ws_root)
        .arg("cite")
        .output()
        .expect("run cite");
    let cite_stderr = String::from_utf8_lossy(&cite_out.stderr);
    assert!(cite_stderr.contains("  2026-07-31 (current)"));

    let role_out = ods_binary()
        .current_dir(&ws_root)
        .arg("role")
        .output()
        .expect("run role");
    let role_stdout = String::from_utf8_lossy(&role_out.stdout);
    assert!(role_stdout.contains("* Source: releases/2026-07-31/orgs.parquet"));
    let role_stderr = String::from_utf8_lossy(&role_out.stderr);
    assert!(!role_stderr.contains("  2026-07-31 (current)"));

    let info_out = ods_binary()
        .current_dir(&ws_root)
        .args(["info", "A82608"])
        .output()
        .expect("run info");
    let info_stdout = String::from_utf8_lossy(&info_out.stdout);
    assert!(info_stdout.contains("* Source: releases/2026-07-31/orgs.parquet"));
    let info_stderr = String::from_utf8_lossy(&info_out.stderr);
    assert!(!info_stderr.contains("  2026-07-31 (current)"));
}

// ---------------------------------------------------------------------------
// Task 4: Footer Content & Verification
// ---------------------------------------------------------------------------

#[test]
fn test_task4_footer_content_and_row_count_matching() {
    let (_tmp, ws_root) = setup_two_release_workspace();

    // 1. ods find sedbergh
    let out_plain = ods_binary()
        .current_dir(&ws_root)
        .args(["find", "sedbergh"])
        .output()
        .expect("run find sedbergh");
    let stdout_plain = String::from_utf8_lossy(&out_plain.stdout);

    // Parse data rows between header separator and footer merge line
    let lines: Vec<&str> = stdout_plain.lines().collect();
    let header_sep_idx = lines.iter().position(|l| l.starts_with('╞')).expect("header separator");
    let footer_merge_idx = lines.iter().position(|l| l.starts_with('├')).expect("footer merge line");

    // Data rows are between header_sep_idx and footer_merge_idx
    let data_rows_count = footer_merge_idx - header_sep_idx - 1;

    // Footer line is immediately below footer_merge_idx
    let footer_line = lines[footer_merge_idx + 1];
    assert!(footer_line.contains("active records"));
    assert!(footer_line.contains("Use --all to include inactive"));

    // Extract the integer from footer line
    let count_word = footer_line
        .split_whitespace()
        .find(|w| w.chars().all(|c| c.is_ascii_digit()))
        .expect("integer count in footer");
    let parsed_count: usize = count_word.parse().unwrap();
    assert_eq!(
        parsed_count, data_rows_count,
        "Footer count {} must equal parsed data rows count {}",
        parsed_count, data_rows_count
    );

    // 2. ods find sedbergh --all
    let out_all = ods_binary()
        .current_dir(&ws_root)
        .args(["find", "sedbergh", "--all"])
        .output()
        .expect("run find sedbergh --all");
    let stdout_all = String::from_utf8_lossy(&out_all.stdout);
    let all_lines: Vec<&str> = stdout_all.lines().collect();
    let all_header_sep = all_lines.iter().position(|l| l.starts_with('╞')).unwrap();
    let all_footer_merge = all_lines.iter().position(|l| l.starts_with('├')).unwrap();
    let all_data_rows = all_footer_merge - all_header_sep - 1;

    let all_footer = all_lines[all_footer_merge + 1];
    assert!(all_footer.contains("records"));
    assert!(all_footer.contains("active ·"));
    assert!(all_footer.contains("inactive"));

    // Verify a + i == n
    // e.g. "│ 8 records ... 6 active · 2 inactive │"
    let parts: Vec<&str> = all_footer.split_whitespace().collect();
    let n_records: usize = parts[1].parse().unwrap();
    assert_eq!(n_records, all_data_rows);

    let active_pos = parts.iter().position(|&p| p == "active").unwrap();
    let a: usize = parts[active_pos - 1].parse().unwrap();

    let inactive_pos = parts.iter().position(|&p| p == "inactive").unwrap();
    let i: usize = parts[inactive_pos - 1].parse().unwrap();

    assert_eq!(a + i, n_records, "a ({}) + i ({}) must equal n ({})", a, i, n_records);

    // 3. Singularisation: single-result query reads "1 active record"
    let out_single = ods_binary()
        .current_dir(&ws_root)
        .args(["find", "--code", "A82608"])
        .output()
        .expect("run find single");
    let stdout_single = String::from_utf8_lossy(&out_single.stdout);
    assert!(
        stdout_single.contains("1 active record"),
        "single result must singularise to '1 active record', got:\n{}",
        stdout_single
    );

    // 4. Zero-result search: plain query with no match prints header and 0 active records
    let out_zero = ods_binary()
        .current_dir(&ws_root)
        .args(["find", "nonexistentquery12345"])
        .output()
        .expect("run find zero");
    let stdout_zero = String::from_utf8_lossy(&out_zero.stdout);
    assert!(out_zero.status.success());
    assert!(stdout_zero.contains("0 active records"));
    assert!(stdout_zero.contains("Use --all to include inactive"));
}

// ---------------------------------------------------------------------------
// Task 5: Narrow terminals
// ---------------------------------------------------------------------------

#[test]
fn test_task5_narrow_terminal_columns_40() {
    let (_tmp, ws_root) = setup_two_release_workspace();

    let output = ods_binary()
        .current_dir(&ws_root)
        .env("COLUMNS", "40")
        .args(["find", "sedbergh"])
        .output()
        .expect("run ods find with COLUMNS=40");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Locate the table lines
    let mut in_table = false;
    let mut table_lines = Vec::new();
    let mut line_below_table = None;

    for line in stdout.lines() {
        if line.starts_with('┌') {
            in_table = true;
        }
        if in_table {
            table_lines.push(line);
            if line.starts_with('└') {
                in_table = false;
            }
        } else if !table_lines.is_empty() && line_below_table.is_none() && !line.is_empty() {
            line_below_table = Some(line);
        }
    }

    assert!(!table_lines.is_empty(), "Table lines must be present");
    use unicode_width::UnicodeWidthStr;
    let table_width = table_lines[0].width();

    // Every line in the box must have identical width
    for (idx, line) in table_lines.iter().enumerate() {
        assert_eq!(
            line.width(),
            table_width,
            "Line {} in table has width {} instead of {}:\n{}",
            idx,
            line.width(),
            table_width,
            line
        );
    }

    // Right-hand hint must be emitted below the box as a `*` line
    let below = line_below_table.expect("Line below table should exist");
    assert!(
        below.contains("* Use --all to include inactive"),
        "Right-hand hint must overflow to '* Use --all to include inactive' below the box, got:\n{}",
        below
    );
}

// ---------------------------------------------------------------------------
// Task 6: The update nudge stays last
// ---------------------------------------------------------------------------

#[test]
fn test_task6_staleness_nudge_uses_exclamation_sigil() {
    let tmp = TempDir::new().unwrap();
    let ws_root = tmp.path().join("ods_data");
    fs::create_dir_all(&ws_root).unwrap();

    let (_find_tmp, sample_parquet_dir) = setup_find_test_workspace();

    // Create a release with an old date (e.g. 60 days old)
    let old_date = (chrono::Utc::now().date_naive() - chrono::Duration::days(60))
        .format("%Y-%m-%d")
        .to_string();

    let rel_old = ws_root.join("releases").join(&old_date);
    fs::create_dir_all(&rel_old).unwrap();
    for entry in fs::read_dir(&sample_parquet_dir).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            fs::copy(entry.path(), rel_old.join(entry.file_name())).unwrap();
        }
    }
    let mut prov = ods::provenance::OdsProvenance::load_from_dir(&rel_old).unwrap_or_default();
    prov.trud_release_date = Some(old_date.clone());
    fs::write(
        rel_old.join(ods::provenance::PROVENANCE_FILENAME),
        serde_json::to_string_pretty(&prov).unwrap(),
    )
    .unwrap();

    let ws = ods::workspace::Workspace::open_or_create(Some(&ws_root)).unwrap();
    ws.set_active(&old_date).unwrap();

    // Create a release index with only this old release
    let mut index = ods::index::OdsReleaseIndex::default();
    index.type_tag = "ods_release_index".to_string();
    index.index_version = 2;
    index.releases.push(ods::index::ReleaseIndexEntry {
        trud_release_date: old_date.clone(),
        dataset_version: "1.0.0".to_string(),
        tag: format!("2026-06-01-v1.0.0"),
        manifest_digest: "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
        trud_release_sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
        tool_version: "0.1.0".to_string(),
        dataset_doi: None,
        withdrawn: None,
    });
    fs::write(
        ws_root.join(ods::index::RELEASES_JSON_FILENAME),
        serde_json::to_string_pretty(&index).unwrap(),
    )
    .unwrap();

    let output = ods_binary()
        .current_dir(&ws_root)
        .args(["find", "sedbergh"])
        .output()
        .expect("run find with stale index");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "ods find failed with code {:?}:\nSTDOUT:\n{}\nSTDERR:\n{}",
        output.status.code(),
        stdout,
        stderr
    );

    // Staleness nudge must appear with `!` sigil
    assert!(
        stderr.contains(&format!("! {} is 60 days old. TRUD ships roughly every 4 weeks", old_date)),
        "nudge must use ! sigil, got:\n{}",
        stderr
    );
    assert!(
        stderr.contains("  Check with: ods pull"),
        "nudge must offer 'Check with: ods pull', got:\n{}",
        stderr
    );
}

#[test]
fn test_render_with_footer_with_rounded_corners() {
    use comfy_table::{presets, ContentArrangement, Table};

    let mut table = Table::new();
    table.load_style(presets::UTF8_FULL_CONDENSED.with_rounded_corners());
    table.set_content_arrangement(ContentArrangement::Disabled);
    table.set_header(vec!["ODS Code", "Organisation Name"]);
    table.add_row(vec!["A01", "René & François Hospital"]);

    let rendered = render_with_footer(&table, "1 active record", "Use --all to include inactive");
    let lines: Vec<&str> = rendered.table.lines().collect();
    let merge_line = lines[lines.len() - 3];
    assert!(
        merge_line.starts_with('├'),
        "Merge line must start with ├, but was: {}",
        merge_line
    );
    assert!(
        merge_line.ends_with('┤'),
        "Merge line must end with ┤, but was: {}",
        merge_line
    );
}


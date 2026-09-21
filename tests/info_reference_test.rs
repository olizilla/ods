use ods::commands::info::model::InfoRecord;
use ods::commands::info::render::{render_info, RenderOptions, ResponsiveBand, TableStyle};
use std::fs;

fn source_header_for(record: &InfoRecord, color: bool) -> Vec<String> {
    let file = "orgs.parquet";
    let release_date = if record.trud_release_date.is_empty() {
        "current"
    } else {
        &record.trud_release_date
    };
    let path = format!("releases/{}/{}", release_date, file);
    vec![ods::workspace::format_source_line(&path, color)]
}

fn test_case(
    fixture_name: &str,
    ref_filename: &str,
    band: ResponsiveBand,
    all: bool,
) {
    let fixture_path = format!("tests/fixtures/info/fixtures/{}.json", fixture_name);
    let ref_path = format!("tests/fixtures/info/reference/{}", ref_filename);

    let fixture_content = fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", fixture_path, e));
    let record: InfoRecord = serde_json::from_str(&fixture_content)
        .unwrap_or_else(|e| panic!("failed to deserialize {}: {}", fixture_path, e));

    let source_header = source_header_for(&record, false);
    let options = RenderOptions {
        band,
        all,
        color: false,
        source_header: &source_header,
        style: TableStyle::Table,
    };

    let rendered = render_info(&record, &options);

    let cmd = if all {
        format!("❯ ods info {} --all\n", fixture_name)
    } else {
        format!("❯ ods info {}\n", fixture_name)
    };
    let full_content = format!("{}{}", cmd, rendered);

    if std::env::var("UPDATE_EXPECT").is_ok() {
        fs::write(&ref_path, &full_content)
            .unwrap_or_else(|e| panic!("failed to write {}: {}", ref_path, e));
    }

    let ref_content = fs::read_to_string(&ref_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", ref_path, e));

    // Strip line 1 (`❯ ods info ...\n`)
    let expected_body = if let Some(pos) = ref_content.find('\n') {
        &ref_content[pos + 1..]
    } else {
        &ref_content
    };

    // 1. Invariant: every line starting with box chars must have length equal to band.width()
    let expected_width = band.width();
    for (line_idx, line) in rendered.lines().enumerate() {
        if line.starts_with('┌')
            || line.starts_with('│')
            || line.starts_with('╞')
            || line.starts_with('└')
        {
            let char_count = line.chars().count();
            assert_eq!(
                char_count,
                expected_width as usize,
                "Line {} in {} does not match band width {}:\n{}",
                line_idx + 1,
                ref_filename,
                expected_width,
                line
            );
        }
    }

    // 2. Invariant: Golden comparison against reference output
    assert_eq!(
        rendered.trim_end(),
        expected_body.trim_end(),
        "Mismatch in reference file {}",
        ref_filename
    );
}

#[test]
fn test_all_13_reference_files() {
    let cases = [
        ("02V", "02V.wide.txt", ResponsiveBand::Wide, false),
        ("15N", "15N.narrow.txt", ResponsiveBand::Narrow, false),
        ("15N", "15N.medium.txt", ResponsiveBand::Medium, false),
        ("15N", "15N.wide.txt", ResponsiveBand::Wide, false),
        ("5QG", "5QG.wide.txt", ResponsiveBand::Wide, false),
        ("A81002", "A81002.medium.txt", ResponsiveBand::Medium, false),
        ("A81002", "A81002.wide.txt", ResponsiveBand::Wide, false),
        ("K81657", "K81657.narrow.txt", ResponsiveBand::Narrow, false),
        ("K81657", "K81657.narrow.all.txt", ResponsiveBand::Narrow, true),
        ("K81657", "K81657.medium.txt", ResponsiveBand::Medium, false),
        ("K81657", "K81657.medium.all.txt", ResponsiveBand::Medium, true),
        ("K81657", "K81657.wide.txt", ResponsiveBand::Wide, false),
        ("K81657", "K81657.wide.all.txt", ResponsiveBand::Wide, true),
    ];

    for (fixture, ref_file, band, all) in cases {
        test_case(fixture, ref_file, band, all);
    }
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn test_coloured_box_width(
    fixture_name: &str,
    ref_filename: &str,
    band: ResponsiveBand,
    all: bool,
) {
    let fixture_path = format!("tests/fixtures/info/fixtures/{}.json", fixture_name);
    let fixture_content = fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", fixture_path, e));
    let record: InfoRecord = serde_json::from_str(&fixture_content)
        .unwrap_or_else(|e| panic!("failed to deserialize {}: {}", fixture_path, e));

    let source_header = source_header_for(&record, true);
    let options = RenderOptions {
        band,
        all,
        color: true,
        source_header: &source_header,
        style: TableStyle::Table,
    };

    let rendered = render_info(&record, &options);

    // Verify ANSI color escape codes are indeed present
    assert!(
        rendered.contains("\x1b["),
        "Rendered output for {} with color: true must contain ANSI escape codes",
        fixture_name
    );

    let ref_path = format!("tests/fixtures/info/reference/{}", ref_filename);
    let ref_content = fs::read_to_string(&ref_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", ref_path, e));

    // Strip line 1 (`❯ ods info ...\n`)
    let expected_body = if let Some(pos) = ref_content.find('\n') {
        &ref_content[pos + 1..]
    } else {
        &ref_content
    };

    // 1. Colour must add escapes and change nothing else: stripped render == reference body
    let stripped_full = strip_ansi(&rendered);
    assert_eq!(
        stripped_full.trim_end(),
        expected_body.trim_end(),
        "Stripped ANSI output for {} does not match reference body {}",
        fixture_name,
        ref_filename
    );

    // 2. Invariant: every line starting with box chars must have length equal to band.width()
    let expected_width = band.width();
    for (line_idx, line) in rendered.lines().enumerate() {
        let stripped = strip_ansi(line);
        if stripped.starts_with('┌')
            || stripped.starts_with('│')
            || stripped.starts_with('╞')
            || stripped.starts_with('└')
        {
            let char_count = stripped.chars().count();
            assert_eq!(
                char_count,
                expected_width as usize,
                "Coloured line {} in {} (band {:?}) does not match band width {}:\nRaw: {}\nStripped: {}",
                line_idx + 1,
                fixture_name,
                band,
                expected_width,
                line,
                stripped
            );
        }
    }

    // 3. Assert the dimming actually happened: the 5QG Wide render contains 93 dimmed box-drawing runs
    if fixture_name == "5QG" && band == ResponsiveBand::Wide && !all {
        let dimmed_box_runs = rendered
            .match_indices(ods::ansi::ANSI_MUTED)
            .filter(|&(idx, _)| {
                rendered[idx + ods::ansi::ANSI_MUTED.len()..]
                    .chars()
                    .next()
                    .is_some_and(ods::ansi::is_box_drawing)
            })
            .count();
        println!("5QG Wide: {} dimmed box-drawing runs", dimmed_box_runs);
        assert_eq!(
            dimmed_box_runs, 93,
            "5QG Wide must contain 93 dimmed box-drawing runs, found {}",
            dimmed_box_runs
        );
    }
}

#[test]
fn test_all_13_reference_files_coloured_box_width_invariant() {
    let cases = [
        ("02V", "02V.wide.txt", ResponsiveBand::Wide, false),
        ("15N", "15N.narrow.txt", ResponsiveBand::Narrow, false),
        ("15N", "15N.medium.txt", ResponsiveBand::Medium, false),
        ("15N", "15N.wide.txt", ResponsiveBand::Wide, false),
        ("5QG", "5QG.wide.txt", ResponsiveBand::Wide, false),
        ("A81002", "A81002.medium.txt", ResponsiveBand::Medium, false),
        ("A81002", "A81002.wide.txt", ResponsiveBand::Wide, false),
        ("K81657", "K81657.narrow.txt", ResponsiveBand::Narrow, false),
        ("K81657", "K81657.narrow.all.txt", ResponsiveBand::Narrow, true),
        ("K81657", "K81657.medium.txt", ResponsiveBand::Medium, false),
        ("K81657", "K81657.medium.all.txt", ResponsiveBand::Medium, true),
        ("K81657", "K81657.wide.txt", ResponsiveBand::Wide, false),
        ("K81657", "K81657.wide.all.txt", ResponsiveBand::Wide, true),
    ];

    for (fixture, ref_file, band, all) in cases {
        test_coloured_box_width(fixture, ref_file, band, all);
    }
}

fn test_markdown_case(fixture_name: &str, ref_filename: &str) {
    let fixture_path = format!("tests/fixtures/info/fixtures/{}.json", fixture_name);
    let ref_path = format!("tests/fixtures/info/reference/{}", ref_filename);

    let fixture_content = fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", fixture_path, e));
    let record: InfoRecord = serde_json::from_str(&fixture_content)
        .unwrap_or_else(|e| panic!("failed to deserialize {}: {}", fixture_path, e));

    let source_header = source_header_for(&record, false);
    let options_wide = RenderOptions {
        band: ResponsiveBand::Wide,
        all: false,
        color: false,
        source_header: &source_header,
        style: TableStyle::Markdown,
    };
    let rendered_wide = render_info(&record, &options_wide);

    let options_narrow = RenderOptions {
        band: ResponsiveBand::Narrow,
        all: false,
        color: false,
        source_header: &source_header,
        style: TableStyle::Markdown,
    };
    let rendered_narrow = render_info(&record, &options_narrow);

    if std::env::var("UPDATE_EXPECT").is_ok() {
        fs::write(&ref_path, &rendered_wide)
            .unwrap_or_else(|e| panic!("failed to write {}: {}", ref_path, e));
    }

    let expected = fs::read_to_string(&ref_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", ref_path, e));

    assert_eq!(
        rendered_wide, expected,
        "Markdown output for {} at Wide does not match golden reference fixture {}",
        fixture_name, ref_filename
    );

    assert_eq!(
        rendered_narrow, expected,
        "Markdown output for {} at Narrow does not match golden reference fixture {}",
        fixture_name, ref_filename
    );

    assert!(
        !rendered_wide.contains('…'),
        "Markdown output for {} at Wide contains ellipsis (…)",
        fixture_name
    );

    assert!(
        !rendered_narrow.contains('…'),
        "Markdown output for {} at Narrow contains ellipsis (…)",
        fixture_name
    );
}

#[test]
fn test_markdown_reference_fixtures() {
    test_markdown_case("5QG", "5QG.markdown.md");
    test_markdown_case("15N", "15N.markdown.md");
    test_markdown_case("A81002", "A81002.markdown.md");
}


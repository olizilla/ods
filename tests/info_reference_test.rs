use ods::commands::info::model::InfoRecord;
use ods::commands::info::render::{render_info, RenderOptions, ResponsiveBand};
use std::fs;

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

    let options = RenderOptions {
        band,
        all,
        color: false,
        source_path: None,
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

fn test_coloured_box_width(fixture_name: &str, band: ResponsiveBand, all: bool) {
    let fixture_path = format!("tests/fixtures/info/fixtures/{}.json", fixture_name);
    let fixture_content = fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", fixture_path, e));
    let record: InfoRecord = serde_json::from_str(&fixture_content)
        .unwrap_or_else(|e| panic!("failed to deserialize {}: {}", fixture_path, e));

    let options = RenderOptions {
        band,
        all,
        color: true,
        source_path: None,
    };

    let rendered = render_info(&record, &options);

    // Verify ANSI color escape codes are indeed present
    assert!(
        rendered.contains("\x1b["),
        "Rendered output for {} with color: true must contain ANSI escape codes",
        fixture_name
    );

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
}

#[test]
fn test_all_13_reference_files_coloured_box_width_invariant() {
    let cases = [
        ("02V", ResponsiveBand::Wide, false),
        ("15N", ResponsiveBand::Narrow, false),
        ("15N", ResponsiveBand::Medium, false),
        ("15N", ResponsiveBand::Wide, false),
        ("5QG", ResponsiveBand::Wide, false),
        ("A81002", ResponsiveBand::Medium, false),
        ("A81002", ResponsiveBand::Wide, false),
        ("K81657", ResponsiveBand::Narrow, false),
        ("K81657", ResponsiveBand::Narrow, true),
        ("K81657", ResponsiveBand::Medium, false),
        ("K81657", ResponsiveBand::Medium, true),
        ("K81657", ResponsiveBand::Wide, false),
        ("K81657", ResponsiveBand::Wide, true),
    ];

    for (fixture, band, all) in cases {
        test_coloured_box_width(fixture, band, all);
    }
}

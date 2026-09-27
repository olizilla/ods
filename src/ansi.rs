use comfy_table::{ContentArrangement, Table};
use std::io::IsTerminal;

pub const ANSI_RESET: &str = "\x1b[0m";
pub const ANSI_BOLD: &str = "\x1b[1m";
pub const ANSI_RED: &str = "\x1b[31m";
pub const ANSI_GREEN: &str = "\x1b[32m";
pub const ANSI_YELLOW: &str = "\x1b[33m";
pub const ANSI_CYAN: &str = "\x1b[36m";
pub const ANSI_MUTED: &str = "\x1b[90m";
pub const ANSI_DEFAULT_FG: &str = "\x1b[39m";

/// Returns true if ANSI color output should be enabled for stdout.
///
/// Follows standard CLI conventions:
/// - Must be connected to a terminal (TTY)
/// - Must not have `--plain` set
/// - Must not have `NO_COLOR` set in environment
/// - Must not have `TERM=dumb`
pub fn stdout_color_enabled(plain: bool) -> bool {
    std::io::stdout().is_terminal()
        && !plain
        && std::env::var_os("NO_COLOR").is_none()
        && std::env::var("TERM").map(|t| t != "dumb").unwrap_or(true)
}

/// Returns true if character `c` is in the Unicode Box Drawing block (U+2500..=U+257F).
#[inline]
pub fn is_box_drawing(c: char) -> bool {
    ('\u{2500}'..='\u{257F}').contains(&c)
}

/// Wraps each maximal run of box-drawing characters (U+2500..=U+257F)
/// in `ANSI_MUTED` ... `ANSI_DEFAULT_FG`. Everything else passes through byte-for-byte.
pub fn dim_borders(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + s.len() / 4);
    let mut in_box = false;

    for c in s.chars() {
        if is_box_drawing(c) {
            if !in_box {
                out.push_str(ANSI_MUTED);
                in_box = true;
            }
            out.push(c);
        } else {
            if in_box {
                out.push_str(ANSI_DEFAULT_FG);
                in_box = false;
            }
            out.push(c);
        }
    }

    if in_box {
        out.push_str(ANSI_DEFAULT_FG);
    }

    out
}

/// Resolves the width for a table or a wrapped notice line written to stdout.
///
/// In order: `COLUMNS` if set to a positive number; the terminal's width when stdout
/// is a terminal and it reports a positive width; otherwise `None`, meaning don't
/// wrap at all — a table or notice written to a file or pipe must not depend on the
/// terminal it happened to run in. `COLUMNS` set to `0`, empty or non-numeric, and a
/// terminal reporting a width of `0`, all count as unset.
pub fn resolve_display_width() -> Option<u16> {
    if let Ok(cols_str) = std::env::var("COLUMNS") {
        if let Ok(cols) = cols_str.parse::<u16>() {
            if cols > 0 {
                return Some(cols);
            }
        }
    }
    if std::io::stdout().is_terminal() {
        if let Ok((width, _)) = crossterm::terminal::size() {
            if width > 0 {
                return Some(width);
            }
        }
    }
    None
}

/// Applies the table width policy shared by `find` and `role`: `Some(width)` sets
/// `Dynamic` arrangement at that width, so a row wraps to fit it; `None` sets
/// `Disabled`, so every row stays on one line regardless of its length, and the
/// table doesn't depend on the terminal it happened to run in.
pub fn apply_table_width_policy(table: &mut Table, explicit_width: Option<u16>) {
    if let Some(width) = explicit_width {
        table.set_content_arrangement(ContentArrangement::Dynamic);
        table.set_width(width);
    } else {
        table.set_content_arrangement(ContentArrangement::Disabled);
    }
}

/// Strips all ANSI escape sequences from `s`.
pub fn strip_ansi(s: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    #[test]
    fn test_dim_borders_wrapping() {
        let input = "┌──┐\n│ A │\n└──┘";
        let dimmed = dim_borders(input);
        assert_eq!(
            dimmed,
            format!(
                "{ANSI_MUTED}┌──┐{ANSI_DEFAULT_FG}\n{ANSI_MUTED}│{ANSI_DEFAULT_FG} A {ANSI_MUTED}│{ANSI_DEFAULT_FG}\n{ANSI_MUTED}└──┘{ANSI_DEFAULT_FG}"
            )
        );
        assert_eq!(strip_ansi(&dimmed), input);
    }

    #[test]
    fn test_dim_borders_no_box_chars() {
        let input = "Hello world! 12345";
        assert_eq!(dim_borders(input), input);
    }

    #[test]
    fn test_apply_table_width_policy_disabled_keeps_long_row_on_one_line() {
        let mut table = Table::new();
        table.load_style(comfy_table::presets::UTF8_FULL_CONDENSED);
        table.set_header(vec!["Code", "Name"]);
        let long_name = "Z".repeat(150);
        table.add_row(vec!["A1".to_string(), long_name.clone()]);

        apply_table_width_policy(&mut table, None);
        let rendered = table.to_string();
        let row_line = rendered
            .lines()
            .find(|l| l.contains(&long_name))
            .expect("the long value should appear whole on one line");
        assert!(
            unicode_width::UnicodeWidthStr::width(row_line) > 120,
            "unwrapped row should stay a single line over 120 columns wide: {row_line:?}"
        );
    }

    #[test]
    fn test_apply_table_width_policy_with_width_wraps_long_row() {
        let mut table = Table::new();
        table.load_style(comfy_table::presets::UTF8_FULL_CONDENSED);
        table.set_header(vec!["Code", "Name"]);
        let long_name = "Z".repeat(150);
        table.add_row(vec!["A1".to_string(), long_name.clone()]);

        apply_table_width_policy(&mut table, Some(60));
        let rendered = table.to_string();
        assert!(
            !rendered.contains(&long_name),
            "the value should wrap across lines once a width is given:\n{rendered}"
        );
        for line in rendered.lines() {
            assert!(
                unicode_width::UnicodeWidthStr::width(line) <= 60,
                "wrapped line should fit the given width: {line:?}"
            );
        }
    }

    #[test]
    fn test_all_13_reference_files_strip_dim_borders_byte_identical() {
        let ref_dir = Path::new("tests/fixtures/info/reference");
        let entries = fs::read_dir(ref_dir).expect("read reference dir");
        let mut txt_count = 0;

        for entry in entries {
            let entry = entry.expect("dir entry");
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("txt") {
                txt_count += 1;
                let original = fs::read_to_string(&path).expect("read txt file");
                let dimmed = dim_borders(&original);
                let stripped = strip_ansi(&dimmed);
                assert_eq!(
                    stripped,
                    original,
                    "Stripping ANSI from dimmed output of {} did not match original",
                    path.display()
                );
            }
        }

        assert_eq!(txt_count, 13, "Expected 13 reference .txt files");
    }
}

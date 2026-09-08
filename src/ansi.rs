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

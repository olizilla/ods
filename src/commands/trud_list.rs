use anyhow::Result;
use clap::Parser;
use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;

use crate::commands::fetch::{fetch_trud_releases, OutcomeSource, ReleaseListItemJson, TrudReleaseItem};
use crate::progress::format_size;

#[derive(Parser, Debug, Default, Clone)]
pub struct Args {
    /// List all available TRUD releases without truncating
    #[arg(long)]
    pub all: bool,

    /// Output format (json for machine-readable list)
    #[arg(long)]
    pub format: Option<String>,

    /// TRUD API Key
    #[arg(long, env = "TRUD_API_KEY")]
    pub api_key: Option<String>,

    /// Workspace directory (default: the one found from the current directory)
    #[arg(long, short = 'w')]
    pub workspace: Option<PathBuf>,

    /// Print verbose output (URL fetched and raw API response text before deserialization)
    #[arg(long, short = 'v')]
    pub verbose: bool,
}

pub fn run(args: Args) -> Result<()> {
    let color = crate::ansi::stdout_color_enabled(false);
    run_with_writer(args, &mut std::io::stdout(), color)
}

pub fn run_with_writer<W: Write>(args: Args, writer: &mut W, color: bool) -> Result<()> {
    let api_key = match args.api_key {
        Some(ref key) if !key.trim().is_empty() => key.clone(),
        _ => {
            anyhow::bail!(
                "✖ Missing API Key\n  Please set $TRUD_API_KEY environment variable or pass --api-key <KEY>.\n  See: https://isd.digital.nhs.uk/trud/user/authenticated/group/0/pack/341/subpack/160/releases"
            );
        }
    };

    let releases = fetch_trud_releases(&api_key, args.verbose)?;

    let workspace_root = match args.workspace {
        Some(ref p) => p.clone(),
        None => crate::workspace::Workspace::open(None)
            .map(|ws| ws.root().to_path_buf())
            .unwrap_or_else(|_| PathBuf::from("./ods_data")),
    };

    let mut held_dates = HashSet::new();
    for r in &releases {
        let archive_file = workspace_root
            .join("releases")
            .join(&r.release_date)
            .join("trud")
            .join(&r.archive_file_name);
        if archive_file.exists() {
            held_dates.insert(r.release_date.clone());
        }
    }

    if args.format.as_deref() == Some("json") {
        let json_items: Vec<ReleaseListItemJson> = releases
            .iter()
            .map(|r| {
                let is_pulled = held_dates.contains(&r.release_date);
                ReleaseListItemJson {
                    source: OutcomeSource::new(
                        Some(r.release_date.clone()),
                        Some(r.archive_file_sha256.clone()),
                        Some(r.archive_file_size),
                    ),
                    status: if is_pulled {
                        "pulled".to_string()
                    } else {
                        "remote".to_string()
                    },
                }
            })
            .collect();
        serde_json::to_writer_pretty(&mut *writer, &json_items)?;
        writeln!(writer)?;
        return Ok(());
    }

    let output = render_trud_releases(&releases, &held_dates, args.all, color);
    write!(writer, "{}", output)?;
    Ok(())
}

/// Computes the table content width required for the releases and footers.
fn compute_content_width(
    releases: &[TrudReleaseItem],
    held_dates: &HashSet<String>,
    all: bool,
) -> (usize, usize) {
    let total_count = releases.len();
    let total_bytes: u64 = releases.iter().map(|r| r.archive_file_size).sum();
    let total_size_str = format_size(total_bytes);

    let held_bytes: u64 = releases
        .iter()
        .filter(|r| held_dates.contains(&r.release_date))
        .map(|r| r.archive_file_size)
        .sum();
    let held_size_str = format_size(held_bytes);

    let count_part = format!("{} releases", total_count);
    let held_part = format!("{} pulled", held_size_str);

    let mut size_end = 19;
    if count_part.len() + 3 + total_size_str.len() > size_end {
        size_end = count_part.len() + 3 + total_size_str.len();
    }

    let mut content_width = 34;
    // Default footer: count on left, "--all to see more" right-aligned
    let min_default_footer = count_part.len() + 1 + 17;
    content_width = content_width.max(min_default_footer);

    if all {
        // --all footer: count_part, total_size_str ending at size_end, held_part right-aligned
        let min_all_footer = size_end + 3 + held_part.len();
        content_width = content_width.max(min_all_footer);
    }

    (content_width, size_end)
}

/// Formats the TRUD releases list and footer table.
pub fn render_trud_releases(
    releases: &[TrudReleaseItem],
    held_dates: &HashSet<String>,
    all: bool,
    color: bool,
) -> String {
    if releases.is_empty() {
        return String::new();
    }

    let total_count = releases.len();
    let total_bytes: u64 = releases.iter().map(|r| r.archive_file_size).sum();
    let total_size_str = format_size(total_bytes);

    let held_bytes: u64 = releases
        .iter()
        .filter(|r| held_dates.contains(&r.release_date))
        .map(|r| r.archive_file_size)
        .sum();
    let held_size_str = format_size(held_bytes);

    let (content_width, size_end) = compute_content_width(releases, held_dates, all);
    let gap1 = size_end - 5;
    let state_width = content_width.saturating_sub(size_end + 3);

    let border_dashes = content_width + 2;
    let top_border = format!("┌{}┐", "─".repeat(border_dashes));
    let header_divider = format!("╞{}╡", "═".repeat(border_dashes));
    let footer_divider = format!("├{}┤", "─".repeat(border_dashes));
    let bottom_border = format!("└{}┘", "─".repeat(border_dashes));

    let mut lines = Vec::new();

    if color {
        lines.push(crate::ansi::dim_borders(&top_border));
    } else {
        lines.push(top_border);
    }

    // Header: Release (0..7), Size ending at size_end, State starting at size_end + 3
    let header_release_part = format!("{:<width$}", "Release", width = size_end.saturating_sub(4));
    let header_content = format!(
        "{}Size   {:<state_width$}",
        header_release_part,
        "State",
        state_width = state_width
    );
    lines.push(format_box_line(&header_content, color, None));

    if color {
        lines.push(crate::ansi::dim_borders(&header_divider));
    } else {
        lines.push(header_divider);
    }

    enum DisplayRow<'a> {
        Release(&'a TrudReleaseItem),
        Ellipsis(usize),
    }

    let mut rows_to_display = Vec::new();
    if all || total_count <= 11 {
        for r in releases {
            rows_to_display.push(DisplayRow::Release(r));
        }
    } else {
        for r in &releases[..10] {
            rows_to_display.push(DisplayRow::Release(r));
        }
        let hidden = total_count - 11;
        rows_to_display.push(DisplayRow::Ellipsis(hidden));
        rows_to_display.push(DisplayRow::Release(releases.last().unwrap()));
    }

    for row in rows_to_display {
        match row {
            DisplayRow::Release(r) => {
                let sz = format_size(r.archive_file_size);
                let is_pulled = held_dates.contains(&r.release_date);
                let state_str = if is_pulled { "pulled" } else { "" };

                let left_str = format!("{:<gap1$}{:>5}", r.release_date, sz, gap1 = gap1);
                let row_content = format!(
                    "{}   {:<state_width$}",
                    left_str,
                    state_str,
                    state_width = state_width
                );

                let text_style = if is_pulled {
                    Some(crate::ansi::ANSI_MUTED)
                } else {
                    None
                };
                lines.push(format_box_line(&row_content, color, text_style));
            }
            DisplayRow::Ellipsis(hidden) => {
                let ellipsis_str = format!("...{} more", hidden);
                let row_content = format!("{:<content_width$}", ellipsis_str, content_width = content_width);
                lines.push(format_box_line(&row_content, color, Some(crate::ansi::ANSI_MUTED)));
            }
        }
    }

    if color {
        lines.push(crate::ansi::dim_borders(&footer_divider));
    } else {
        lines.push(footer_divider);
    }

    // Footer
    let count_str = format!("{} releases", total_count);
    if all {
        // count on left, total_size ending at size_end, held_size right-aligned
        let left_part = format!("{:<gap1$}{:>5}", count_str, total_size_str, gap1 = gap1);
        let right_part = format!("{} pulled", held_size_str);
        let spaces = content_width.saturating_sub(left_part.len() + right_part.len());
        let footer_content = format!("{}{}{}", left_part, " ".repeat(spaces), right_part);
        lines.push(format_box_line(&footer_content, color, None));
    } else {
        let right_part = "--all to see more";
        let spaces = content_width.saturating_sub(count_str.len() + right_part.len());
        let footer_content = format!("{}{}{}", count_str, " ".repeat(spaces), right_part);
        lines.push(format_box_line(&footer_content, color, None));
    }

    if color {
        lines.push(crate::ansi::dim_borders(&bottom_border));
    } else {
        lines.push(bottom_border);
    }

    // Hint line: newest release that isn't held
    if let Some(unheld) = releases.iter().find(|r| !held_dates.contains(&r.release_date)) {
        let hint = format!("* To pull: ods trud pull {}", unheld.release_date);
        if color {
            lines.push(format!("{}{}{}", crate::ansi::ANSI_MUTED, hint, crate::ansi::ANSI_RESET));
        } else {
            lines.push(hint);
        }
    }

    let mut out = lines.join("\n");
    out.push('\n');
    out
}

fn format_box_line(content: &str, color: bool, style: Option<&str>) -> String {
    let styled_content = match style {
        Some(s) if color => format!("{}{}{}", s, content, crate::ansi::ANSI_RESET),
        _ => content.to_string(),
    };
    let line = format!("│ {} │", styled_content);
    if color {
        crate::ansi::dim_borders(&line)
    } else {
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn mock_release(date: &str, size_bytes: u64) -> TrudReleaseItem {
        TrudReleaseItem {
            id: format!("test_{}.zip", date),
            name: Some("Test".to_string()),
            release_date: date.to_string(),
            archive_file_name: format!("archive_{}.zip", date),
            archive_file_sha256: "dummy_sha".to_string(),
            archive_file_size: size_bytes,
            download_url: "https://example.com/file.zip".to_string(),
            checksum_file_url: None,
            checksum_file_name: None,
            signature_file_url: None,
            signature_file_name: None,
            public_key_file_url: None,
            public_key_file_name: None,
        }
    }

    #[test]
    fn test_held_and_unheld_rows() {
        let releases = vec![
            mock_release("2026-07-31", 37_983_173),
            mock_release("2026-06-26", 37_957_865),
        ];
        let mut held = HashSet::new();
        held.insert("2026-07-31".to_string());

        let out = render_trud_releases(&releases, &held, false, false);
        assert!(out.contains("│ 2026-07-31     36MB   pulled       │"));
        assert!(out.contains("│ 2026-06-26     36MB                │"));
        assert!(out.contains("* To pull: ods trud pull 2026-06-26"));
    }

    #[test]
    fn test_ellipsis_count_and_both_footers() {
        let mut releases = Vec::new();
        for i in (0..96).rev() {
            let year = 2018 + (i / 12);
            let month = (i % 12) + 1;
            let date = format!("{:04}-{:02}-15", year, month);
            releases.push(mock_release(&date, 35_000_000));
        }
        let mut held = HashSet::new();
        held.insert(releases[0].release_date.clone());

        // Default: ellipsis count should be 96 - 11 = 85
        let out_default = render_trud_releases(&releases, &held, false, false);
        assert!(out_default.contains("│ ...85 more                         │"));
        assert!(out_default.contains("│ 96 releases      --all to see more │"));

        // --all: no ellipsis, full list, footer with archive total and held size
        let out_all = render_trud_releases(&releases, &held, true, false);
        assert!(!out_all.contains("..."));
        assert!(out_all.contains("33MB pulled"));
    }

    #[test]
    fn test_hint_date_choice_and_all_held() {
        let releases = vec![
            mock_release("2026-07-31", 37_000_000),
            mock_release("2026-06-26", 37_000_000),
            mock_release("2026-05-29", 37_000_000),
        ];
        let mut held = HashSet::new();
        held.insert("2026-07-31".to_string());

        let out = render_trud_releases(&releases, &held, false, false);
        assert!(out.contains("* To pull: ods trud pull 2026-06-26"));

        // When all releases are held, hint line is omitted
        held.insert("2026-06-26".to_string());
        held.insert("2026-05-29".to_string());
        let out_all_held = render_trud_releases(&releases, &held, false, false);
        assert!(!out_all_held.contains("* To pull:"));
    }

    #[test]
    fn test_json_items_carry_sha256() {
        let releases = [mock_release("2026-07-31", 37_983_173)];
        let held_dates: HashSet<String> = HashSet::new();
        let json_items: Vec<ReleaseListItemJson> = releases
            .iter()
            .map(|r| {
                let is_pulled = held_dates.contains(&r.release_date);
                ReleaseListItemJson {
                    source: OutcomeSource::new(
                        Some(r.release_date.clone()),
                        Some(r.archive_file_sha256.clone()),
                        Some(r.archive_file_size),
                    ),
                    status: if is_pulled { "pulled".to_string() } else { "remote".to_string() },
                }
            })
            .collect();
        let json = serde_json::to_string(&json_items).unwrap();
        assert_eq!(
            json,
            r#"[{"source":{"version":"2026-07-31","hash":"sha256:dummy_sha","bytes":37983173},"status":"remote"}]"#
        );
    }

    #[test]
    fn test_thousand_releases_frame_widens() {
        let mut releases = Vec::new();
        for i in 0..1000 {
            releases.push(mock_release(&format!("2026-{:04}", i), 28_000_000));
        }
        let mut held = HashSet::new();
        held.insert(releases[0].release_date.clone());

        let out_default = render_trud_releases(&releases, &held, false, false);
        let out_all = render_trud_releases(&releases, &held, true, false);

        // Find footer lines
        let def_footer = out_default.lines().find(|l| l.contains("1000 releases")).unwrap();
        let all_footer = out_all.lines().find(|l| l.contains("1000 releases")).unwrap();

        println!("Default 1000 footer: {}", def_footer);
        println!("All 1000 footer:     {}", all_footer);

        assert!(def_footer.contains("--all to see more"));
        assert!(all_footer.contains("pulled"));
    }
}

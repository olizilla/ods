use anyhow::{bail, Context, Result};
use arrow::array::{Array, Date32Array, ListArray, StringArray};
use clap::{Parser, ValueEnum};
use comfy_table::{presets, ContentArrangement, Table};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde::Serialize;
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Table,
    Markdown,
    Csv,
    Json,
}

#[derive(Parser, Debug, Clone)]
pub struct Args {
    /// Search query matching role name or role code (case-insensitive substring)
    pub query: Option<String>,

    /// Output comma-separated role codes for pasting into --role (takes precedence over --format)
    #[arg(long)]
    pub codes: bool,

    /// Output format
    #[arg(long, short, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,

    /// Input directory containing Parquet files (defaults to active release)
    #[arg(long, short)]
    pub input: Option<PathBuf>,

    /// Count closed and inactive organisations as holders too
    #[arg(long)]
    pub all: bool,

    /// Disable ANSI colored output
    #[arg(long)]
    pub plain: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            query: None,
            codes: false,
            format: OutputFormat::Table,
            input: None,
            all: false,
            plain: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RoleEntry {
    pub role_code: String,
    pub role_name: String,
    pub holders: usize,
}

pub fn run(args: Args) -> Result<()> {
    let resolved_input = crate::workspace::resolve_parquet_input(args.input.as_deref())?;
    let color = crate::ansi::stdout_color_enabled(args.plain);
    let explicit_width = crate::ansi::resolve_display_width();
    run_with_writer_color_width(args, &mut std::io::stdout(), &resolved_input, color, explicit_width)
}

pub fn run_with_writer(args: Args, writer: &mut dyn std::io::Write, parquet_dir: &Path) -> Result<()> {
    run_with_writer_color(args, writer, parquet_dir, false)
}

pub fn run_with_writer_color(args: Args, writer: &mut dyn std::io::Write, parquet_dir: &Path, color: bool) -> Result<()> {
    run_with_writer_color_width(args, writer, parquet_dir, color, None)
}

pub fn run_with_writer_color_width(
    args: Args,
    writer: &mut dyn std::io::Write,
    parquet_dir: &Path,
    color: bool,
    explicit_width: Option<u16>,
) -> Result<()> {
    crate::workspace::check_release_provenance(parquet_dir)?;
    let path = parquet_dir.join("orgs.parquet");
    if !path.exists() {
        if args.input.is_some()
            || parquet_dir.join(crate::datapackage::DATAPACKAGE_FILENAME).exists()
            || parquet_dir.join("oci").is_dir()
        {
            anyhow::bail!(
                "✖ Parquet file 'orgs.parquet' not found in '{}'",
                parquet_dir.display()
            );
        }
        if let Ok(ws) = crate::workspace::Workspace::open(None) {
            let releases = ws.releases().unwrap_or_default();
            if !releases.is_empty() {
                let n = releases.len();
                let count_str = if n == 1 { "1 release".to_string() } else { format!("{} releases", n) };
                let newest_date = &releases[0].date;
                let ws_name = ws.root().file_name().and_then(|n| n.to_str()).unwrap_or(crate::workspace::DEFAULT_WORKSPACE_DIR);
                anyhow::bail!(
                    "✖ No active release pinned\n  {} in {}/releases/, none active.\n  Pin one:  ods use {}",
                    count_str,
                    ws_name,
                    newest_date
                );
            }
        }
        anyhow::bail!(
            "✖ no ods workspace found here\n  Pass -i <trud.zip> -o <dir>, or run `ods pull` or `ods trud pull` to create a workspace."
        );
    }

    let holder_counts = count_role_holders(&path, args.all)?;
    let vocab = crate::roles::role_names();

    let mut entries: Vec<RoleEntry> = Vec::with_capacity(vocab.names.len());

    let norm_query = args.query.as_deref().map(|q| q.trim().to_lowercase());

    for (code, name) in &vocab.names {
        let count = holder_counts.get(code).copied().unwrap_or(0);

        if let Some(ref q) = norm_query {
            let code_matches = code.to_lowercase().contains(q);
            let name_matches = name.to_lowercase().contains(q);
            if !code_matches && !name_matches {
                continue;
            }
        }

        entries.push(RoleEntry {
            role_code: code.clone(),
            role_name: name.clone(),
            holders: count,
        });
    }

    if entries.is_empty() {
        if let Some(ref q) = args.query {
            let suggestions = crate::roles::find_role_suggestions(q.trim());
            let mut msg = format!("✖ No role matches '{}'\n", q.trim());
            if !suggestions.is_empty() {
                let names_str = suggestions.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(" · ");
                msg.push_str(&format!("  Did you mean: {}", names_str));
            }
            bail!("{}", msg);
        }
    }

    // Sort by holder count descending, then role name ascending
    entries.sort_by(|a, b| b.holders.cmp(&a.holders).then_with(|| a.role_name.cmp(&b.role_name)));

    // --codes always wins over --format
    if args.codes {
        let code_list: Vec<&str> = entries.iter().map(|e| e.role_code.as_str()).collect();
        writeln!(writer, "{}", code_list.join(","))?;
        return Ok(());
    }

    match args.format {
        OutputFormat::Table | OutputFormat::Markdown => {
            let is_table = args.format == OutputFormat::Table;
            let source_header = crate::workspace::format_source_header(parquet_dir, "orgs.parquet", color && is_table);
            for line in &source_header {
                writeln!(writer, "{}", line)?;
            }
            if args.format == OutputFormat::Markdown {
                writeln!(writer)?;
            }

            let rendered = render_role_table(&entries, args.format, color, explicit_width);
            writeln!(writer, "{}", rendered)?;
        }
        OutputFormat::Csv => {
            writeln!(writer, "role_code,role_name,holders")?;
            for e in &entries {
                writeln!(
                    writer,
                    "{},{},{}",
                    crate::roles::escape_csv(&e.role_code),
                    crate::roles::escape_csv(&e.role_name),
                    e.holders
                )?;
            }
        }
        OutputFormat::Json => {
            writeln!(writer, "{}", serde_json::to_string_pretty(&entries)?)?;
        }
    }

    Ok(())
}

/// Renders the `role` table (or, for `markdown`, a plain ASCII table that never
/// wraps). `explicit_width` follows the width policy shared with `find`
/// (`crate::ansi::apply_table_width_policy`): `Some(width)` wraps to that width,
/// `None` disables wrapping so every row stays on one line.
fn render_role_table(
    entries: &[RoleEntry],
    format: OutputFormat,
    color: bool,
    explicit_width: Option<u16>,
) -> String {
    let is_table = format == OutputFormat::Table;
    let mut table = Table::new();

    if format == OutputFormat::Markdown {
        table.load_style(presets::ASCII_MARKDOWN);
        table.set_content_arrangement(ContentArrangement::Disabled);
    } else {
        table.load_style(presets::UTF8_FULL_CONDENSED);
        table.set_truncation_indicator("…");
        crate::ansi::apply_table_width_policy(&mut table, explicit_width);
    }

    table.set_header(vec!["Code", "Name", "Holders"]);

    for e in entries {
        let (code_str, name_str, holders_str) = if color && is_table {
            if e.holders > 0 {
                (
                    format!("{}{}{}", crate::ansi::ANSI_YELLOW, e.role_code, crate::ansi::ANSI_RESET),
                    e.role_name.clone(),
                    format_number_with_commas(e.holders),
                )
            } else {
                (
                    format!("{}{}{}", crate::ansi::ANSI_MUTED, e.role_code, crate::ansi::ANSI_RESET),
                    format!("{}{}{}", crate::ansi::ANSI_MUTED, e.role_name, crate::ansi::ANSI_RESET),
                    format!("{}{}{}", crate::ansi::ANSI_MUTED, format_number_with_commas(e.holders), crate::ansi::ANSI_RESET),
                )
            }
        } else {
            (
                e.role_code.clone(),
                e.role_name.clone(),
                format_number_with_commas(e.holders),
            )
        };

        table.add_row(vec![code_str, name_str, holders_str]);
    }

    if format == OutputFormat::Table && color {
        crate::ansi::dim_borders(&table.to_string())
    } else {
        table.to_string()
    }
}

fn count_role_holders(path: &Path, include_inactive: bool) -> Result<HashMap<String, usize>> {
    let file = File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let reader = builder.build()?;

    let mut counts: HashMap<String, usize> = HashMap::new();

    for batch in reader {
        let batch = batch?;
        let schema = batch.schema();
        let role_codes_idx = schema.index_of("role_codes").context("role_codes column")?;
        let role_codes_arr = batch
            .column(role_codes_idx)
            .as_any()
            .downcast_ref::<ListArray>()
            .context("role_codes ListArray")?;
        let status_idx = schema.index_of("status").context("status column")?;
        let status_arr = batch
            .column(status_idx)
            .as_any()
            .downcast_ref::<StringArray>()
            .context("status StringArray")?;
        let legal_end_arr = schema.index_of("legal_end").ok()
            .and_then(|i| batch.column(i).as_any().downcast_ref::<Date32Array>());
        let publication_date_arr = schema.index_of("publication_date").ok()
            .and_then(|i| batch.column(i).as_any().downcast_ref::<Date32Array>());

        for i in 0..batch.num_rows() {
            if !include_inactive {
                if status_arr.value(i) != "active" {
                    continue;
                }
                if let (Some(legal_end), Some(publication_date)) = (legal_end_arr, publication_date_arr) {
                    if legal_end.is_valid(i) && publication_date.is_valid(i)
                        && legal_end.value(i) <= publication_date.value(i)
                    {
                        continue;
                    }
                }
            }
            if role_codes_arr.is_valid(i) {
                let value_arr = role_codes_arr.value(i);
                if let Some(str_arr) = value_arr.as_any().downcast_ref::<StringArray>() {
                    for j in 0..str_arr.len() {
                        if str_arr.is_valid(j) {
                            let code = str_arr.value(j);
                            *counts.entry(code.to_string()).or_insert(0) += 1;
                        }
                    }
                }
            }
        }
    }

    Ok(counts)
}

pub fn format_number_with_commas(n: usize) -> String {
    let s = n.to_string();
    let mut result = String::new();
    let bytes = s.as_bytes();
    let len = bytes.len();
    for (i, &b) in bytes.iter().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(b as char);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    /// Protects the guarantee in `docs/cli.md`'s "Table width" section: a table
    /// written to a file or pipe doesn't wrap unless `COLUMNS` says so. Curated role
    /// names (`data/role_names.json`) top out around 55 characters, too short to force
    /// a real row past 120 columns, so this drives `render_role_table` — the exact
    /// function `run_with_writer_color_width` calls — with a deliberately long name.
    #[test]
    fn test_role_table_width_none_disables_wrap_and_columns_60_wraps() {
        let long_name = "Extremely Long Curated Role Name That Exceeds Any Reasonable \
                          Terminal Width By Design For This Test Case Only"
            .to_string();
        let entries = vec![RoleEntry {
            role_code: "ZZ1".to_string(),
            role_name: long_name.clone(),
            holders: 42,
        }];

        // No resolved width (piped, COLUMNS unset, stdout not a terminal): the row
        // stays on one line, longer than 120 characters.
        let unwrapped = render_role_table(&entries, OutputFormat::Table, false, None);
        assert!(
            unwrapped.contains(&long_name),
            "the whole name should appear on one line when unwrapped:\n{unwrapped}"
        );
        let row_line = unwrapped
            .lines()
            .find(|l| l.contains("ZZ1"))
            .expect("row present");
        assert!(
            row_line.width() > 120,
            "unwrapped row should stay a single line over 120 columns wide: {row_line:?}"
        );

        // COLUMNS=60: the same row wraps, so the whole name no longer appears contiguously.
        let wrapped = render_role_table(&entries, OutputFormat::Table, false, Some(60));
        assert!(
            !wrapped.contains(&long_name),
            "the name should wrap across lines once a width is resolved:\n{wrapped}"
        );
    }
}

use anyhow::{bail, Context, Result};
use arrow::array::{Array, ListArray, StringArray};
use clap::{Parser, ValueEnum};
use comfy_table::{presets, ContentArrangement, Table};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde::Serialize;
use std::collections::HashMap;
use std::fs::File;
use std::io::IsTerminal;
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
}

#[derive(Debug, Clone, Serialize)]
pub struct RoleEntry {
    pub role_code: String,
    pub role_name: String,
    pub holders: usize,
}

pub fn run(args: Args) -> Result<()> {
    let resolved_input = crate::workspace::resolve_parquet_input(args.input.as_deref())?;
    run_with_writer(args, &mut std::io::stdout(), &resolved_input)
}

pub fn run_with_writer(args: Args, writer: &mut dyn std::io::Write, parquet_dir: &Path) -> Result<()> {
    let path = parquet_dir.join("orgs.parquet");
    if !path.exists() {
        if args.input.is_some() || parquet_dir.join(crate::provenance::PROVENANCE_FILENAME).exists() {
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

    let holder_counts = count_role_holders(&path)?;
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
            let mut table = Table::new();

            if args.format == OutputFormat::Markdown {
                table.load_preset(presets::ASCII_MARKDOWN);
                table.set_content_arrangement(ContentArrangement::Disabled);
            } else {
                table.load_preset(presets::UTF8_FULL_CONDENSED);
                table.set_content_arrangement(ContentArrangement::Dynamic);
                if std::io::stdout().is_terminal() {
                    if let Ok((cols, _)) = crossterm::terminal::size() {
                        table.set_width(cols);
                    }
                } else {
                    table.set_width(120);
                }
            }

            table.set_header(vec!["Code", "Name", "Holders"]);

            for e in &entries {
                table.add_row(vec![
                    &e.role_code,
                    &e.role_name,
                    &format_number_with_commas(e.holders),
                ]);
            }

            writeln!(writer, "{}", table)?;
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

    let is_machine = args.codes || args.format == OutputFormat::Json || args.format == OutputFormat::Csv;
    let release_date = crate::provenance::OdsProvenance::load_from_dir(parquet_dir)
        .and_then(|p| p.trud_release_date)
        .or_else(|| {
            crate::workspace::find_workspace_root_from(parquet_dir, None)
                .and_then(|r| crate::workspace::Workspace::open(Some(&r)).ok())
                .and_then(|ws| ws.active_release().ok().map(|(d, _)| d))
        });

    if let Some(ref d) = release_date {
        crate::workspace::report_release_resolution(d, Some(parquet_dir), is_machine);
    }

    Ok(())
}

fn count_role_holders(path: &Path) -> Result<HashMap<String, usize>> {
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

        for i in 0..batch.num_rows() {
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

fn format_number_with_commas(n: usize) -> String {
    let s = n.to_string();
    let mut result = String::new();
    let bytes = s.as_bytes();
    let len = bytes.len();
    for (i, &b) in bytes.iter().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            result.push(',');
        }
        result.push(b as char);
    }
    result
}

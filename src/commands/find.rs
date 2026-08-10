use anyhow::{Context, Result};
use arrow::array::{Array, StringArray};
use clap::{Parser, ValueEnum};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use std::fs::File;
use std::path::{Path, PathBuf};

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Table,
    Csv,
    Json,
}

#[derive(Parser, Debug, Clone)]
pub struct Args {
    /// Query string (searches ODS code or Name)
    pub query: Option<String>,

    /// Optional primary role filter
    #[arg(long, short)]
    pub role: Option<String>,

    /// Query the complete historical database (orgs_all.parquet) instead of active only (orgs.parquet)
    #[arg(long, short)]
    pub all: bool,

    /// Output format: table, csv, json
    #[arg(long, short, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,

    /// Input directory containing Parquet files (orgs.parquet, etc.)
    #[arg(long, short, default_value = ".")]
    pub input: PathBuf,
}

pub fn run(args: Args) -> Result<()> {
    let input = args.input.clone();
    run_with_writer(args, &mut std::io::stdout(), &input)
}

pub fn run_with_writer(args: Args, writer: &mut dyn std::io::Write, parquet_dir: &Path) -> Result<()> {
    let file_name = if args.all { "orgs_all.parquet" } else { "orgs.parquet" };
    let path = parquet_dir.join(file_name);
    if !path.exists() {
        anyhow::bail!("Parquet file not found at: {}. Please run `ods parquet` first.", path.display());
    }

    let file = File::open(&path).with_context(|| format!("opening {}", path.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let reader = builder.build()?;

    let query_str = match args.query {
        Some(ref q) => q.clone(),
        None => {
            eprintln!("Launching interactive mode... (press 'q' to exit, '/' to search)");
            return crate::tui::run(args);
        }
    };

    let query_words: Vec<String> = query_str.to_lowercase()
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();
    let role_filter_lower = args.role.as_ref().map(|r| r.to_lowercase());

    let mut matched_count = 0;

    if args.format == OutputFormat::Csv {
        writeln!(writer, "ods_code,name,record_class,status,role,postcode,commissioner,parent")?;
    } else if args.format == OutputFormat::Table {
        writeln!(
            writer,
            "{:<10} | {:<45} | {:<12} | {:<8} | {:<25} | {:<9}",
            "ODS Code", "Name", "Class", "Status", "Role", "Postcode"
        )?;
        writeln!(writer, "{:-<116}", "")?;
    }

    for batch in reader {
        let batch = batch?;
        let schema = batch.schema();
        let num_rows = batch.num_rows();

        let ods_code_arr = batch.column(schema.index_of("ods_code")?)
            .as_any().downcast_ref::<StringArray>().context("ods_code StringArray")?;
        let name_arr = batch.column(schema.index_of("name")?)
            .as_any().downcast_ref::<StringArray>().context("name StringArray")?;
        let record_class_arr = batch.column(schema.index_of("record_class")?)
            .as_any().downcast_ref::<StringArray>().context("record_class StringArray")?;
        let status_arr = batch.column(schema.index_of("status")?)
            .as_any().downcast_ref::<StringArray>().context("status StringArray")?;
        let role_arr = batch.column(schema.index_of("role")?)
            .as_any().downcast_ref::<StringArray>().context("role StringArray")?;

        let postcode_idx = schema.index_of("postcode").ok();
        let commissioner_idx = schema.index_of("commissioner").ok();
        let parent_idx = schema.index_of("parent").ok();

        let postcode_arr = postcode_idx.and_then(|idx| batch.column(idx).as_any().downcast_ref::<StringArray>());
        let commissioner_arr = commissioner_idx.and_then(|idx| batch.column(idx).as_any().downcast_ref::<StringArray>());
        let parent_arr = parent_idx.and_then(|idx| batch.column(idx).as_any().downcast_ref::<StringArray>());

        for i in 0..num_rows {
            let code = ods_code_arr.value(i);
            let name = name_arr.value(i);
            let class = record_class_arr.value(i);
            let status = status_arr.value(i);
            let role = role_arr.value(i);
            
            let postcode = postcode_arr.map(|arr| if arr.is_valid(i) { arr.value(i) } else { "" }).unwrap_or("");
            let commissioner = commissioner_arr.map(|arr| if arr.is_valid(i) { arr.value(i) } else { "" }).unwrap_or("");
            let parent = parent_arr.map(|arr| if arr.is_valid(i) { arr.value(i) } else { "" }).unwrap_or("");

            let matched = query_words.iter().all(|word| {
                code.to_lowercase().starts_with(word)
                || name.to_lowercase().contains(word)
                || postcode.to_lowercase().replace(" ", "").contains(word)
            });

            if matched {
                if let Some(ref role_filter) = role_filter_lower {
                    if !role.to_lowercase().contains(role_filter) {
                        continue;
                    }
                }

                matched_count += 1;

                match args.format {
                    OutputFormat::Json => {
                        let json = serde_json::json!({
                            "ods_code": code,
                            "name": name,
                            "record_class": class,
                            "status": status,
                            "role": role,
                            "postcode": postcode,
                            "commissioner": commissioner,
                            "parent": parent,
                        });
                        writeln!(writer, "{}", json)?;
                    }
                    OutputFormat::Csv => {
                        let escape_csv = |s: &str| {
                            if s.contains(',') || s.contains('"') || s.contains('\n') {
                                format!("\"{}\"", s.replace('"', "\"\""))
                            } else {
                                s.to_string()
                            }
                        };
                        writeln!(
                            writer,
                            "{},{},{},{},{},{},{},{}",
                            code,
                            escape_csv(name),
                            class,
                            status,
                            escape_csv(role),
                            postcode,
                            escape_csv(commissioner),
                            escape_csv(parent)
                        )?;
                    }
                    OutputFormat::Table => {
                        let name_truncated = if name.len() > 45 { &name[..42] } else { name };
                        let name_display = if name.len() > 45 { format!("{}...", name_truncated) } else { name.to_string() };
                        let role_truncated = if role.len() > 25 { &role[..22] } else { role };
                        let role_display = if role.len() > 25 { format!("{}...", role_truncated) } else { role.to_string() };
                        writeln!(
                            writer,
                            "{:<10} | {:<45} | {:<12} | {:<8} | {:<25} | {:<9}",
                            code, name_display, class, status, role_display, postcode
                        )?;
                    }
                }
            }
        }
    }

    if args.format == OutputFormat::Table {
        writeln!(writer, "\nFound {} matching records.", matched_count)?;
    }

    Ok(())
}


use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
pub struct MakeArgs {
    #[command(subcommand)]
    pub command: Option<MakeCommand>,

    /// Input XML or ZIP file path when running default make
    #[arg(long, short)]
    pub input: Option<PathBuf>,

    /// Output workspace or directory path
    #[arg(long, short)]
    pub output: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
pub enum MakeCommand {
    /// Generate target projections (Parquet tables + OKF Markdown wiki) from XML
    All {
        #[arg(long, short)]
        input: PathBuf,
        #[arg(long, short)]
        output: Option<PathBuf>,
    },

    /// Generate columnar Parquet tables from TRUD XML
    Parquet(crate::commands::parquet::Args),

    /// Generate OKF Markdown wiki archive from Parquet tables
    #[command(alias = "md")]
    Markdown(crate::commands::md::Args),

    /// Generate canonical NDJSON document stream from TRUD XML (hidden)
    #[command(hide = true)]
    Ndjson(crate::commands::ndjson::Args),
}

pub fn run(args: MakeArgs) -> Result<()> {
    match args.command {
        Some(MakeCommand::All { input, output }) => {
            eprintln!("Generating dataset target projections (Parquet + Markdown)...");
            
            // 1. Generate Parquet
            let parquet_out = output.clone().unwrap_or_else(|| {
                if let Some(root) = crate::workspace::find_workspace_root() {
                    root.join("current")
                } else {
                    PathBuf::from(".")
                }
            });
            crate::commands::parquet::run(crate::commands::parquet::Args {
                input: input.clone(),
                output: parquet_out.clone(),
            })?;

            // 2. Generate Markdown
            let md_out = parquet_out.join("markdown").join("wiki.zip");
            crate::commands::md::run(crate::commands::md::Args {
                input: parquet_out.clone(),
                output: md_out,
            })?;

            // 3. Write SHA256SUMS and update _provenance.json
            crate::provenance::update_provenance_and_write_sha256sums(&parquet_out)?;

            crate::commands::parquet::warn_unexpected_files(&parquet_out);

            eprintln!("✓ Dataset target projections generated successfully.");
            Ok(())
        }
        Some(MakeCommand::Parquet(parquet_args)) => crate::commands::parquet::run(parquet_args),
        Some(MakeCommand::Markdown(md_args)) => crate::commands::md::run(md_args),
        Some(MakeCommand::Ndjson(ndjson_args)) => crate::commands::ndjson::run(ndjson_args),
        None => {
            // Bare `ods make` defaults to `make all` using workspace trud/ directory or local XML
            let input = args.input.unwrap_or_else(|| {
                if let Some(root) = crate::workspace::find_workspace_root() {
                    let trud_dir = root.join("current").join("trud");
                    if trud_dir.exists() {
                        return trud_dir;
                    }
                }
                PathBuf::from(".")
            });
            run(MakeArgs {
                command: Some(MakeCommand::All {
                    input,
                    output: args.output,
                }),
                input: None,
                output: None,
            })
        }
    }
}


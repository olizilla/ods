use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
pub struct CompileArgs {
    #[command(subcommand)]
    pub command: Option<CompileCommand>,

    /// Input XML or ZIP file path when running default compile
    #[arg(long, short)]
    pub input: Option<PathBuf>,

    /// Output workspace or directory path
    #[arg(long, short)]
    pub output: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
pub enum CompileCommand {
    /// Compile all projections (Parquet + Markdown + NDJSON) from XML
    All {
        #[arg(long, short)]
        input: PathBuf,
        #[arg(long, short)]
        output: Option<PathBuf>,
    },

    /// Convert TRUD XML to columnar Parquet tables
    Parquet(crate::commands::parquet::Args),

    /// Convert Parquet tables to OKF Markdown wiki archive
    #[command(alias = "md")]
    Markdown(crate::commands::md::Args),

    /// Convert TRUD XML to canonical NDJSON document stream
    Ndjson(crate::commands::ndjson::Args),
}

pub fn run(args: CompileArgs) -> Result<()> {
    match args.command {
        Some(CompileCommand::All { input, output }) => {
            eprintln!("Compiling all dataset projections (Parquet + Markdown + NDJSON)...");
            
            // 1. Compile Parquet
            let parquet_out = output.clone().unwrap_or_else(|| PathBuf::from("./ods_data/current/parquet"));
            crate::commands::parquet::run(crate::commands::parquet::Args {
                input: input.clone(),
                output: Some(parquet_out.clone()),
            })?;

            // 2. Compile Markdown
            let md_out = output.clone().unwrap_or_else(|| PathBuf::from("./ods_data/current/markdown/wiki.zip"));
            crate::commands::md::run(crate::commands::md::Args {
                input: Some(parquet_out),
                output: Some(md_out),
            })?;

            // 3. Compile NDJSON
            let ndjson_out = output.unwrap_or_else(|| PathBuf::from("./ods_data/current/ndjson"));
            crate::commands::ndjson::run(crate::commands::ndjson::Args {
                input,
                output: Some(ndjson_out),
            })?;

            eprintln!("✓ All dataset projections compiled successfully.");
            Ok(())
        }
        Some(CompileCommand::Parquet(parquet_args)) => crate::commands::parquet::run(parquet_args),
        Some(CompileCommand::Markdown(md_args)) => crate::commands::md::run(md_args),
        Some(CompileCommand::Ndjson(ndjson_args)) => crate::commands::ndjson::run(ndjson_args),
        None => {
            // Bare `ods compile` defaults to `compile all` using workspace trud/ directory or local XML
            let input = args.input.unwrap_or_else(|| {
                if let Some(root) = crate::workspace::find_workspace_root() {
                    let trud_dir = root.join("current").join("trud");
                    if trud_dir.exists() {
                        return trud_dir;
                    }
                }
                PathBuf::from(".")
            });
            run(CompileArgs {
                command: Some(CompileCommand::All {
                    input,
                    output: args.output,
                }),
                input: None,
                output: None,
            })
        }
    }
}


use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
pub struct MakeArgs {
    #[command(subcommand)]
    pub command: Option<MakeCommand>,

    /// TRUD XML file or ZIP archive input path [default: active release in workspace]
    #[arg(long, short)]
    pub input: Option<PathBuf>,

    /// Output release directory path [default: active release in workspace]
    #[arg(long, short)]
    pub output: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
pub enum MakeCommand {
    /// Generate target projections (Parquet tables) from XML
    Parquet(crate::commands::parquet::Args),

    /// Generate OCI image layout for a compiled release
    Oci(crate::commands::make_oci::Args),

    /// Cut and validate a publishable release row for data/releases.json
    Release(crate::commands::make_release::Args),
}

pub fn run(args: MakeArgs) -> Result<()> {
    match args.command {
        Some(MakeCommand::Parquet(parquet_args)) => run_make_parquet(parquet_args).map(|_| ()),
        Some(MakeCommand::Oci(oci_args)) => crate::commands::make_oci::run(oci_args),
        Some(MakeCommand::Release(release_args)) => crate::commands::make_release::run(release_args),
        None => run_make_parquet(crate::commands::parquet::Args {
            input: args.input,
            output: args.output,
        }).map(|_| ()),
    }
}

pub fn run_make_parquet(args: crate::commands::parquet::Args) -> Result<PathBuf> {
    eprintln!("Generating dataset target projections (Parquet)...");
    let output_dir = crate::commands::parquet::run(args)?;
    eprintln!("✓ Dataset target projections generated successfully.");
    Ok(output_dir)
}

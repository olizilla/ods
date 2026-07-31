use anyhow::Result;
use clap::{Parser, Subcommand};

use ods::commands;

#[derive(Parser)]
#[command(name = "ods", author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Compile TRUD XML into canonical NDJSON
    Compile(commands::compile::Args),

    /// Export canonical NDJSON into Parquet tables
    Parquet(commands::parquet::Args),

    /// Export Parquet tables into OKF Markdown wiki
    #[command(alias = "md")]
    Markdown(commands::md::Args),

    /// Search for organisations or sites in the Parquet tables
    Find(commands::find::Args),

    /// Print academic citation & provenance block
    Cite(commands::cite::Args),

    /// Compare two TRUD ODS releases (NDJSON, XML, or ZIP) and output diffs
    Diff(commands::diff::Args),
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Compile(args) => commands::compile::run(args),
        Command::Parquet(args) => commands::parquet::run(args),
        Command::Markdown(args) => commands::md::run(args),
        Command::Find(args) => commands::find::run(args),
        Command::Cite(args) => commands::cite::run(args),
        Command::Diff(args) => commands::diff::run(args),
    }
}

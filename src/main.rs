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

    /// Export NDJSON into flattened Parquet
    Parquet(commands::parquet::Args),

    /// Export NDJSON into flat OKF Markdown directory
    Okf(commands::okf::Args),
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Compile(args) => commands::compile::run(args),
        Command::Parquet(args) => commands::parquet::run(args),
        Command::Okf(args) => commands::okf::run(args),
    }
}

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
    /// Search NHS organisations and sites
    Find(commands::find::Args),

    /// Download pre-built dataset releases
    Pull(commands::pull::Args),

    /// Show provenance metadata and academic citation
    Cite(commands::cite::Args),

    /// Build from official NHS source data (requires TRUD API key)
    Trud(commands::trud::TrudArgs),

    /// Compile TRUD XML into Parquet tables and Markdown
    Make(commands::make::MakeArgs),
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Find(args) => commands::find::run(args),
        Command::Pull(args) => commands::pull::run(args),
        Command::Cite(args) => commands::cite::run(args),
        Command::Trud(args) => commands::trud::run(args),
        Command::Make(args) => commands::make::run(args),
    };

    // Release XML is unpacked to scratch space (~660 MB) for the duration of
    // the command only. Remove it on the way out, including on error.
    commands::ndjson::cleanup_scratch();

    // Commands that have already printed their own diagnostics signal failure
    // with this marker: set a non-zero exit status without printing again.
    if let Err(ref e) = result {
        if e.downcast_ref::<commands::pull::AlreadyReported>().is_some() {
            std::process::exit(1);
        }
    }

    result
}

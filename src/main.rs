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

    /// Show full details for a single organisation by ODS code
    Info(commands::info::Args),

    /// Search and list role codes and names with holder counts
    Role(commands::role::Args),

    /// Download pre-built dataset releases
    Pull(commands::pull::Args),

    /// Show provenance metadata and academic citation
    Cite(commands::cite::Args),

    /// Build from official NHS source data (requires TRUD API key)
    Trud(commands::trud::TrudArgs),

    /// Compile TRUD XML into Parquet tables
    Make(commands::make::MakeArgs),

    /// Pin an active dataset release
    Use(commands::use_cmd::Args),

    /// Audit release data against TRUD source XML
    Audit(commands::audit::Args),
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Find(args) => commands::find::run(args),
        Command::Info(args) => commands::info::run(args),
        Command::Role(args) => commands::role::run(args),
        Command::Pull(args) => commands::pull::run(args),
        Command::Cite(args) => commands::cite::run(args),
        Command::Trud(args) => commands::trud::run(args),
        Command::Make(args) => commands::make::run(args),
        Command::Use(args) => commands::use_cmd::run(args),
        Command::Audit(args) => commands::audit::run(args),
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

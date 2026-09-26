use clap::{Parser, Subcommand};

use ods::commands;

fn version_string() -> &'static str {
    const CARGO_PKG_VERSION: &str = env!("CARGO_PKG_VERSION");
    const GIT_SHA: Option<&str> = option_env!("ODS_GIT_SHA");
    const GIT_DIRTY: Option<&str> = option_env!("ODS_GIT_DIRTY");

    match (GIT_SHA, GIT_DIRTY) {
        (Some(sha), Some("true")) => {
            let short = if sha.len() >= 7 { &sha[..7] } else { sha };
            // Leak to &'static str so Clap can take &'static str without allocations
            Box::leak(format!("{} ({}-dirty)", CARGO_PKG_VERSION, short).into_boxed_str())
        }
        (Some(sha), _) => {
            let short = if sha.len() >= 7 { &sha[..7] } else { sha };
            Box::leak(format!("{} ({})", CARGO_PKG_VERSION, short).into_boxed_str())
        }
        (None, _) => CARGO_PKG_VERSION,
    }
}

#[derive(Parser)]
#[command(name = "ods", author, version = version_string(), about, long_about = None)]
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
}

fn main() {
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
    };

    // Release XML is unpacked to scratch space (~660 MB) for the duration of
    // the command only. Remove it on the way out, including on error.
    ods::ods_xml::cleanup_scratch();

    if let Err(err) = result {
        if err.chain().any(|c| {
            c.downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == std::io::ErrorKind::BrokenPipe)
        }) {
            std::process::exit(0);
        }

        if err.chain().any(|c| c.downcast_ref::<commands::pull::AlreadyReported>().is_some()) {
            std::process::exit(1);
        }

        let causes: Vec<String> = err.chain().map(|c| c.to_string()).collect();
        let message = causes.join(": ");
        let formatted = if message.starts_with('✖') {
            message
        } else {
            format!("✖ {}", message)
        };
        eprintln!("{}", formatted.trim_end_matches('\n'));
        std::process::exit(1);
    }
}

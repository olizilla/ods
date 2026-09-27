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

/// `println!`/`print!` panic instead of returning an error when stdout is a pipe
/// whatever reads it has already closed ("failed printing to stdout: Broken pipe
/// (os error 32)", exit 101) — the `ods … | head` case. Every command uses these
/// macros somewhere, so the smallest fix that covers all of them in one place is a
/// panic hook: let a genuine panic print and exit 101 as always, but a broken-pipe
/// print panic exits quietly with 0, the same as the `io::Error` chain check below
/// already does for commands that propagate the error instead of panicking. The
/// hook calls `cleanup_scratch()` itself before exiting: `std::process::exit`
/// skips the rest of `main`, including the `cleanup_scratch()` call below it, and
/// a command such as `ods trud audit` has already unpacked release XML (~660 MB)
/// to scratch space by the time it's printing its report, so a quiet exit still
/// needs to remove that.
fn install_broken_pipe_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| info.payload().downcast_ref::<&str>().copied());
        let is_broken_pipe = message.is_some_and(|m| {
            m.starts_with("failed printing to stdout") && m.contains("Broken pipe")
        });
        if is_broken_pipe {
            ods::ods_xml::cleanup_scratch();
            std::process::exit(0);
        }
        default_hook(info);
    }));
}

fn main() {
    install_broken_pipe_panic_hook();
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

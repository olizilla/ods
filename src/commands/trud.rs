use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
pub struct TrudArgs {
    #[command(subcommand)]
    pub command: TrudCommand,
}

#[derive(Subcommand, Debug)]
pub enum TrudCommand {
    /// List TRUD release archives, newest first
    List(crate::commands::trud_list::Args),

    /// Pull official release archive from TRUD REST API and verify SHA-256
    Pull(crate::commands::fetch::Args),

    /// Compare two TRUD ODS releases and generate diff report
    Diff(crate::commands::diff::Args),

    /// Audit workspace Parquet projections against ground-truth TRUD XML/ZIP
    Audit(crate::commands::audit::Args),

    /// Verify local release archive SHA-256 against TRUD REST API
    Verify {
        /// Local archive file or directory to verify
        #[arg(value_name = "PATH")]
        path: PathBuf,

        /// TRUD API Key
        #[arg(long, env = "TRUD_API_KEY")]
        api_key: Option<String>,

        /// Custom release index URL or file path
        #[arg(long, hide = true)]
        index: Option<String>,

        /// Workspace path (defaults to ./ods_data)
        #[arg(long, short = 'w')]
        workspace: Option<PathBuf>,
    },
}

pub fn run(args: TrudArgs) -> Result<()> {
    match args.command {
        TrudCommand::List(list_args) => crate::commands::trud_list::run(list_args),
        TrudCommand::Pull(fetch_args) => crate::commands::fetch::run(fetch_args),
        TrudCommand::Diff(diff_args) => crate::commands::diff::run(diff_args),
        TrudCommand::Audit(audit_args) => crate::commands::audit::run(audit_args),
        TrudCommand::Verify { path, api_key, index, workspace } => {
            let fetch_args = crate::commands::fetch::Args {
                api_key,
                verify_only: Some(path),
                index,
                workspace,
                ..Default::default()
            };
            crate::commands::fetch::run(fetch_args)
        }
    }
}


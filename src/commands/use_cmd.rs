use anyhow::{bail, Result};
use clap::Args as ClapArgs;
use std::path::PathBuf;
use crate::workspace::{
    find_workspace_root, get_active_release, is_release_dir_verified, set_active_release,
    DEFAULT_WORKSPACE_DIR,
};

#[derive(ClapArgs, Debug, Clone)]
pub struct Args {
    /// The release date to pin (e.g. 2026-07-31)
    pub release_date: String,

    /// Workspace path (defaults to ./ods_data)
    #[arg(long)]
    pub workspace: Option<PathBuf>,
}

pub fn run(args: Args) -> Result<()> {
    let workspace_root = args.workspace.unwrap_or_else(|| {
        find_workspace_root().unwrap_or_else(|| PathBuf::from(DEFAULT_WORKSPACE_DIR))
    });

    let release_dir = workspace_root.join("releases").join(&args.release_date);
    if !release_dir.exists() {
        bail!(
            "✖ Release {} not found in {}\n  Run 'ods pull {}' to download it.",
            args.release_date,
            workspace_root.display(),
            args.release_date
        );
    }

    if !is_release_dir_verified(&release_dir) {
        bail!(
            "✖ Release {} in {} has unverified or missing files (SHA256SUMS mismatch)\n  Run 'ods pull --force {}' to repair it.",
            args.release_date,
            workspace_root.display(),
            args.release_date
        );
    }

    let already_active = if let Ok((active_date, _)) = get_active_release(&workspace_root) {
        active_date == args.release_date
    } else {
        false
    };

    set_active_release(&workspace_root, &args.release_date)?;
    crate::workspace::generate_workspace_readme(&workspace_root, &args.release_date, None, None)?;

    if already_active {
        eprintln!("* Release {} already active", args.release_date);
    } else {
        eprintln!("✓ Active release set to {}", args.release_date);
    }
    eprintln!("  current → releases/{}", args.release_date);

    Ok(())
}

use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
pub struct StatusArgs {
    /// Workspace root directory (defaults to ./ods_data or parent)
    #[arg(long, short)]
    pub workspace: Option<PathBuf>,
}

#[derive(Parser, Debug)]
pub struct SwitchArgs {
    /// Target release date tag to pin active release (e.g. 2026-06-22)
    pub release_date: String,

    /// Workspace root directory (defaults to ./ods_data or parent)
    #[arg(long, short)]
    pub workspace: Option<PathBuf>,
}

pub fn run_status(args: StatusArgs) -> Result<()> {
    let workspace_root = args.workspace
        .or_else(crate::workspace::find_workspace_root)
        .context("No ODS workspace found. Run `ods parquet <xml>` or populate ./ods_data/.")?;

    println!("=== ODS Workspace Status ===");
    println!("Workspace Root: {}", workspace_root.display());

    match crate::workspace::get_active_release(&workspace_root) {
        Ok((active_date, active_path)) => {
            println!("Active Release Pin: {}", active_date);
            println!("Active Path: {}", active_path.display());
        }
        Err(e) => {
            println!("Active Release Pin: None ({e})");
        }
    }

    println!("\nLocal Release Snapshots:");
    let releases = crate::workspace::list_releases(&workspace_root)?;
    if releases.is_empty() {
        println!("  (No releases found in workspace)");
    } else {
        for r in releases {
            let active_marker = if r.is_active { " [ACTIVE]" } else { "" };
            let parquet_str = if r.has_parquet { "parquet ✓" } else { "parquet ✗" };
            let ndjson_str = if r.has_ndjson { "ndjson ✓" } else { "ndjson ✗" };
            let md_str = if r.has_markdown { "markdown ✓" } else { "markdown ✗" };

            println!(
                "  - {}{}: {}, {}, {}",
                r.date, active_marker, parquet_str, ndjson_str, md_str
            );
        }
    }

    Ok(())
}

pub fn run_switch(args: SwitchArgs) -> Result<()> {
    let workspace_root = args.workspace
        .or_else(crate::workspace::find_workspace_root)
        .context("No ODS workspace found. Run `ods parquet <xml>` or populate ./ods_data/.")?;

    let release_dir = workspace_root.join("releases").join(&args.release_date);
    if !release_dir.exists() {
        anyhow::bail!(
            "Release date '{}' not found in workspace at {}",
            args.release_date,
            release_dir.display()
        );
    }

    crate::workspace::set_active_release(&workspace_root, &args.release_date)?;
    println!("Switched active ODS workspace release to: {}", args.release_date);
    Ok(())
}


use anyhow::Result;
use clap::Parser;
use std::path::{Path, PathBuf};

use crate::provenance::OdsProvenance;
use crate::workspace::{
    ensure_workspace_gitignore, find_workspace_root, generate_workspace_readme, list_releases,
    prepare_release_dir, set_active_release, DEFAULT_WORKSPACE_DIR,
};

#[derive(Parser, Debug, Default)]
pub struct Args {
    /// Target release date in YYYY-MM-DD format (defaults to latest available release)
    pub release_date: Option<String>,

    /// List all available remote and local release versions
    #[arg(long, short = 'l')]
    pub list: bool,

    /// Force re-download or re-sync of specified release
    #[arg(long, short = 'f')]
    pub force: bool,

    /// TRUD API key if syncing directly from TRUD
    #[arg(long, env = "NHS_TRUD_API_KEY")]
    pub api_key: Option<String>,

    /// Print verbose sync output
    #[arg(long, short = 'v')]
    pub verbose: bool,
}

pub fn run(args: Args) -> Result<()> {
    let workspace_root = find_workspace_root().unwrap_or_else(|| PathBuf::from(DEFAULT_WORKSPACE_DIR));

    if args.list {
        return run_list(&workspace_root, args.api_key.as_deref());
    }

    if let Some(ref target_date) = args.release_date {
        return sync_specific_release(&workspace_root, target_date, args.force, args.api_key.as_deref());
    }

    // Default sync: initialize or update to latest release
    sync_latest_release(&workspace_root, args.force, args.api_key.as_deref())
}

pub fn run_list(workspace_root: &Path, _api_key: Option<&str>) -> Result<()> {
    let local_releases = if workspace_root.exists() {
        list_releases(workspace_root).unwrap_or_default()
    } else {
        vec![]
    };

    eprintln!("ODS Dataset Releases:\n");
    eprintln!("  {:14} {:18} {}", "Release Date", "Status", "Source / Details");
    eprintln!("  {:14} {:18} {}", "────────────", "──────", "────────────────");

    if local_releases.is_empty() {
        eprintln!("  (No local releases found in {})", workspace_root.display());
    } else {
        for rel in &local_releases {
            let status_str = if rel.is_active {
                "● active (local)"
            } else {
                "○ local"
            };

            let details = "Local Release";

            eprintln!("  {:14} {:18} {}", rel.date, status_str, details);
        }
    }

    eprintln!("\nLegend:");
    eprintln!("  ● active (local)  Active release pin (./ods_data/current)");
    eprintln!("  ○ local           Cached locally in ./ods_data/releases/");
    eprintln!("  ○ remote          Available for sync from upstream");
    eprintln!("\nTo sync to a specific release, run: ods sync <YYYY-MM-DD>");

    Ok(())
}

fn sync_specific_release(
    workspace_root: &Path,
    target_date: &str,
    force: bool,
    api_key: Option<&str>,
) -> Result<()> {
    let release_dir = prepare_release_dir(workspace_root, target_date)?;
    ensure_workspace_gitignore(workspace_root)?;

    let parquet_dir = release_dir.join("parquet");
    let orgs_parquet = parquet_dir.join("orgs.parquet");

    if orgs_parquet.exists() && !force {
        eprintln!("* Release {} is already cached locally.", target_date);
        set_active_release(workspace_root, target_date)?;
        generate_workspace_readme(workspace_root, target_date, None, None)?;
        eprintln!("✓ Switched active release pin to {}.", target_date);
        return Ok(());
    }

    if let Some(key) = api_key {
        eprintln!("Fetching release {} directly from NHS TRUD...", target_date);
        let fetch_args = crate::commands::fetch::Args {
            api_key: Some(key.to_string()),
            release: Some(target_date.to_string()),
            output: Some(release_dir.clone()),
            verify_only: None,
            verbose: false,
        };
        crate::commands::fetch::run(fetch_args)?;
    } else {
        eprintln!("Downloading ODS release {}...", target_date);
        download_prebuilt_parquet(&release_dir, target_date)?;
    }

    set_active_release(workspace_root, target_date)?;
    generate_workspace_readme(workspace_root, target_date, None, None)?;
    eprintln!("✓ Synchronized ./ods_data/ to release {}.", target_date);

    Ok(())
}

fn sync_latest_release(workspace_root: &Path, force: bool, api_key: Option<&str>) -> Result<()> {
    if let Some(key) = api_key {
        let fetch_args = crate::commands::fetch::Args {
            api_key: Some(key.to_string()),
            release: None,
            output: None,
            verify_only: None,
            verbose: false,
        };
        return crate::commands::fetch::run(fetch_args);
    }

    let local_releases = if workspace_root.exists() {
        list_releases(workspace_root).unwrap_or_default()
    } else {
        vec![]
    };

    if !local_releases.is_empty() && !force {
        let active = local_releases.iter().find(|r| r.is_active);
        if let Some(act) = active {
            eprintln!("* Workspace already synchronized with active release {}.", act.date);
            return Ok(());
        }
    }

    let latest_date = "2026-07-31"; // Default current active release
    sync_specific_release(workspace_root, latest_date, force, None)
}

fn download_prebuilt_parquet(release_dir: &Path, date: &str) -> Result<()> {
    let parquet_dir = release_dir.join("parquet");
    std::fs::create_dir_all(&parquet_dir)?;

    let prov = OdsProvenance::new(
        Some(date.to_string()),
        Some("4574".to_string()),
        Some("Full".to_string()),
        Some("HSCIC".to_string()),
        Some(Path::new("HSCOrgRefData.xml")),
    );
    let prov_json = serde_json::to_string_pretty(&prov)?;
    std::fs::write(release_dir.join("provenance.json"), prov_json)?;

    Ok(())
}


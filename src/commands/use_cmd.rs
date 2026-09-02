use anyhow::{bail, Result};
use clap::Args as ClapArgs;
use std::path::PathBuf;
use crate::workspace::Workspace;

#[derive(ClapArgs, Debug, Clone)]
pub struct Args {
    /// The release date to pin (e.g. 2026-07-31)
    pub release_date: String,

    /// Workspace path (defaults to ./ods_data)
    #[arg(long)]
    pub workspace: Option<PathBuf>,
}

pub fn run(args: Args) -> Result<()> {
    let ws = Workspace::open_or_create(args.workspace.as_deref())?;
    let workspace_root = ws.root().to_path_buf();

    let release_dir = workspace_root.join("releases").join(&args.release_date);
    if !release_dir.exists() {
        bail!(
            "✖ Release {} not found in {}\n  Run 'ods pull {}' to download it.",
            args.release_date,
            workspace_root.display(),
            args.release_date
        );
    }

    let index = if let Ok(Some(cached)) = crate::index::CachedReleaseIndex::load_from_workspace(&workspace_root) {
        let baked = crate::index::OdsReleaseIndex::baked().unwrap_or_else(|_| cached.index.clone());
        baked.merge(&cached.index).unwrap_or(baked)
    } else {
        crate::index::OdsReleaseIndex::baked().unwrap_or_default()
    };

    let outcome = crate::workspace::verify_release_dir(&release_dir, Some(&index));
    match outcome {
        crate::workspace::VerificationOutcome::VerifiedPublished { date, version, digest } => {
            eprintln!("✓ reconstructed manifest {} matches the index for {} ({})", digest, date, version);
        }
        crate::workspace::VerificationOutcome::VerifiedUnpublished { digest, .. } => {
            eprintln!("* reconstructed manifest {} verified (unpublished local release)", digest);
        }
        crate::workspace::VerificationOutcome::Mismatch { date, version, expected_digest, reconstructed_digest } => {
            bail!(
                "✖ reconstructed manifest {} does not match the index for {} ({}): {}\n  A file in this directory does not match the published release.",
                reconstructed_digest,
                date,
                version,
                expected_digest
            );
        }
        crate::workspace::VerificationOutcome::Corrupted(err) => {
            bail!(
                "✖ Release {} in {} is invalid: {}\n  Run 'ods pull --force {}' to repair it.",
                args.release_date,
                workspace_root.display(),
                err,
                args.release_date
            );
        }
    }

    let already_active = if let Ok((active_date, _)) = ws.active_release() {
        active_date == args.release_date
    } else {
        false
    };

    ws.set_active(&args.release_date)?;

    if already_active {
        eprintln!("* Release {} already active", args.release_date);
    } else {
        eprintln!("✓ Active release set to {}", args.release_date);
    }
    eprintln!("  current → releases/{}", args.release_date);

    Ok(())
}

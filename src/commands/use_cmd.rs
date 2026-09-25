use anyhow::{bail, Result};
use clap::Args as ClapArgs;
use crate::workspace::Workspace;

#[derive(ClapArgs, Debug, Clone)]
pub struct Args {
    /// The release date to pin (e.g. 2026-07-31)
    pub release_date: String,
}

pub fn run(args: Args) -> Result<()> {
    run_with_writer(args, std::io::stderr())
}

pub fn run_with_writer<W: std::io::Write>(args: Args, mut err_writer: W) -> Result<()> {
    let ws = Workspace::open(None)?;
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

    let (index, _) = crate::commands::pull::resolve_index(
        &workspace_root,
        None,
        false,
        false,
        &crate::commands::pull::HttpOciFetcher,
    )?;

    let already_active = if let Ok((active_date, _)) = ws.active_release() {
        active_date == args.release_date
    } else {
        false
    };

    ws.set_active(&args.release_date)?;

    if already_active {
        writeln!(err_writer, "* Release {} already active", args.release_date)?;
    } else {
        writeln!(err_writer, "✓ Active release set to {}", args.release_date)?;
    }
    writeln!(err_writer, "  current → releases/{}", args.release_date)?;

    let outcome = crate::workspace::verify_release_dir(&release_dir, &index);
    match outcome {
        crate::workspace::VerificationOutcome::VerifiedPublished { date, version, digest } => {
            writeln!(err_writer, "✓ reconstructed manifest {} matches the index for {} ({})", digest, date, version)?;
        }
        crate::workspace::VerificationOutcome::DateUnknown { digest, .. }
        | crate::workspace::VerificationOutcome::VersionUnpublished { digest, .. }
        | crate::workspace::VerificationOutcome::DifferentArchive {
            published_digest: None,
            digest,
            ..
        } => {
            writeln!(
                err_writer,
                "* reconstructed manifest {} verified (unpublished local release)",
                digest
            )?;
        }
        crate::workspace::VerificationOutcome::DifferentBytes {
            date,
            version,
            published_digest,
            reconstructed_digest,
        }
        | crate::workspace::VerificationOutcome::DifferentArchive {
            date,
            version,
            published_digest: Some(published_digest),
            digest: reconstructed_digest,
            ..
        } => {
            writeln!(
                err_writer,
                "! releases/{} does not match the published {} ({})\n  expected manifest {}\n  got      {}\n  Repair it: ods pull --force {}",
                args.release_date,
                date,
                version,
                published_digest,
                reconstructed_digest,
                args.release_date
            )?;
        }
        crate::workspace::VerificationOutcome::ChangedSinceBuilt { file } => {
            writeln!(
                err_writer,
                "! releases/{} can't be checked: datapackage resource {} hash mismatch\n  Repair it: ods pull --force {}",
                args.release_date,
                file,
                args.release_date
            )?;
        }
        crate::workspace::VerificationOutcome::NoProvenance => {
            writeln!(
                err_writer,
                "! releases/{} can't be checked: Missing or unreadable _provenance.json\n  Repair it: ods pull --force {}",
                args.release_date,
                args.release_date
            )?;
        }
        crate::workspace::VerificationOutcome::Corrupted(err) => {
            writeln!(
                err_writer,
                "! releases/{} can't be checked: {}\n  Repair it: ods pull --force {}",
                args.release_date,
                err,
                args.release_date
            )?;
        }
    }

    Ok(())
}

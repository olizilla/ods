use anyhow::{bail, Result};
use clap::Args as ClapArgs;
use crate::workspace::Workspace;

#[derive(ClapArgs, Debug, Clone)]
pub struct Args {
    /// The release date to pin (e.g. 2026-07-31), or latest for the newest release in the workspace
    pub release_date: String,
}

pub fn run(args: Args) -> Result<()> {
    run_with_writer(args, std::io::stderr())
}

pub fn run_with_writer<W: std::io::Write>(args: Args, mut err_writer: W) -> Result<()> {
    let ws = Workspace::open(None)?;
    let workspace_root = ws.root().to_path_buf();

    let release_date = resolve_release_date(&ws, &workspace_root, &args.release_date)?;

    let release_dir = workspace_root.join("releases").join(&release_date);
    if !release_dir.exists() {
        bail!(
            "✖ Release {} not found in {}\n  Run 'ods pull {}' to download it.",
            release_date,
            workspace_root.display(),
            release_date
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
        active_date == release_date
    } else {
        false
    };

    ws.set_active(&release_date)?;

    if already_active {
        writeln!(err_writer, "* Release {} already active", release_date)?;
    } else {
        writeln!(err_writer, "✓ Active release set to {}", release_date)?;
    }
    writeln!(err_writer, "  current → releases/{}", release_date)?;

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
                release_date,
                date,
                version,
                published_digest,
                reconstructed_digest,
                release_date
            )?;
        }
        crate::workspace::VerificationOutcome::ChangedSinceBuilt { file } => {
            writeln!(
                err_writer,
                "! releases/{} can't be checked: datapackage resource {} hash mismatch\n  Repair it: ods pull --force {}",
                release_date,
                file,
                release_date
            )?;
        }
        crate::workspace::VerificationOutcome::NoProvenance => {
            writeln!(
                err_writer,
                "! releases/{} can't be checked: Missing or unreadable _provenance.json\n  Repair it: ods pull --force {}",
                release_date,
                release_date
            )?;
        }
        crate::workspace::VerificationOutcome::Corrupted(err) => {
            writeln!(
                err_writer,
                "! releases/{} can't be checked: {}\n  Repair it: ods pull --force {}",
                release_date,
                err,
                release_date
            )?;
        }
    }

    Ok(())
}

/// Resolves the `ods use` argument to a release date, before anything else runs.
///
/// `latest` pins the newest local release: the latest-dated directory under `releases/`
/// whose name is a `YYYY-MM-DD` date and which holds `orgs.parquet`. A `YYYY-MM-DD` date is
/// returned as-is (its not-found handling happens later). Anything else is refused.
fn resolve_release_date(ws: &Workspace, workspace_root: &std::path::Path, arg: &str) -> Result<String> {
    if arg == "latest" {
        let releases = ws.releases()?;
        return releases
            .into_iter()
            .find(|r| chrono::NaiveDate::parse_from_str(&r.date, "%Y-%m-%d").is_ok() && r.has_parquet)
            .map(|r| r.date)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "✖ No releases in {}\n  Run: ods pull",
                    workspace_root.display()
                )
            });
    }

    if chrono::NaiveDate::parse_from_str(arg, "%Y-%m-%d").is_ok() {
        return Ok(arg.to_string());
    }

    bail!(
        "✖ {} isn't a release date\n  Name a release as YYYY-MM-DD, or pin the newest one here: ods use latest",
        arg
    );
}

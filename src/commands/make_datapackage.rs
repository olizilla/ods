//! `ods make datapackage`: writes `datapackage.json`, the release's Data Package view, from the
//! object its Parquet files carry, the files themselves and `ods`'s compiled table schemas. `ods
//! make` and `ods pull` write it too. Never packed, and `ods` never reads a value from it, so
//! it can be regenerated any time.

use anyhow::{Context, Result};
use clap::Parser;
use std::path::{Path, PathBuf};

#[derive(Parser, Debug, Clone, Default)]
pub struct Args {
    /// Release directory to read (defaults to active release in workspace)
    #[arg(long, short)]
    pub input: Option<PathBuf>,

    /// Where to write datapackage.json ("-" prints it to stdout; default: the release directory)
    #[arg(long, short)]
    pub output: Option<PathBuf>,
}

pub fn run(args: Args) -> Result<()> {
    let release_dir = match args.input {
        Some(ref p) => p.clone(),
        None => {
            let ws = crate::workspace::Workspace::open(None)?;
            let (date, active_path) = ws.active_release()?;
            crate::workspace::report_inferred_release_write(&date, &active_path);
            active_path
        }
    };

    let index = index_for_view(&release_dir, None)?;
    let json = crate::datapackage::view_json(&release_dir, Some(&index))?;

    match args.output.as_deref() {
        Some(p) if p == Path::new("-") => {
            print!("{}", json);
        }
        Some(p) => {
            std::fs::write(p, &json).with_context(|| format!("writing {}", p.display()))?;
            eprintln!("✓ datapackage.json written  {}", crate::workspace::relative_to_cwd(p).display());
        }
        None => {
            let out_path = release_dir.join(crate::datapackage::DATAPACKAGE_FILENAME);
            std::fs::write(&out_path, &json).with_context(|| format!("writing {}", out_path.display()))?;
            eprintln!(
                "✓ datapackage.json written  {}",
                crate::workspace::relative_to_cwd(&release_dir).display()
            );
        }
    }
    Ok(())
}

/// The release index a view's DOI comes from: `index_arg` (`--index`), else the cached or
/// built-in index of the workspace holding `release_dir` or the working directory, the way
/// `ods cite` finds it. A DOI is a post-publish, index-only fact: the view may use it, since it
/// isn't part of any digest. Never fetched.
pub fn index_for_view(release_dir: &Path, index_arg: Option<&str>) -> Result<crate::index::OdsReleaseIndex> {
    let cwd = std::env::current_dir()?;
    let root = match crate::workspace::find_workspace_root_from(release_dir, None)? {
        Some(ws) => ws,
        None => crate::workspace::find_workspace_root_from(&cwd, None)?.unwrap_or(cwd),
    };
    let (index, _) =
        crate::commands::pull::resolve_index(&root, index_arg, false, false, &crate::commands::pull::HttpOciFetcher)?;
    Ok(index)
}

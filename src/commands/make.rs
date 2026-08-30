use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

#[derive(Parser, Debug)]
pub struct MakeArgs {
    #[command(subcommand)]
    pub command: Option<MakeCommand>,

    /// Input XML or ZIP file path when running default make
    #[arg(long, short)]
    pub input: Option<PathBuf>,

    /// Output workspace or directory path
    #[arg(long, short)]
    pub output: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
pub enum MakeCommand {
    /// Generate target projections (Parquet tables) from XML
    Parquet(crate::commands::parquet::Args),

    /// Generate OCI image layout for a compiled release
    Oci(crate::commands::make_oci::Args),

    /// Cut and validate a publishable release row for data/releases.json
    Release(crate::commands::make_release::Args),

    /// Generate canonical NDJSON document stream from TRUD XML (hidden)
    #[command(hide = true)]
    Ndjson(crate::commands::ndjson::Args),
}

pub fn run(args: MakeArgs) -> Result<()> {
    match args.command {
        Some(MakeCommand::Parquet(parquet_args)) => run_make_parquet(parquet_args),
        Some(MakeCommand::Oci(oci_args)) => crate::commands::make_oci::run(oci_args),
        Some(MakeCommand::Release(release_args)) => crate::commands::make_release::run(release_args),
        Some(MakeCommand::Ndjson(ndjson_args)) => crate::commands::ndjson::run(ndjson_args),
        None => {
            // Bare `ods make` is an alias for `ods make parquet`
            let input = args.input.unwrap_or_else(|| {
                if let Some(root) = crate::workspace::find_workspace_root() {
                    let trud_dir = root.join("current").join("trud");
                    if trud_dir.exists() {
                        return trud_dir;
                    }
                }
                PathBuf::from(".")
            });
            let output = args.output.unwrap_or_else(|| {
                if let Some(root) = crate::workspace::find_workspace_root() {
                    root.join("current")
                } else {
                    PathBuf::from(".")
                }
            });
            run_make_parquet(crate::commands::parquet::Args { input, output })
        }
    }
}

pub fn run_make_parquet(mut args: crate::commands::parquet::Args) -> Result<()> {
    eprintln!("Generating dataset target projections (Parquet)...");

    if (args.input == Path::new("./ods.ndjson") || args.input == Path::new(".")) && !args.input.exists() {
        if let Some(root) = crate::workspace::find_workspace_root() {
            let trud_dir = root.join("current").join("trud");
            if trud_dir.exists() {
                args.input = trud_dir;
            }
        }
    }
    if args.output == Path::new(".") {
        if let Some(root) = crate::workspace::find_workspace_root() {
            args.output = root.join("current");
        }
    }

    // 1. Generate Parquet
    let parquet_out = args.output.clone();
    crate::commands::parquet::run(args)?;

    // 2. Write enriched Frictionless datapackage.json into release directory
    let prov = crate::provenance::OdsProvenance::load_from_dir(&parquet_out);
    let release_pkg = crate::datapackage::generate_release_datapackage(&parquet_out, prov.as_ref(), None, None);
    let pkg_json = serde_json::to_string_pretty(&release_pkg)?;
    std::fs::write(parquet_out.join("datapackage.json"), pkg_json)
        .context("writing datapackage.json to release directory")?;

    // 3. Update _provenance.json with tool_* and dataset_*
    crate::provenance::update_provenance(&parquet_out, None)?;

    crate::commands::parquet::warn_unexpected_files(&parquet_out);

    eprintln!("✓ Dataset target projections generated successfully.");
    Ok(())
}

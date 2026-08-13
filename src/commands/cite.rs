use anyhow::Result;
use clap::Parser;
use parquet::file::reader::{FileReader, SerializedFileReader};
use std::fs::File;
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
pub struct Args {
    /// Input directory containing the Parquet files (defaults to active workspace if omitted)
    #[arg(long, short)]
    pub input: Option<PathBuf>,
}

pub fn run(args: Args) -> Result<()> {
    let input_dir = crate::workspace::discover_parquet_dir(args.input.as_deref())?;
    let args_with_dir = Args {
        input: Some(input_dir),
    };
    run_with_writer(args_with_dir, &mut std::io::stdout())
}

pub fn run_with_writer(args: Args, writer: &mut dyn std::io::Write) -> Result<()> {
    let files = vec![
        "orgs.parquet",
        "orgs_all.parquet",
        "org_roles.parquet",
        "roles.parquet",
        "rels.parquet",
        "successors.parquet",
    ];

    let input_dir = match args.input {
        Some(ref dir) => dir.clone(),
        None => crate::workspace::discover_parquet_dir(None)?,
    };

    // 1. Fail early: check if at least one expected Parquet file exists
    let mut any_exist = false;
    for f in &files {
        if input_dir.join(f).exists() {
            any_exist = true;
            break;
        }
    }
    if !any_exist {
        anyhow::bail!(
            "No Parquet files found in the directory '{}'. Did you run `ods parquet` first?",
            input_dir.display()
        );
    }

    let prov = crate::provenance::OdsProvenance::load_from_dir(&input_dir);

    let mut pub_date = prov.as_ref().and_then(|p| p.trud_release_date.clone()).unwrap_or_else(|| "unknown".to_string());
    let mut release_name = prov.as_ref().and_then(|p| p.trud_release_name.clone()).unwrap_or_else(|| "Release".to_string());
    let mut release_file = prov.as_ref().and_then(|p| p.trud_release_file.clone()).unwrap_or_else(|| "hscorgrefdataxml".to_string());
    let mut archive_sha256 = prov.as_ref().and_then(|p| p.trud_release_sha256.clone()).unwrap_or_else(|| "<not verified>".to_string());
    let mut compiler_version = prov.as_ref().map(|p| p.ods_cmd_version.clone()).unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());

    // Fall back to key-value metadata in Parquet file headers if missing from provenance
    for f in &files {
        let path = input_dir.join(f);
        if path.exists() {
            if let Ok(file) = File::open(&path) {
                if let Ok(reader) = SerializedFileReader::new(file) {
                    let file_metadata = reader.metadata().file_metadata();
                    if let Some(kv) = file_metadata.key_value_metadata() {
                        for item in kv {
                            match item.key.as_str() {
                                "ods.trud_release_date" | "ods.publication_date" => {
                                    if pub_date == "unknown" {
                                        if let Some(ref val) = item.value {
                                            pub_date = val.clone();
                                        }
                                    }
                                }
                                "ods.trud_release_name" => {
                                    if let Some(ref val) = item.value {
                                        release_name = val.clone();
                                    }
                                }
                                "ods.trud_release_file" => {
                                    if let Some(ref val) = item.value {
                                        release_file = val.clone();
                                    }
                                }
                                "ods.trud_release_sha256" => {
                                    if archive_sha256 == "<not verified>" {
                                        if let Some(ref val) = item.value {
                                            archive_sha256 = val.clone();
                                        }
                                    }
                                }
                                "ods.cmd_version" | "ods.ods_cmd_version" | "ods.compiler_version" => {
                                    if let Some(ref val) = item.value {
                                        compiler_version = val.clone();
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            break;
        }
    }

    let year = pub_date.split('-').next().unwrap_or("2026");

    writeln!(writer, "Source")?;
    writeln!(
        writer,
        "  NHS Digital Organisation Data Service (ODS), published via NHS TRUD."
    )?;
    writeln!(writer, "  Release name:       {}", release_name)?;
    writeln!(writer, "  Release date:       {}", pub_date)?;
    writeln!(writer, "  Release file:       {}", release_file)?;
    writeln!(writer, "  Release SHA-256:    {}", archive_sha256)?;
    writeln!(writer)?;

    writeln!(writer, "Derived Sources")?;
    writeln!(
        writer,
        "  {:19} github.com/olizilla/ods v{}",
        "Created by:",
        compiler_version
    )?;
    writeln!(writer, "  Format:             Apache Parquet")?;
    for f in &files {
        let path = input_dir.join(f);
        if path.exists() {
            if let Ok(hash) = crate::provenance::compute_file_sha256(&path) {
                writeln!(writer, "  {:19} {}", format!("{}:", f), hash.to_ascii_uppercase())?;
            } else {
                writeln!(writer, "  {:19} <error computing hash>", format!("{}:", f))?;
            }
        } else {
            writeln!(writer, "  {:19} <not generated>", format!("{}:", f))?;
        }
    }
    writeln!(writer)?;

    writeln!(writer, "How to Cite")?;
    writeln!(writer, "  When citing the source data:")?;
    writeln!(
        writer,
        "    NHS Digital. ({}). NHS Organisation Data Service — {}\n    ({}). NHS TRUD. https://isd.digital.nhs.uk/trud",
        year, release_name, pub_date
    )?;
    writeln!(writer)?;
    writeln!(writer, "  When citing the derived sources:")?;
    writeln!(
        writer,
        "    Evans, O. ({}). ods: NHS Organisation Data in Open Formats\n    (v{}) [Software]. https://github.com/olizilla/ods",
        year, compiler_version
    )?;
    writeln!(writer)?;
    writeln!(
        writer,
        "  These Parquet files are deterministic projections of the official\n  TRUD ODS XML. You can verify this by running `ods trud audit` or\n  by rebuilding from source with `ods trud pull && ods make`."
    )?;

    Ok(())
}

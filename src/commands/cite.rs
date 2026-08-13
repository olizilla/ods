use anyhow::Result;
use clap::Parser;
use parquet::file::reader::{FileReader, SerializedFileReader};
use serde_json::json;
use std::fs::File;
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
pub struct Args {
    /// Input directory containing the Parquet files (defaults to active workspace if omitted)
    #[arg(long, short)]
    pub input: Option<PathBuf>,

    /// Citation output format: text, bibtex, csljson, apa
    #[arg(long, short, default_value = "text")]
    pub format: String,
}

pub fn run(args: Args) -> Result<()> {
    let input_dir = crate::workspace::discover_parquet_dir(args.input.as_deref())?;
    let args_with_dir = Args {
        input: Some(input_dir),
        format: args.format,
    };
    if let Err(e) = run_with_writer(args_with_dir, &mut std::io::stdout()) {
        if let Some(io_err) = e.downcast_ref::<std::io::Error>() {
            if io_err.kind() == std::io::ErrorKind::BrokenPipe {
                return Ok(());
            }
        }
        return Err(e);
    }
    Ok(())
}

pub fn run_with_writer(args: Args, writer: &mut dyn std::io::Write) -> Result<()> {
    let files = vec![
        "orgs.parquet",
        "orgs_all.parquet",
        "org_roles.parquet",
        "roles.parquet",
        "rels.parquet",
        "successors.parquet",
        "category_rules.json",
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

    let mut publication_date = prov.as_ref().and_then(|p| p.publication_date.clone())
        .unwrap_or_else(|| "unknown".to_string());
    let mut publication_seq_num = prov.as_ref().and_then(|p| p.publication_seq_num.clone())
        .unwrap_or_else(|| "unknown".to_string());
    let mut publication_type = prov.as_ref().and_then(|p| p.publication_type.clone())
        .unwrap_or_else(|| "Full".to_string());
    let mut _release_name = prov.as_ref().and_then(|p| p.trud_release_name.clone())
        .unwrap_or_else(|| "Release".to_string());
    let mut release_file = prov.as_ref().and_then(|p| p.trud_release_file.clone())
        .unwrap_or_else(|| "hscorgrefdataxml".to_string());
    let mut archive_sha256 = prov.as_ref().and_then(|p| p.trud_release_sha256.clone())
        .unwrap_or_else(|| "<not verified>".to_string());
    let mut tool_version = prov.as_ref().map(|p| p.tool_version.clone())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());

    // Fall back to key-value metadata in Parquet file headers if missing from provenance struct
    for f in &files {
        let path = input_dir.join(f);
        if path.exists() {
            if let Ok(file) = File::open(&path) {
                if let Ok(reader) = SerializedFileReader::new(file) {
                    let file_metadata = reader.metadata().file_metadata();
                    if let Some(kv) = file_metadata.key_value_metadata() {
                        for item in kv {
                            match item.key.as_str() {
                                "ods.publication_date" => {
                                    if publication_date == "unknown" {
                                        if let Some(ref val) = item.value {
                                            publication_date = val.clone();
                                        }
                                    }
                                }
                                "ods.publication_seq_num" => {
                                    if publication_seq_num == "unknown" {
                                        if let Some(ref val) = item.value {
                                            publication_seq_num = val.clone();
                                        }
                                    }
                                }
                                "ods.publication_type" => {
                                    if let Some(ref val) = item.value {
                                        publication_type = val.clone();
                                    }
                                }
                                "ods.trud_release_name" => {
                                    if let Some(ref val) = item.value {
                                        _release_name = val.clone();
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
                                "ods.tool_version" => {
                                    if let Some(ref val) = item.value {
                                        tool_version = val.clone();
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

    let year = publication_date.split('-').next().unwrap_or("2026");
    let month = publication_date.split('-').nth(1).unwrap_or("07");
    let day = publication_date.split('-').nth(2).unwrap_or("28");

    let orgs_parquet_hash = if input_dir.join("orgs.parquet").exists() {
        crate::provenance::compute_file_sha256(&input_dir.join("orgs.parquet")).unwrap_or_else(|_| "unknown".to_string())
    } else {
        "unknown".to_string()
    };

    match args.format.to_lowercase().as_str() {
        "bibtex" => {
            let key = format!("nhs_england_ods_{}_{}", publication_date.replace('-', "_"), publication_seq_num);
            writeln!(writer, "@misc{{{key},")?;
            writeln!(writer, "  author = {{NHS England}},")?;
            writeln!(writer, "  title = {{Organisation Data Service: {} publication ({}, Seq {})}},", publication_type, publication_date, publication_seq_num)?;
            writeln!(writer, "  year = {{{year}}},")?;
            writeln!(writer, "  month = {{{month}}},")?;
            writeln!(writer, "  howpublished = {{NHS TRUD}},")?;
            writeln!(writer, "  url = {{https://isd.digital.nhs.uk/trud}},")?;
            writeln!(writer, "  note = {{orgs.parquet SHA-256: {}; compiled by ods v{}}}", orgs_parquet_hash.to_ascii_uppercase(), tool_version)?;
            writeln!(writer, "}}")?;
        }
        "csljson" | "csl-json" | "json" => {
            let y: i32 = year.parse().unwrap_or(2026);
            let m: i32 = month.parse().unwrap_or(7);
            let d: i32 = day.parse().unwrap_or(28);

            let csl = json!([
                {
                    "type": "dataset",
                    "id": format!("nhs-ods-{}-{}", publication_date, publication_seq_num),
                    "title": format!("Organisation Data Service: {} publication ({}, Seq {})", publication_type, publication_date, publication_seq_num),
                    "author": [
                        { "literal": "NHS England" }
                    ],
                    "issued": {
                        "date-parts": [[y, m, d]]
                    },
                    "publisher": "NHS TRUD",
                    "URL": "https://isd.digital.nhs.uk/trud",
                    "note": format!("orgs.parquet SHA-256: {}; compiled by ods v{}", orgs_parquet_hash.to_ascii_uppercase(), tool_version)
                }
            ]);
            writeln!(writer, "{}", serde_json::to_string_pretty(&csl)?)?;
        }
        "apa" => {
            writeln!(
                writer,
                "NHS England. ({}) Organisation Data Service: {} publication ({}, Seq {}) [Data set]. NHS TRUD. https://isd.digital.nhs.uk/trud",
                year, publication_type, publication_date, publication_seq_num
            )?;
        }
        _ => {
            // Default text format
            writeln!(writer, "Source")?;
            writeln!(
                writer,
                "  NHS England Organisation Data Service (ODS), published via NHS TRUD."
            )?;
            writeln!(writer, "  Publication date:   {}", publication_date)?;
            writeln!(writer, "  Publication seq:    {}", publication_seq_num)?;
            writeln!(writer, "  Publication type:   {}", publication_type)?;
            writeln!(writer, "  Release file:       {}", release_file)?;
            writeln!(writer, "  Release SHA-256:    {}", archive_sha256)?;
            writeln!(writer)?;

            writeln!(writer, "Derived Sources")?;
            writeln!(
                writer,
                "  {:19} github.com/olizilla/ods v{}",
                "Created by:",
                tool_version
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
                "    NHS England. ({}). Organisation Data Service: {} publication\n    ({}, Seq {}). NHS TRUD. https://isd.digital.nhs.uk/trud",
                year, publication_type, publication_date, publication_seq_num
            )?;
            writeln!(writer)?;
            writeln!(writer, "  When citing the derived sources:")?;
            writeln!(
                writer,
                "    Evans, O. ({}). ods: NHS Organisation Data in Open Formats\n    (v{}) [Software]. https://github.com/olizilla/ods",
                year, tool_version
            )?;
            writeln!(writer)?;
            writeln!(
                writer,
                "  These Parquet files are deterministic projections of the official\n  TRUD ODS XML. You can verify this by running `ods trud audit` or\n  by rebuilding from source with `ods trud pull && ods make`."
            )?;
        }
    }

    Ok(())
}

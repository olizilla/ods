use anyhow::{Context, Result};
use clap::Parser;
use parquet::file::reader::{FileReader, SerializedFileReader};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
pub struct Args {
    /// Input directory containing the Parquet files
    #[arg(long, short, default_value = ".")]
    pub input: PathBuf,
}

pub fn run(args: Args) -> Result<()> {
    run_with_writer(args, &mut std::io::stdout())
}

pub fn run_with_writer(args: Args, writer: &mut dyn std::io::Write) -> Result<()> {
    let files = vec![
        "orgs.parquet",
        "orgs_all.parquet",
        "roles.parquet",
        "rels.parquet",
        "successors.parquet",
    ];

    // 1. Fail early: check if at least one expected Parquet file exists
    let mut any_exist = false;
    for f in &files {
        if args.input.join(f).exists() {
            any_exist = true;
            break;
        }
    }
    if !any_exist {
        anyhow::bail!(
            "No Parquet files found in the directory '{}'. Did you run `ods parquet` first?",
            args.input.display()
        );
    }

    let mut edition = "TRUD ODS Full".to_string();
    let mut pub_date = "unknown".to_string();
    let mut pub_seq = "unknown".to_string();
    let mut pub_source = "HSCIC".to_string();
    let mut xml_version = "2-0-0".to_string();
    let mut compiler_version = "unknown".to_string();
    let mut created_at = "unknown".to_string();

    // 2. Find the first Parquet file in the directory to extract key-value metadata from
    for f in &files {
        let path = args.input.join(f);
        if path.exists() {
            if let Ok(file) = File::open(&path) {
                if let Ok(reader) = SerializedFileReader::new(file) {
                    let file_metadata = reader.metadata().file_metadata();
                    if let Some(kv) = file_metadata.key_value_metadata() {
                        for item in kv {
                            match item.key.as_str() {
                                "ods.publication_date" => {
                                    if let Some(ref val) = item.value {
                                        pub_date = val.clone();
                                    }
                                }
                                "ods.publication_seq_num" => {
                                    if let Some(ref val) = item.value {
                                        pub_seq = val.clone();
                                    }
                                }
                                "ods.publication_source" => {
                                    if let Some(ref val) = item.value {
                                        pub_source = val.clone();
                                    }
                                }
                                "ods.publication_type" => {
                                    if let Some(ref val) = item.value {
                                        edition = format!("TRUD ODS {}", val);
                                    }
                                }
                                "ods.edition_label" => {
                                    if let Some(ref val) = item.value {
                                        edition = val.clone();
                                    }
                                }
                                "ods.xml_version" => {
                                    if let Some(ref val) = item.value {
                                        xml_version = val.clone();
                                    }
                                }
                                "ods.compiler_version" => {
                                    if let Some(ref val) = item.value {
                                        compiler_version = val.clone();
                                    }
                                }
                                "ods.created_at" => {
                                    if let Some(ref val) = item.value {
                                        created_at = val.clone();
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

    let edition_label = if pub_date != "unknown" && !edition.contains(&pub_date) {
        format!("{} {}", edition, pub_date)
    } else {
        edition
    };

    // 3. Output academic citation block
    writeln!(writer, "=== Academic Citation & Verification Block ===")?;
    writeln!(writer, "NHS ODS Data Tool (ods) Version: {}", compiler_version)?;
    writeln!(writer, "Edition Label:                 {}", edition_label)?;
    writeln!(writer, "Publication Date:              {}", pub_date)?;
    writeln!(writer, "Publication Sequence Num:      {}", pub_seq)?;
    writeln!(writer, "Publication Source:            {}", pub_source)?;
    writeln!(writer, "XML Schema Version:            {}", xml_version)?;
    writeln!(writer, "Created At (UTC):              {}", created_at)?;
    writeln!(writer, "\nRecommended APA Citation:")?;
    
    let year = pub_date.split('-').next().unwrap_or("2026");
    writeln!(
        writer,
        "{}. ({}). Digital Organisation Reference Data (Publication Date: {}, Seq #{}, XML v{}). Retrieved from NHS TRUD ODS Data Service.",
        pub_source,
        year,
        pub_date,
        pub_seq,
        xml_version
    )?;
    writeln!(writer, "\nSHA256 Signatures for Provenance Verification:")?;

    // 4. Compute SHA256 hashes of all generated files
    for f in &files {
        let path = args.input.join(f);
        if path.exists() {
            let mut file = File::open(&path).with_context(|| format!("reading {}", path.display()))?;
            let mut hasher = Sha256::new();
            std::io::copy(&mut file, &mut hasher)?;
            let hash = format!("{:x}", hasher.finalize());
            writeln!(writer, "  {}: {}", f, hash)?;
        } else {
            writeln!(writer, "  {}: <not generated>", f)?;
        }
    }

    Ok(())
}

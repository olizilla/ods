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
        "roles.parquet",
        "relationships.parquet",
        "successions.parquet",
        "datapackage.json",
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
            "✖ No Parquet files found in the directory '{}'. Did you run `ods parquet` first?",
            input_dir.display()
        );
    }

    let prov = crate::provenance::OdsProvenance::load_from_dir(&input_dir);

    // Decline citation if source archive is unverified
    let verification_status = prov.as_ref().and_then(|p| p.trud_release_sha256_verified);
    match verification_status {
        Some(crate::provenance::TrudVerificationSource::TrudApi)
        | Some(crate::provenance::TrudVerificationSource::PublishedRelease) => {}
        _ => {
            anyhow::bail!(
                "✖ Cannot generate citation for unverified release\n  The source TRUD archive has not been verified against an upstream TRUD API checksum."
            );
        }
    }

    let mut publication_date = prov.as_ref().and_then(|p| p.publication_date.clone())
        .unwrap_or_else(|| "unknown".to_string());
    let mut publication_seq_num = prov.as_ref().and_then(|p| p.publication_seq_num.clone())
        .unwrap_or_else(|| "unknown".to_string());
    let mut publication_type = prov.as_ref().and_then(|p| p.publication_type.clone())
        .unwrap_or_else(|| "unknown".to_string());
    let mut release_file = prov.as_ref().and_then(|p| p.trud_release_file.clone())
        .unwrap_or_else(|| "hscorgrefdataxml".to_string());
    let mut archive_sha256 = prov.as_ref().and_then(|p| p.trud_release_sha256.clone())
        .unwrap_or_else(|| "<not verified>".to_string());
    let mut tool_version = prov.as_ref().and_then(|p| p.tool_version.clone())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
    let mut dataset_version = prov.as_ref().and_then(|p| p.dataset_version.clone());
    let mut manifest_digest: Option<String> = None;
    let mut withdrawn: Option<String> = None;
    let mut dataset_doi: Option<String> = None;

    let release_json_path = input_dir.join("_release.json");
    if release_json_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&release_json_path) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(doi) = val.get("dataset_doi").and_then(|d| d.as_str()) {
                    dataset_doi = Some(doi.to_string());
                }
                if let Some(ver) = val.get("dataset_version").and_then(|v| v.as_str()) {
                    if dataset_version.is_none() {
                        dataset_version = Some(ver.to_string());
                    }
                }
                if let Some(digest) = val.get("manifest_digest").and_then(|d| d.as_str()) {
                    manifest_digest = Some(digest.to_string());
                }
                if let Some(w) = val.get("withdrawn").and_then(|w| w.as_str()) {
                    withdrawn = Some(w.to_string());
                }
            }
        }
    }

    let trud_date = prov.as_ref().and_then(|p| p.trud_release_date.clone());
    if withdrawn.is_none() {
        if let (Some(ref d), Some(ref v)) = (&trud_date, &dataset_version) {
            if let Ok(index) = crate::index::OdsReleaseIndex::baked() {
                if let Some(entry) = index.releases.iter().find(|r| &r.trud_release_date == d && &r.dataset_version == v) {
                    if let Some(ref w) = entry.withdrawn {
                        withdrawn = Some(w.clone());
                    }
                }
            }
        }
    }

    if let Some(ref reason) = withdrawn {
        let d_str = trud_date.as_deref().unwrap_or(&publication_date);
        let v_str = dataset_version.as_deref().unwrap_or("unknown");
        anyhow::bail!(
            "✖ Refusing to cite {} v{}\n  This release was withdrawn: {}\n  Update to a valid release: ods pull {}",
            d_str, v_str, reason, d_str
        );
    }

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
                                    if publication_type == "unknown" {
                                        if let Some(ref val) = item.value {
                                            publication_type = val.clone();
                                        }
                                    }
                                }
                                "ods.trud_release_file" => {
                                    if let Some(ref val) = item.value {
                                        release_file = val.clone();
                                    }
                                }
                                "ods.trud_release_sha256" => {
                                    if let Some(ref val) = item.value {
                                        archive_sha256 = val.clone();
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
        }
    }

    let year = if publication_date.len() >= 4 {
        &publication_date[0..4]
    } else {
        "unknown"
    };

    let orgs_parquet_hash = crate::provenance::compute_file_sha256(&input_dir.join("orgs.parquet"))
        .unwrap_or_else(|_| "<hash error>".to_string());

    let type_title_part = if publication_type == "unknown" {
        "".to_string()
    } else {
        format!(": {} publication", publication_type)
    };

    let d_tag = trud_date.as_deref().unwrap_or(&publication_date);

    match args.format.to_lowercase().as_str() {
        "bibtex" => {
            let cite_key = if let Some(ref ver) = dataset_version {
                format!("ods-{}-v{}", d_tag, ver)
            } else {
                format!(
                    "nhs_england_ods_{}_{}",
                    publication_date.replace('-', "_"),
                    publication_seq_num
                )
            };
            writeln!(writer, "@misc{{{},", cite_key)?;
            writeln!(writer, "  author = {{NHS England}},")?;
            let title_str = if let Some(ref ver) = dataset_version {
                format!("NHS Organisation Data Service ({} cut, v{})", d_tag, ver)
            } else {
                format!(
                    "Organisation Data Service{} ({}, Seq {})",
                    type_title_part, publication_date, publication_seq_num
                )
            };
            writeln!(writer, "  title = {{{}}},", title_str)?;
            writeln!(writer, "  year = {{{}}},", year)?;
            if publication_date.len() >= 7 {
                writeln!(writer, "  month = {{{}}},", &publication_date[5..7])?;
            }
            if let Some(ref ver) = dataset_version {
                writeln!(writer, "  version = {{{}}},", ver)?;
            }
            if let Some(ref doi) = dataset_doi {
                writeln!(writer, "  doi = {{{}}},", doi)?;
            }
            writeln!(writer, "  howpublished = {{NHS TRUD}},")?;
            writeln!(
                writer,
                "  url = {{https://isd.digital.nhs.uk/trud}},"
            )?;
            let note_str = if let Some(ref md) = manifest_digest {
                format!("Manifest: {}", md)
            } else {
                format!("orgs.parquet SHA-256: {}; compiled by ods v{}", orgs_parquet_hash.to_ascii_uppercase(), tool_version)
            };
            writeln!(
                writer,
                "  note = {{{}}}",
                note_str
            )?;
            writeln!(writer, "}}")?;
        }
        "csljson" | "csl-json" | "json" => {
            let (y, m, d) = if publication_date.len() == 10 {
                let parts: Vec<&str> = publication_date.split('-').collect();
                (
                    parts[0].parse().unwrap_or(2026),
                    parts[1].parse().unwrap_or(7),
                    parts[2].parse().unwrap_or(28),
                )
            } else {
                (2026, 7, 28)
            };

            let title_str = if let Some(ref ver) = dataset_version {
                format!("Organisation Data Service ({} cut, v{})", d_tag, ver)
            } else {
                format!("Organisation Data Service{} ({}, Seq {})", type_title_part, publication_date, publication_seq_num)
            };

            let note_str = if let Some(ref md) = manifest_digest {
                format!("Manifest: {}", md)
            } else {
                format!("orgs.parquet SHA-256: {}; compiled by ods v{}", orgs_parquet_hash.to_ascii_uppercase(), tool_version)
            };

            let mut item_obj = json!({
                "type": "dataset",
                "id": format!("nhs-ods-{}-{}", publication_date, publication_seq_num),
                "title": title_str,
                "author": [
                    { "literal": "NHS England" }
                ],
                "issued": {
                    "date-parts": [[y, m, d]]
                },
                "publisher": "NHS TRUD",
                "URL": "https://isd.digital.nhs.uk/trud",
                "note": note_str
            });

            if let Some(ref ver) = dataset_version {
                item_obj["version"] = json!(ver);
            }

            if let Some(ref doi) = dataset_doi {
                item_obj["DOI"] = json!(doi);
            }

            let csl = json!([item_obj]);
            writeln!(writer, "{}", serde_json::to_string_pretty(&csl)?)?;
        }
        "apa" => {
            let title_str = if let Some(ref ver) = dataset_version {
                format!("Organisation Data Service ({} cut, v{})", d_tag, ver)
            } else {
                format!("Organisation Data Service{} ({}, Seq {})", type_title_part, publication_date, publication_seq_num)
            };
            if let Some(ref doi) = dataset_doi {
                let doi_url = if doi.starts_with("http") { doi.clone() } else { format!("https://doi.org/{}", doi) };
                writeln!(
                    writer,
                    "NHS England. ({}) {} [Data set]. NHS TRUD. {}",
                    year, title_str, doi_url
                )?;
            } else {
                writeln!(
                    writer,
                    "NHS England. ({}) {} [Data set]. NHS TRUD. https://isd.digital.nhs.uk/trud",
                    year, title_str
                )?;
            }
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
            if let Some(ref ver) = dataset_version {
                writeln!(writer, "  Dataset version:    v{}", ver)?;
            }
            if let Some(ref md) = manifest_digest {
                writeln!(writer, "  Manifest digest:    {}", md)?;
            }
            if let Some(ref doi) = dataset_doi {
                writeln!(writer, "  Dataset DOI:        {}", doi)?;
            }
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
            if let Some(ref ver) = dataset_version {
                writeln!(
                    writer,
                    "    NHS England. ({}). NHS ODS Dataset ({} cut, v{}).\n    NHS TRUD. https://isd.digital.nhs.uk/trud",
                    year, d_tag, ver
                )?;
            } else {
                writeln!(
                    writer,
                    "    NHS England. ({}). Organisation Data Service{}\n    ({}, Seq {}). NHS TRUD. https://isd.digital.nhs.uk/trud",
                    year, type_title_part, publication_date, publication_seq_num
                )?;
            }
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

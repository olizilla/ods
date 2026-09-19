use anyhow::{Context, Result};
use clap::Parser;
use parquet::file::reader::{FileReader, SerializedFileReader};
use serde_json::json;
use std::fs::File;
use std::path::{Path, PathBuf};

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
    let input_dir = crate::workspace::resolve_parquet_input(args.input.as_deref())?;
    let args_with_dir = Args {
        input: Some(input_dir),
        format: args.format,
    };
    run_with_writer(args_with_dir, &mut std::io::stdout())
}

pub fn run_with_writer(args: Args, writer: &mut dyn std::io::Write) -> Result<()> {
    let pwd = std::env::current_dir()?;
    run_with_writer_and_fetcher(args, writer, &crate::commands::pull::HttpOciFetcher, &pwd)
}

pub fn resolve_cite_index(
    workspace_root: Option<&Path>,
) -> Result<crate::index::OdsReleaseIndex> {
    let baked = crate::index::OdsReleaseIndex::baked().unwrap_or_default();

    // 1. Try loading cached index from workspace
    if let Some(ws) = workspace_root {
        if let Ok(Some(loaded)) = crate::index::OdsReleaseIndex::load_from_workspace(ws) {
            match baked.merge(&loaded) {
                Ok(merged) => return Ok(merged),
                Err(e) => {
                    if e.downcast_ref::<crate::index::SecurityError>().is_some() || e.to_string().contains("Security error") {
                        return Err(e);
                    }
                }
            }
        }
    }

    // 2. Fall back to baked index
    Ok(baked)
}

pub fn run_with_writer_and_fetcher<F: crate::commands::pull::OciBlobFetcher>(
    args: Args,
    writer: &mut dyn std::io::Write,
    _fetcher: &F,
    cwd: &Path,
) -> Result<()> {
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
        None => crate::workspace::resolve_parquet_input_from(cwd, None)?,
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

    let prov_unwrapped = prov.as_ref().cloned().unwrap_or_default();
    let dataset_version = prov_unwrapped
        .dataset_version
        .clone()
        .ok_or_else(|| anyhow::anyhow!("✖ Refusing to cite corrupted release in {}: _provenance.json missing dataset_version", input_dir.display()))?;

    let mut publication_date = prov_unwrapped.publication_date.clone()
        .unwrap_or_else(|| "unknown".to_string());
    let mut publication_seq_num = prov_unwrapped.publication_seq_num.clone()
        .unwrap_or_else(|| "unknown".to_string());
    let mut publication_type = prov_unwrapped.publication_type.clone()
        .unwrap_or_else(|| "unknown".to_string());
    let mut release_file = prov_unwrapped.trud_release_file.clone()
        .unwrap_or_else(|| "hscorgrefdataxml".to_string());
    let mut archive_sha256 = prov_unwrapped.trud_release_sha256.clone()
        .unwrap_or_else(|| "<not verified>".to_string());
    let mut tool_version = prov_unwrapped.tool_version.clone()
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
    let trud_date = prov_unwrapped.trud_release_date.clone();

    // 1. Reconstruct manifest in memory
    let (manifest, _) = crate::commands::make_oci::build_manifest_from_dir(&input_dir, &prov_unwrapped, &dataset_version)
        .context("reconstructing manifest in memory for citation")?;
    let manifest_digest = manifest.digest()?;

    // 2. Discover workspace and load/cache index
    let workspace_root = match crate::workspace::find_workspace_root_from(&input_dir, None)? {
        Some(ws) => Some(ws),
        None => crate::workspace::find_workspace_root_from(cwd, None)?,
    };

    let index = resolve_cite_index(workspace_root.as_deref())?;

    // 3. Verify directory against index
    let outcome = crate::workspace::verify_release_dir(&input_dir, Some(&index));
    match outcome {
        crate::workspace::VerificationOutcome::Mismatch { expected_digest, reconstructed_digest, .. } => {
            anyhow::bail!(
                "✖ Refusing to cite corrupted release in {}\n  Reconstructed manifest {} != expected {}",
                input_dir.display(),
                reconstructed_digest,
                expected_digest
            );
        }
        crate::workspace::VerificationOutcome::Corrupted(err) => {
            anyhow::bail!("✖ Refusing to cite corrupted release in {}: {}", input_dir.display(), err);
        }
        _ => {}
    }

    // 4. Check for withdrawal in index
    let mut dataset_doi: Option<String> = None;
    let d_ref = trud_date.as_deref().unwrap_or(&publication_date);
    let dataset = index
        .releases
        .iter()
        .find(|r| r.trud_release_date == d_ref)
        .and_then(|r| r.datasets.iter().find(|d| d.dataset_version == dataset_version));

    if let Some(entry) = dataset {
        if let Some(ref reason) = entry.withdrawn {
            anyhow::bail!(
                "✖ Refusing to cite {} v{}\n  This release was withdrawn: {}\n  Update to a valid release: ods pull {}",
                d_ref, dataset_version, reason, d_ref
            );
        }
        dataset_doi = entry.dataset_doi.clone();
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

    let d_tag = trud_date.as_deref().unwrap_or(&publication_date);

    let (d_y, d_m, d_d) = if d_tag.len() == 10 {
        let parts: Vec<&str> = d_tag.split('-').collect();
        (
            parts[0].parse().unwrap_or(2026),
            parts[1].parse().unwrap_or(8),
            parts[2].parse().unwrap_or(28),
        )
    } else {
        (2026, 8, 28)
    };

    let (src_y, src_m, src_d) = if publication_date.len() == 10 {
        let parts: Vec<&str> = publication_date.split('-').collect();
        (
            parts[0].parse().unwrap_or(2026),
            parts[1].parse().unwrap_or(8),
            parts[2].parse().unwrap_or(27),
        )
    } else {
        (2026, 8, 27)
    };

    let d_month = if d_tag.len() >= 7 {
        &d_tag[5..7]
    } else {
        "08"
    };

    let src_month = if publication_date.len() >= 7 {
        &publication_date[5..7]
    } else {
        "08"
    };

    match args.format.to_lowercase().as_str() {
        "bibtex" => {
            // 1. Data release
            let data_key = format!("ods-{}-v{}", d_tag, dataset_version);
            writeln!(writer, "@misc{{{},", data_key)?;
            writeln!(writer, "  author = {{Evans, Oli}},")?;
            writeln!(
                writer,
                "  title = {{ods: NHS Organisation Data as verifiable Parquet files, release {}}},",
                d_tag
            )?;
            writeln!(writer, "  year = {{{}}},", year)?;
            writeln!(writer, "  month = {{{}}},", d_month)?;
            writeln!(writer, "  version = {{{}}},", dataset_version)?;
            writeln!(writer, "  howpublished = {{ods.fyi}},")?;
            writeln!(writer, "  url = {{https://ods.fyi}},")?;
            if let Some(ref doi) = dataset_doi {
                writeln!(writer, "  doi = {{{}}},", doi)?;
            }
            writeln!(
                writer,
                "  note = {{Manifest: {}. Contains information from NHS England, licensed under the current version of the Open Government Licence.}}",
                manifest_digest
            )?;
            writeln!(writer, "}}\n")?;

            // 2. Tool
            let tool_key = format!("ods-software-v{}", dataset_version);
            writeln!(writer, "@misc{{{},", tool_key)?;
            writeln!(writer, "  author = {{Evans, Oli}},")?;
            writeln!(writer, "  title = {{ods}},")?;
            writeln!(writer, "  year = {{{}}},", year)?;
            writeln!(writer, "  version = {{{}}},", dataset_version)?;
            writeln!(writer, "  howpublished = {{Computer software}},")?;
            writeln!(writer, "  url = {{https://github.com/olizilla/ods}}")?;
            writeln!(writer, "}}\n")?;

            // 3. Upstream source
            let src_key = format!("nhs-ods-{}-{}", publication_date, publication_seq_num);
            writeln!(writer, "@misc{{{},", src_key)?;
            writeln!(writer, "  author = {{NHS England}},")?;
            writeln!(
                writer,
                "  title = {{NHS Organisation Data Service XML Data, release {}}},",
                d_tag
            )?;
            writeln!(writer, "  year = {{{}}},", year)?;
            writeln!(writer, "  month = {{{}}},", src_month)?;
            writeln!(writer, "  howpublished = {{NHS TRUD}},")?;
            writeln!(writer, "  url = {{https://isd.digital.nhs.uk/trud}},")?;
            writeln!(
                writer,
                "  note = {{Publication {}, published {}. Release file {}, SHA-256 {}.}}",
                publication_seq_num, publication_date, release_file, archive_sha256
            )?;
            writeln!(writer, "}}")?;
        }
        "csljson" | "csl-json" | "json" => {
            let mut data_obj = json!({
                "type": "dataset",
                "id": format!("ods-{}-v{}", d_tag, dataset_version),
                "title": format!("ods: NHS Organisation Data as verifiable Parquet files, release {}", d_tag),
                "author": [{ "family": "Evans", "given": "Oli" }],
                "issued": { "date-parts": [[d_y, d_m, d_d]] },
                "publisher": "ods.fyi",
                "URL": "https://ods.fyi",
                "version": dataset_version,
                "note": format!("Manifest: {}. Contains information from NHS England, licensed under the current version of the Open Government Licence.", manifest_digest)
            });
            if let Some(ref doi) = dataset_doi {
                data_obj["DOI"] = json!(doi);
            }

            let tool_obj = json!({
                "type": "software",
                "id": format!("ods-software-v{}", dataset_version),
                "title": "ods",
                "author": [{ "family": "Evans", "given": "Oli" }],
                "issued": { "date-parts": [[d_y]] },
                "URL": "https://github.com/olizilla/ods",
                "version": dataset_version
            });

            let src_obj = json!({
                "type": "dataset",
                "id": format!("nhs-ods-{}-{}", publication_date, publication_seq_num),
                "title": format!("NHS Organisation Data Service XML Data, release {}", d_tag),
                "author": [{ "literal": "NHS England" }],
                "issued": { "date-parts": [[src_y, src_m, src_d]] },
                "publisher": "NHS TRUD",
                "URL": "https://isd.digital.nhs.uk/trud",
                "note": format!("Publication {}, published {}. Release file {}, SHA-256 {}.", publication_seq_num, publication_date, release_file, archive_sha256)
            });

            let csl = json!([data_obj, tool_obj, src_obj]);
            writeln!(writer, "{}", serde_json::to_string_pretty(&csl)?)?;
        }
        "apa" => {
            let data_url = if let Some(ref doi) = dataset_doi {
                if doi.starts_with("http") {
                    doi.clone()
                } else {
                    format!("https://doi.org/{}", doi)
                }
            } else {
                "https://ods.fyi".to_string()
            };
            writeln!(
                writer,
                "Evans, O. ({}). ods: NHS Organisation Data as verifiable Parquet files, release {} (Version {}) [Data set]. ods.fyi. {}",
                year, d_tag, dataset_version, data_url
            )?;
            writeln!(
                writer,
                "Evans, O. ({}). ods (Version {}) [Computer software]. https://github.com/olizilla/ods",
                year, dataset_version
            )?;
            writeln!(
                writer,
                "NHS England. ({}). NHS Organisation Data Service XML Data, release {} [Data set]. NHS TRUD. https://isd.digital.nhs.uk/trud",
                year, d_tag
            )?;
        }
        _ => {
            // Source header on stdout
            let header_lines = crate::workspace::format_cite_source_header(&input_dir, d_tag, &dataset_version, Some(cwd));
            for line in header_lines {
                writeln!(writer, "{}", line)?;
            }
            writeln!(writer)?;

            let data_url = if let Some(ref doi) = dataset_doi {
                if doi.starts_with("http") {
                    doi.clone()
                } else {
                    format!("https://doi.org/{}", doi)
                }
            } else {
                "https://ods.fyi".to_string()
            };

            // How to Cite
            writeln!(writer, "How to Cite")?;
            writeln!(writer, "  The data:")?;
            writeln!(
                writer,
                "    Evans, O. ({}). ods: NHS Organisation Data as verifiable Parquet files,\n    release {} (Version {}) [Data set]. ods.fyi. {}",
                year, d_tag, dataset_version, data_url
            )?;
            writeln!(writer)?;
            writeln!(writer, "  The tool:")?;
            writeln!(
                writer,
                "    Evans, O. ({}). ods (Version {}) [Computer software].\n    https://github.com/olizilla/ods",
                year, dataset_version
            )?;
            writeln!(writer)?;
            writeln!(writer, "  The source:")?;
            writeln!(
                writer,
                "    NHS England. ({}). NHS Organisation Data Service XML Data, release {}\n    [Data set]. NHS TRUD. https://isd.digital.nhs.uk/trud\n    Contains information from NHS England, licensed under the current version of the\n    Open Government Licence.",
                year, d_tag
            )?;
            writeln!(writer)?;

            // Data
            writeln!(writer, "Data")?;
            writeln!(writer, "  Dataset version:    v{}", dataset_version)?;
            writeln!(writer, "  Manifest digest:    {}", manifest_digest)?;
            if let Some(ref doi) = dataset_doi {
                writeln!(writer, "  Dataset DOI:        {}", doi)?;
            }
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

            // Source
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
            writeln!(
                writer,
                "  These Parquet files are deterministic projections of the official\n  TRUD ODS XML. You can verify this by running `ods trud audit` or\n  by rebuilding from source with `ods trud pull && ods make`."
            )?;
        }
    }

    let is_human = args.format == "text";
    crate::workspace::check_and_emit_staleness_nudge(&input_dir, is_human);

    Ok(())
}

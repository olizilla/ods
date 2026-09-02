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
    let fetcher = crate::commands::pull::HttpOciFetcher;
    run_with_writer_and_fetcher(args, writer, &fetcher)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexFetchStatus {
    JustNow,
    Cached(String),
    Baked,
}

pub fn resolve_cite_index<F: crate::commands::pull::OciBlobFetcher>(
    workspace_root: Option<&Path>,
    fetcher: &F,
) -> Result<(crate::index::OdsReleaseIndex, IndexFetchStatus)> {
    let baked = crate::index::OdsReleaseIndex::baked().unwrap_or_default();

    // 1. Try fetching remote index online
    match fetcher.fetch_release_index() {
        Ok(Some(fetched)) => {
            match baked.merge(&fetched) {
                Ok(merged) => {
                    let now_rfc3339 = chrono::Utc::now().to_rfc3339();
                    if let Some(ws) = workspace_root {
                        let cached = crate::index::CachedReleaseIndex {
                            fetched_at: now_rfc3339,
                            index: merged.clone(),
                        };
                        let _ = cached.save_to_workspace(ws);
                    }
                    return Ok((merged, IndexFetchStatus::JustNow));
                }
                Err(e) => {
                    if e.downcast_ref::<crate::index::SecurityError>().is_some() || e.to_string().contains("Security error") {
                        return Err(e);
                    }
                }
            }
        }
        Ok(None) => {}
        Err(e) => {
            if e.downcast_ref::<crate::index::SecurityError>().is_some() || e.to_string().contains("Security error") {
                return Err(e);
            }
        }
    }

    // 2. Try loading cached index from workspace
    if let Some(ws) = workspace_root {
        if let Ok(Some(cached)) = crate::index::CachedReleaseIndex::load_from_workspace(ws) {
            match baked.merge(&cached.index) {
                Ok(merged) => return Ok((merged, IndexFetchStatus::Cached(cached.fetched_at))),
                Err(e) => {
                    if e.downcast_ref::<crate::index::SecurityError>().is_some() || e.to_string().contains("Security error") {
                        return Err(e);
                    }
                }
            }
        }
    }

    // 3. Fall back to baked index
    Ok((baked, IndexFetchStatus::Baked))
}

pub fn run_with_writer_and_fetcher<F: crate::commands::pull::OciBlobFetcher>(
    args: Args,
    writer: &mut dyn std::io::Write,
    fetcher: &F,
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
        None => crate::workspace::resolve_parquet_input(None)?,
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
    let workspace_root = input_dir
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .or_else(|| crate::workspace::Workspace::open(None).ok().map(|ws| ws.root().to_path_buf()));

    let (index, index_status) = resolve_cite_index(workspace_root.as_deref(), fetcher)?;

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
    if let Some(entry) = index.releases.iter().find(|r| r.trud_release_date == d_ref && r.dataset_version == dataset_version) {
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

    match args.format.to_lowercase().as_str() {
        "bibtex" => {
            let cite_key = format!("ods-{}-v{}", d_tag, dataset_version);
            writeln!(writer, "@misc{{{},", cite_key)?;
            writeln!(writer, "  author = {{NHS England}},")?;
            let title_str = format!("NHS Organisation Data Service ({} cut, v{})", d_tag, dataset_version);
            writeln!(writer, "  title = {{{}}},", title_str)?;
            writeln!(writer, "  year = {{{}}},", year)?;
            if publication_date.len() >= 7 {
                writeln!(writer, "  month = {{{}}},", &publication_date[5..7])?;
            }
            writeln!(writer, "  version = {{{}}},", dataset_version)?;
            if let Some(ref doi) = dataset_doi {
                writeln!(writer, "  doi = {{{}}},", doi)?;
            }
            writeln!(writer, "  howpublished = {{NHS TRUD}},")?;
            writeln!(
                writer,
                "  url = {{https://isd.digital.nhs.uk/trud}},"
            )?;
            let note_str = format!("Manifest: {}", manifest_digest);
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

            let title_str = format!("Organisation Data Service ({} cut, v{})", d_tag, dataset_version);
            let note_str = format!("Manifest: {}", manifest_digest);

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
                "note": note_str,
                "version": dataset_version
            });

            if let Some(ref doi) = dataset_doi {
                item_obj["DOI"] = json!(doi);
            }

            let csl = json!([item_obj]);
            writeln!(writer, "{}", serde_json::to_string_pretty(&csl)?)?;
        }
        "apa" => {
            let title_str = format!("Organisation Data Service ({} cut, v{})", d_tag, dataset_version);
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
            // Header line per Task 4
            match index_status {
                IndexFetchStatus::JustNow => {
                    writeln!(writer, "✓ {} ({}) — checked against the index just now\n", d_tag, dataset_version)?;
                }
                IndexFetchStatus::Cached(ref fetched_at) => {
                    let fetched_day = fetched_at.split('T').next().unwrap_or(fetched_at);
                    writeln!(writer, "✓ {} ({})\n  index last fetched {}; a withdrawal published since would not show here\n", d_tag, dataset_version, fetched_day)?;
                }
                IndexFetchStatus::Baked => {
                    writeln!(writer, "✓ {} ({})\n", d_tag, dataset_version)?;
                }
            }

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
            writeln!(writer, "  Dataset version:    v{}", dataset_version)?;
            writeln!(writer, "  Manifest digest:    {}", manifest_digest)?;
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
            writeln!(
                writer,
                "    NHS England. ({}). NHS ODS Dataset ({} cut, v{}).\n    NHS TRUD. https://isd.digital.nhs.uk/trud",
                year, d_tag, dataset_version
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

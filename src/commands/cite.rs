use anyhow::{Context, Result};
use chrono::Datelike;
use clap::Parser;
use serde_json::json;
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


pub fn run_with_writer_and_fetcher<F: crate::commands::pull::OciBlobFetcher>(
    args: Args,
    writer: &mut dyn std::io::Write,
    _fetcher: &F,
    cwd: &Path,
) -> Result<()> {
    let input_dir = match args.input {
        Some(ref dir) => dir.clone(),
        None => crate::workspace::resolve_parquet_input_from(cwd, None)?,
    };

    // Everything this command says about the release comes from the object its Parquet files
    // carry: the release date, the dataset version, the source archive's hash, the licence and
    // attribution. A directory holding only the files (what `oras pull` yields) cites the same.
    let record = crate::provenance::read_release(&input_dir)?;
    if matches!(record, crate::provenance::ReleaseRecord::NoFiles) {
        anyhow::bail!(
            "✖ No Parquet files found in the directory '{}'. Did you run `ods make` first?",
            input_dir.display()
        );
    }
    let Some(facts) = record.facts().cloned() else {
        eprintln!("{}", crate::provenance::format_record_refusal(&input_dir, &record).unwrap_or_default());
        return Err(crate::commands::pull::AlreadyReported.into());
    };
    let dataset_version = facts.dataset_version.clone();
    let archive_sha256 = facts.source_sha256_upper();
    let trud_date = facts.release_date.clone();
    let attribution = facts.license().attribution.clone();

    // 1. Discover workspace and load/cache index
    let workspace_root = match crate::workspace::find_workspace_root_from(&input_dir, None)? {
        Some(ws) => Some(ws),
        None => crate::workspace::find_workspace_root_from(cwd, None)?,
    };

    let (index, origin) = crate::commands::pull::resolve_index(
        workspace_root.as_deref().unwrap_or(cwd),
        None,
        false,
        false,
        _fetcher,
    )?;

    // 2. Verify directory against index: the manifest rebuilt from the files, by digest
    let outcome = crate::workspace::verify_release_dir(&input_dir, &index);
    let manifest_digest = match &outcome {
        crate::workspace::VerificationOutcome::NoProvenance => {
            eprintln!("{}", crate::provenance::format_no_provenance_error(&input_dir));
            return Err(crate::commands::pull::AlreadyReported.into());
        }
        crate::workspace::VerificationOutcome::Corrupted(err) => {
            eprintln!("{}", err);
            return Err(crate::commands::pull::AlreadyReported.into());
        }
        crate::workspace::VerificationOutcome::NoFiles => {
            anyhow::bail!("✖ No Parquet files found in the directory '{}'", input_dir.display());
        }
        crate::workspace::VerificationOutcome::VerifiedPublished { digest, .. }
        | crate::workspace::VerificationOutcome::DateUnknown { digest, .. }
        | crate::workspace::VerificationOutcome::DifferentArchive { digest, .. }
        | crate::workspace::VerificationOutcome::VersionUnpublished { digest, .. } => digest.clone(),
        crate::workspace::VerificationOutcome::DifferentBytes { reconstructed_digest, .. } => reconstructed_digest.clone(),
    };

    let is_published = matches!(outcome, crate::workspace::VerificationOutcome::VerifiedPublished { .. });

    // 4. Check for withdrawal in index
    let mut published_doi: Option<String> = None;
    let mut withdrawal_reason: Option<String> = None;
    let dataset = index.dataset(facts.version()).map(|(_, d)| d);

    if let Some(entry) = dataset {
        if let Some(ref reason) = entry.withdrawn {
            withdrawal_reason = Some(reason.clone());
        }
        if is_published {
            published_doi = entry.doi.clone();
        }
    }

    // Known issues with the source release these files were built from: the index row's, when
    // it records the same archive.
    let source_issue_urls: Vec<(String, String)> = index
        .release(&trud_date)
        .filter(|r| r.source.hash == facts.source.hash)
        .map(|r| {
            r.source
                .issues
                .iter()
                .map(|id| (id.clone(), crate::index::source_issue_url(id)))
                .collect()
        })
        .unwrap_or_default();
    let source_note = if source_issue_urls.is_empty() {
        format!("Release {}, SHA-256 {}.", trud_date, archive_sha256)
    } else {
        format!(
            "Release {}, SHA-256 {}. Known source issues: {}.",
            trud_date,
            archive_sha256,
            source_issue_urls.iter().map(|(_, url)| url.as_str()).collect::<Vec<_>>().join(", ")
        )
    };

    // The tool that built a published dataset is on its index row; a release the index doesn't
    // know is cited against the ods running now.
    let tool_version = if is_published {
        dataset
            .as_ref()
            .map(|d| d.tool_version.clone())
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
    } else {
        env!("CARGO_PKG_VERSION").to_string()
    };

    let parsed_date = chrono::NaiveDate::parse_from_str(&trud_date, "%Y-%m-%d")
        .context("parsing release date")?;
    let year = parsed_date.year();
    let d_y = parsed_date.year();
    let d_m = parsed_date.month();
    let d_d = parsed_date.day();
    let d_month = format!("{:02}", d_m);

    let d_tag = &trud_date;
    let src_key = format!("nhs-ods-xml/{}", d_tag);
    let data_key = format!("ods-data/{}_{}", d_tag, dataset_version);
    let tool_key = format!("ods/v{}", tool_version);

    let data_url = if let Some(ref doi) = published_doi {
        if doi.starts_with("http") {
            doi.clone()
        } else {
            format!("https://doi.org/{}", doi)
        }
    } else {
        "https://ods.fyi".to_string()
    };

    match args.format.to_lowercase().as_str() {
        "bibtex" => {
            // 1. Upstream source
            writeln!(writer, "@misc{{{},", src_key)?;
            writeln!(writer, "  author = {{NHS England}},")?;
            writeln!(
                writer,
                "  title = {{NHS Organisation Data Service XML Data, release {}}},",
                d_tag
            )?;
            writeln!(writer, "  year = {{{}}},", year)?;
            writeln!(writer, "  month = {{{}}},", d_month)?;
            writeln!(writer, "  howpublished = {{NHS TRUD}},")?;
            writeln!(writer, "  url = {{https://isd.digital.nhs.uk/trud}},")?;
            writeln!(writer, "  note = {{{}}}", source_note)?;
            writeln!(writer, "}}\n")?;

            // 2. Data release
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
            if is_published {
                writeln!(writer, "  howpublished = {{ods.fyi}},")?;
                writeln!(writer, "  url = {{{}}},", data_url)?;
                if let Some(ref doi) = published_doi {
                    writeln!(writer, "  doi = {{{}}},", doi)?;
                }
            }
            writeln!(
                writer,
                "  note = {{Manifest: {}. {}}}",
                manifest_digest,
                attribution
            )?;
            writeln!(writer, "}}\n")?;

            // 3. Tool
            writeln!(writer, "@misc{{{},", tool_key)?;
            writeln!(writer, "  author = {{Evans, Oli}},")?;
            writeln!(writer, "  title = {{ods}},")?;
            writeln!(writer, "  year = {{{}}},", year)?;
            writeln!(writer, "  version = {{{}}},", tool_version)?;
            writeln!(writer, "  howpublished = {{Computer software}},")?;
            writeln!(writer, "  url = {{https://github.com/olizilla/ods}}")?;
            writeln!(writer, "}}")?;
        }
        "csljson" | "csl-json" | "json" => {
            let src_obj = json!({
                "type": "dataset",
                "id": src_key,
                "title": format!("NHS Organisation Data Service XML Data, release {}", d_tag),
                "author": [{ "literal": "NHS England" }],
                "issued": { "date-parts": [[d_y, d_m, d_d]] },
                "publisher": "NHS TRUD",
                "URL": "https://isd.digital.nhs.uk/trud",
                "note": source_note
            });

            let mut data_map = serde_json::Map::new();
            data_map.insert("type".to_string(), json!("dataset"));
            data_map.insert("id".to_string(), json!(data_key));
            data_map.insert(
                "title".to_string(),
                json!(format!(
                    "ods: NHS Organisation Data as verifiable Parquet files, release {}",
                    d_tag
                )),
            );
            data_map.insert("author".to_string(), json!([{ "family": "Evans", "given": "Oli" }]));
            data_map.insert("issued".to_string(), json!({ "date-parts": [[d_y, d_m, d_d]] }));
            if is_published {
                data_map.insert("publisher".to_string(), json!("ods.fyi"));
                data_map.insert("URL".to_string(), json!(data_url));
                if let Some(ref doi) = published_doi {
                    data_map.insert("DOI".to_string(), json!(doi));
                }
            }
            data_map.insert("version".to_string(), json!(dataset_version));
            data_map.insert(
                "note".to_string(),
                json!(format!(
                    "Manifest: {}. {}",
                    manifest_digest,
                    attribution
                )),
            );
            let data_obj = serde_json::Value::Object(data_map);

            let tool_obj = json!({
                "type": "software",
                "id": tool_key,
                "title": "ods",
                "author": [{ "family": "Evans", "given": "Oli" }],
                "issued": { "date-parts": [[d_y]] },
                "URL": "https://github.com/olizilla/ods",
                "version": tool_version
            });

            let csl = json!([src_obj, data_obj, tool_obj]);
            writeln!(writer, "{}", serde_json::to_string_pretty(&csl)?)?;
        }
        "apa" => {
            writeln!(
                writer,
                "NHS England. ({}). NHS Organisation Data Service XML Data, release {} [Data set]. NHS TRUD. https://isd.digital.nhs.uk/trud\n",
                year, d_tag
            )?;
            if is_published {
                writeln!(
                    writer,
                    "Evans, O. ({}). ods: NHS Organisation Data as verifiable Parquet files, release {} (Version {}) [Data set]. ods.fyi. {}\n",
                    year, d_tag, dataset_version, data_url
                )?;
            } else {
                writeln!(
                    writer,
                    "Evans, O. ({}). ods: NHS Organisation Data as verifiable Parquet files, release {} (Version {}) [Data set].\n",
                    year, d_tag, dataset_version
                )?;
            }
            writeln!(
                writer,
                "Evans, O. ({}). ods (Version {}) [Computer software]. https://github.com/olizilla/ods",
                year, tool_version
            )?;
        }
        _ => {
            // Source header on stdout
            let header_lines = crate::workspace::format_cite_source_header(&input_dir, d_tag, &dataset_version, Some(cwd));
            for line in header_lines {
                writeln!(writer, "{}", line)?;
            }
            writeln!(writer)?;

            // How to Cite
            writeln!(writer, "How to Cite")?;
            writeln!(writer, "  The source:")?;
            writeln!(
                writer,
                "    NHS England. ({}). NHS Organisation Data Service XML Data, release {}\n    [Data set]. NHS TRUD. https://isd.digital.nhs.uk/trud\n    {}",
                year,
                d_tag,
                wrap_words(&attribution, 80).join("\n    ")
            )?;
            writeln!(writer)?;
            writeln!(writer, "  The data:")?;
            if is_published {
                writeln!(
                    writer,
                    "    Evans, O. ({}). ods: NHS Organisation Data as verifiable Parquet files,\n    release {} (Version {}) [Data set]. ods.fyi. {}",
                    year, d_tag, dataset_version, data_url
                )?;
            } else {
                writeln!(
                    writer,
                    "    Evans, O. ({}). ods: NHS Organisation Data as verifiable Parquet files,\n    release {} (Version {}) [Data set].",
                    year, d_tag, dataset_version
                )?;
            }
            writeln!(writer)?;
            writeln!(writer, "  The tool:")?;
            writeln!(
                writer,
                "    Evans, O. ({}). ods (Version {}) [Computer software].\n    https://github.com/olizilla/ods",
                year, tool_version
            )?;
            writeln!(writer)?;

            // Data availability
            writeln!(writer, "Data availability")?;
            match &outcome {
                crate::workspace::VerificationOutcome::VerifiedPublished { .. } => {
                    writeln!(
                        writer,
                        "  {} is available from {}. This OCI manifest\n  digest identifies its exact files:\n    {}",
                        data_key, data_url, manifest_digest
                    )?;
                    writeln!(writer)?;
                    writeln!(
                        writer,
                        "  {} built it from {}, the TRUD archive with SHA-256:\n    {}",
                        tool_key, src_key, archive_sha256
                    )?;
                    writeln!(
                        writer,
                        "  Rebuild and verify with `ods trud pull {} && ods make`.",
                        d_tag
                    )?;
                }
                crate::workspace::VerificationOutcome::DateUnknown { .. } => {
                    writeln!(
                        writer,
                        "  {} isn't in the release index this ods read, so where to get\n  it is unknown. This OCI manifest digest identifies its exact files:\n    {}",
                        data_key, manifest_digest
                    )?;
                    writeln!(writer)?;
                    writeln!(
                        writer,
                        "  It was built from {}, the TRUD archive with SHA-256:\n    {}",
                        src_key, archive_sha256
                    )?;
                    writeln!(
                        writer,
                        "  Rebuild and verify with `ods trud pull {} && ods make`.",
                        d_tag
                    )?;
                }
                crate::workspace::VerificationOutcome::DifferentArchive { this_archive_sha256, .. } => {
                    writeln!(
                        writer,
                        "  {} was built from a different TRUD archive than the one\n  published for {}, so it isn't the published dataset. This OCI manifest digest\n  identifies its exact files:\n    {}",
                        data_key, d_tag, manifest_digest
                    )?;
                    writeln!(writer)?;
                    writeln!(
                        writer,
                        "  It was built from the TRUD archive with SHA-256:\n    {}",
                        this_archive_sha256
                    )?;
                }
                crate::workspace::VerificationOutcome::VersionUnpublished { published_versions, .. } => {
                    let pub_list = published_versions
                        .iter()
                        .map(|v| format!("ods-data/{}_{}", d_tag, v))
                        .collect::<Vec<_>>()
                        .join(", ");
                    writeln!(
                        writer,
                        "  {} hasn't been published. The published release for\n  {} is {}. This OCI manifest digest identifies its\n  exact files:\n    {}",
                        data_key, d_tag, pub_list, manifest_digest
                    )?;
                    writeln!(writer)?;
                    writeln!(
                        writer,
                        "  It was built from {}, the TRUD archive with SHA-256:\n    {}",
                        src_key, archive_sha256
                    )?;
                }
                crate::workspace::VerificationOutcome::DifferentBytes { reconstructed_digest, .. } => {
                    writeln!(
                        writer,
                        "  {} as built here differs from the published dataset of that\n  name. This OCI manifest digest identifies the files here:\n    {}",
                        data_key, reconstructed_digest
                    )?;
                    writeln!(writer)?;
                    writeln!(
                        writer,
                        "  It was built from {}, the TRUD archive with SHA-256:\n    {}",
                        src_key, archive_sha256
                    )?;
                }
                _ => {}
            }

            if !source_issue_urls.is_empty() {
                writeln!(writer)?;
                writeln!(writer, "Known source issues")?;
                for (id, url) in &source_issue_urls {
                    writeln!(writer, "  {}\n    {}", id, url)?;
                }
            }
        }
    }

    // Stderr warnings for cases A, B, C, F
    match &outcome {
        crate::workspace::VerificationOutcome::DateUnknown { date, .. } => {
            eprintln!("! {} isn't in the cached release index from ({})", date, origin);
            eprintln!("  To update the release index run: ods pull");
        }
        crate::workspace::VerificationOutcome::DifferentArchive {
            date,
            this_archive_sha256,
            published_archive_sha256,
            ..
        } => {
            eprintln!(
                "✖ releases/{} was built from a different TRUD archive than the published {}",
                date, date
            );
            eprintln!("  {:<10}  sha256 {}", "this build", this_archive_sha256);
            eprintln!("  {:<10}  sha256 {}", "published", published_archive_sha256);
            eprintln!("  Cite the published release: ods pull {}", date);
        }
        crate::workspace::VerificationOutcome::VersionUnpublished {
            date,
            version,
            published_versions,
            ..
        } => {
            eprintln!(
                "! dataset {} was never published for {}. Published: {}",
                version,
                date,
                published_versions.join(", ")
            );
        }
        crate::workspace::VerificationOutcome::DifferentBytes {
            date,
            version,
            published_digest,
            reconstructed_digest,
        } => {
            let pub_d = if published_digest.starts_with("sha256:") {
                published_digest.clone()
            } else {
                format!("sha256:{}", published_digest)
            };
            let recon_d = if reconstructed_digest.starts_with("sha256:") {
                reconstructed_digest.clone()
            } else {
                format!("sha256:{}", reconstructed_digest)
            };
            eprintln!(
                "✖ releases/{} doesn't match the published ods-data/{}_{}",
                date, date, version
            );
            eprintln!("  {:<10}  {}", "published", pub_d);
            eprintln!("  {:<10}  {}", "this build", recon_d);
            eprintln!(
                "  Its files name that release but aren't its files: an ods that doesn't reproduce\n  dataset {} built them, or they've changed since. Cite the published release: ods pull {}",
                version, date
            );
        }
        _ => {}
    }

    let is_human = args.format == "text";
    crate::workspace::check_and_emit_staleness_nudge(&input_dir, is_human);

    if let Some(ref reason) = withdrawal_reason {
        eprintln!(
            "✖ {} ({}) was withdrawn: {}\n  Pull a valid release: ods pull",
            trud_date, dataset_version, reason
        );
        return Err(crate::commands::pull::AlreadyReported.into());
    }

    if matches!(
        outcome,
        crate::workspace::VerificationOutcome::DifferentArchive { .. }
            | crate::workspace::VerificationOutcome::DifferentBytes { .. }
    ) {
        return Err(crate::commands::pull::AlreadyReported.into());
    }

    Ok(())
}

/// Breaks `text` at spaces into lines of at most `width` characters.
fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split(' ') {
        match lines.last_mut() {
            Some(line) if line.len() + 1 + word.len() <= width => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    lines
}

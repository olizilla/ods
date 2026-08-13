use anyhow::{Context, Result};
use clap::Parser;
use serde::Deserialize;
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::PathBuf;

use crate::provenance::{compute_file_sha256, OdsProvenance};
use crate::workspace::{find_workspace_root, prepare_release_dir, set_active_release, DEFAULT_WORKSPACE_DIR};

pub const TRUD_ODS_ITEM_ID: &str = "341";

#[derive(Parser, Debug)]
pub struct Args {
    /// TRUD API Key (defaults to $NHS_TRUD_API_KEY if omitted)
    #[arg(long, env = "NHS_TRUD_API_KEY")]
    pub api_key: Option<String>,

    /// Target TRUD release date (defaults to latest available)
    #[arg(long)]
    pub release: Option<String>,

    /// Custom output directory for downloaded archive and provenance (defaults to workspace release dir ./ods_data/releases/<date>/)
    #[arg(long, short = 'o')]
    pub output: Option<PathBuf>,

    /// Verify local archive file checksum against TRUD API without downloading
    #[arg(long)]
    pub verify_only: Option<PathBuf>,

    /// Local archive or directory containing canned TRUD response.json & ZIP for offline testing
    #[arg(long)]
    pub local_archive: Option<PathBuf>,

    /// Print verbose output (URL fetched and raw API response text before deserialization)
    #[arg(long, short)]
    pub verbose: bool,
}

#[derive(Debug, Deserialize)]
pub struct TrudReleaseItem {
    pub id: String,

    #[serde(default)]
    pub name: Option<String>,

    #[serde(rename = "releaseDate")]
    pub release_date: String,

    #[serde(rename = "archiveFileName")]
    pub archive_file_name: String,

    #[serde(rename = "archiveFileSha256")]
    pub archive_file_sha256: String,

    #[serde(rename = "archiveFileSizeBytes")]
    pub archive_file_size: u64,

    #[serde(rename = "archiveFileUrl")]
    pub download_url: String,
}

#[derive(Debug, Deserialize)]
struct TrudApiResponse {
    #[serde(rename = "apiVersion", default)]
    pub _api_version: Option<String>,

    pub releases: Vec<TrudReleaseItem>,
}

pub fn run(args: Args) -> Result<()> {
    if let Some(ref local_path) = args.local_archive {
        return run_local_archive(&args, local_path);
    }

    let api_key = match args.api_key {
        Some(key) if !key.trim().is_empty() => key,
        _ => {
            anyhow::bail!(
                "✖ Missing API Key\n  Please set $NHS_TRUD_API_KEY environment variable or pass --api-key <KEY>.\n  See: https://isd.digital.nhs.uk/trud/user/authenticated/group/0/pack/341/subpack/160/releases"
            );
        }
    };

    println!("Querying NHS TRUD REST API for ODS release...");
    let releases = fetch_trud_releases(&api_key, args.verbose)?;

    let target_release = if let Some(target_date) = &args.release {
        releases.into_iter().find(|r| r.release_date == *target_date)
            .context(format!("✖ Target Release Not Found\n  No TRUD release found for date {}", target_date))?
    } else {
        releases.into_iter().next()
            .context("✖ Target Release Not Found\n  No TRUD releases returned by API")?
    };

    let size_mb = target_release.archive_file_size / (1024 * 1024);
    println!("✓ Found Release: {}, Size: {} MB", target_release.release_date, size_mb);

    if let Some(verify_path) = args.verify_only {
        println!("Verifying local archive {}...", verify_path.display());
        let local_sha256 = compute_file_sha256(&verify_path)?;
        if local_sha256.eq_ignore_ascii_case(&target_release.archive_file_sha256) {
            println!("✓ SHA-256 OK ({})", target_release.archive_file_sha256);
            println!("Done!");
            return Ok(());
        } else {
            let bad_path = mark_bad_sha_file(&verify_path);
            anyhow::bail!(
                "✖ SHA-256 Checksum Failed!\n  Local SHA-256: {}\n  TRUD SHA-256:  {}\n  Renamed bad local file to {}",
                local_sha256,
                target_release.archive_file_sha256,
                bad_path.display()
            );
        }
    }

    let (dest_dir, trud_dir, is_workspace) = match args.output {
        Some(out) => {
            let trud = out.join("trud");
            std::fs::create_dir_all(&trud)?;
            (out, trud, false)
        }
        None => {
            let workspace_root = find_workspace_root()
                .unwrap_or_else(|| PathBuf::from(DEFAULT_WORKSPACE_DIR));
            let release_dir = prepare_release_dir(&workspace_root, &target_release.release_date)?;
            let trud = release_dir.join("trud");
            std::fs::create_dir_all(&trud)?;
            (release_dir, trud, true)
        }
    };

    let dest_path = trud_dir.join(&target_release.archive_file_name);

    if dest_path.exists() {
        let local_sha256 = compute_file_sha256(&dest_path)?;
        if local_sha256.eq_ignore_ascii_case(&target_release.archive_file_sha256) {
            println!("* Using {} already in ods_data dir", target_release.archive_file_name);
            println!("Verifying archive SHA-256...");
            println!("✓ SHA-256 OK ({})", target_release.archive_file_sha256);
            write_provenance_json(&dest_dir, &target_release, &local_sha256, false)?;
            if is_workspace {
                let workspace_root = find_workspace_root().unwrap_or_else(|| PathBuf::from(DEFAULT_WORKSPACE_DIR));
                ensure_active_release_link(&workspace_root, &target_release.release_date)?;
            }
            println!("Done!");
            return Ok(());
        } else {
            let bad_path = mark_bad_sha_file(&dest_path);
            println!("✖ SHA-256 Checksum Mismatch on cached file! Renamed to {}", bad_path.display());
            println!("Fetching fresh archive from TRUD...");
        }
    }

    println!("Downloading archive {}...", target_release.archive_file_name);
    download_archive(&target_release.download_url, &dest_path, &api_key, args.verbose)?;
    println!("✓ Saved to {}", trud_dir.display());

    println!("Verifying archive SHA-256...");
    let mut local_sha256 = compute_file_sha256(&dest_path)?;

    // Automated 1-retry fallback for remote downloads on hash mismatch
    if !local_sha256.eq_ignore_ascii_case(&target_release.archive_file_sha256) {
        println!("✖ SHA-256 Checksum Mismatch on remote download! Retrying download (attempt 2/2)...");
        let _ = std::fs::remove_file(&dest_path);
        download_archive(&target_release.download_url, &dest_path, &api_key, args.verbose)?;
        local_sha256 = compute_file_sha256(&dest_path)?;

        if !local_sha256.eq_ignore_ascii_case(&target_release.archive_file_sha256) {
            let bad_path = mark_bad_sha_file(&dest_path);
            anyhow::bail!(
                "SHA-256 Mismatch after retry. Expected: {}, Got: {}. File renamed to {}",
                target_release.archive_file_sha256,
                local_sha256,
                bad_path.display()
            );
        }
    }

    println!("✓ SHA-256 OK ({})", local_sha256);

    write_provenance_json(&dest_dir, &target_release, &local_sha256, true)?;
    if is_workspace {
        let workspace_root = find_workspace_root().unwrap_or_else(|| PathBuf::from(DEFAULT_WORKSPACE_DIR));
        ensure_active_release_link(&workspace_root, &target_release.release_date)?;
    }

    println!("Done!");
    Ok(())
}

fn mark_bad_sha_file(path: &PathBuf) -> PathBuf {
    let bad_path = PathBuf::from(format!("{}.bad-sha", path.display()));
    let _ = std::fs::rename(path, &bad_path);
    bad_path
}

fn write_provenance_json(release_dir: &std::path::Path, release: &TrudReleaseItem, sha256: &str, force: bool) -> Result<()> {
    let prov_path = release_dir.join(crate::provenance::PROVENANCE_FILENAME);
    if prov_path.exists() && !force {
        return Ok(());
    }
    write_provenance_json_with_verification(release_dir, release, sha256, true)
}

fn ensure_active_release_link(workspace_root: &std::path::Path, release_date: &str) -> Result<()> {
    if let Ok((active_date, _)) = crate::workspace::get_active_release(workspace_root) {
        if active_date == release_date {
            // Current link already points to target release date - omit lines
            return Ok(());
        }
    }

    println!("Updating current release link...");
    set_active_release(workspace_root, release_date)?;
    println!("✓ Linked to releases/{}", release_date);
    Ok(())
}

pub fn fetch_trud_releases(api_key: &str, verbose: bool) -> Result<Vec<TrudReleaseItem>> {
    let url = format!(
        "https://isd.digital.nhs.uk/trud/api/v1/keys/{}/items/{}/releases",
        api_key, TRUD_ODS_ITEM_ID
    );

    if verbose {
        let safe_url = crate::provenance::sanitize_trud_url(&url, Some(api_key));
        println!("[VERBOSE] Request URL: {}", safe_url);
    }

    let response = ureq::get(&url)
        .call()
        .context("Failed to connect to TRUD REST API")?;

    let status = response.status();
    let raw_body = response.into_string()
        .context("Failed to read response body from TRUD REST API")?;

    if verbose {
        let safe_body = crate::provenance::sanitize_trud_url(&raw_body, Some(api_key));
        println!("[VERBOSE] HTTP Status: {}", status);
        println!("[VERBOSE] Response Body:\n{}", safe_body);
    }

    let resp: TrudApiResponse = serde_json::from_str(&raw_body)
        .context("Failed to parse TRUD REST API response JSON")?;

    // Sanitize download URLs inside release items so API keys are never leaked
    let sanitized_releases = resp.releases.into_iter().map(|mut r| {
        r.download_url = crate::provenance::sanitize_trud_url(&r.download_url, Some(api_key));
        r
    }).collect();

    Ok(sanitized_releases)
}

fn download_archive(url: &str, dest_path: &PathBuf, api_key: &str, verbose: bool) -> Result<()> {
    let real_url = url.replace("<REDACTED_API_KEY>", api_key);
    if verbose {
        let safe_url = crate::provenance::sanitize_trud_url(&real_url, Some(api_key));
        println!("[VERBOSE] Download URL: {}", safe_url);
    }

    let resp = ureq::get(&real_url)
        .call()
        .context("Failed to initiate archive download from TRUD")?;

    if verbose {
        println!("[VERBOSE] Download HTTP Status: {}", resp.status());
    }

    let file = File::create(dest_path)?;
    let mut writer = BufWriter::with_capacity(128 * 1024, file);
    let mut reader = resp.into_reader();
    let mut buffer = [0u8; 64 * 1024];

    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        writer.write_all(&buffer[..n])?;
    }

    writer.flush()?;
    Ok(())
}

pub fn parse_trud_filename(filename: &str) -> Result<(String, Option<String>)> {
    let clean = filename.trim();
    let name_without_ext = clean.strip_suffix(".zip").unwrap_or(clean);

    let parts: Vec<&str> = name_without_ext.split('_').collect();
    if parts.len() >= 4 && parts[0] == "hscorgrefdataxml" && parts[1] == "data" {
        let version = parts[2];
        let timestamp = parts[3];

        let release_name = format!("Release {}", version);

        if timestamp.len() >= 8 {
            let year = &timestamp[0..4];
            let month = &timestamp[4..6];
            let day = &timestamp[6..8];
            let date_str = format!("{}-{}-{}", year, month, day);
            return Ok((date_str, Some(release_name)));
        }
    }

    anyhow::bail!(
        "Unrecognized TRUD ZIP filename format '{}'. Expected format: hscorgrefdataxml_data_<VERSION>_<YYYYMMDD...>.zip",
        filename
    )
}

fn run_local_archive(args: &Args, local_path: &std::path::Path) -> Result<()> {
    let zip_file = if local_path.is_dir() {
        let mut zip = None;
        for entry in walkdir::WalkDir::new(local_path) {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() {
                let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
                if name.ends_with(".zip") {
                    zip = Some(path.to_path_buf());
                    break;
                }
            }
        }
        zip.context(format!("No .zip file found in local archive dir {}", local_path.display()))?
    } else {
        local_path.to_path_buf()
    };

    println!("* Using local offline archive: {}", zip_file.display());
    let local_sha256 = compute_file_sha256(&zip_file)?;
    println!("Verifying archive SHA-256...");
    println!("✓ SHA-256 OK ({})", local_sha256);

    let file_name = zip_file.file_name()
        .and_then(|s| s.to_str())
        .context("Invalid archive filename UTF-8")?
        .to_string();
    let file_size = std::fs::metadata(&zip_file)?.len();

    let (release_date, trud_name) = parse_trud_filename(&file_name)?;

    let release_item = TrudReleaseItem {
        id: TRUD_ODS_ITEM_ID.to_string(),
        name: trud_name,
        release_date: release_date.clone(),
        archive_file_name: file_name.clone(),
        archive_file_sha256: local_sha256.clone(),
        archive_file_size: file_size,
        download_url: format!("file://{}", zip_file.display()),
    };

    let (dest_dir, trud_dir, is_workspace) = match &args.output {
        Some(out) => {
            let trud = out.join("trud");
            std::fs::create_dir_all(&trud)?;
            (out.clone(), trud, false)
        }
        None => {
            let workspace_root = find_workspace_root().unwrap_or_else(|| PathBuf::from(DEFAULT_WORKSPACE_DIR));
            let release_dir = prepare_release_dir(&workspace_root, &release_date)?;
            let trud = release_dir.join("trud");
            std::fs::create_dir_all(&trud)?;
            (release_dir, trud, true)
        }
    };

    let dest_path = trud_dir.join(&file_name);
    if dest_path != zip_file {
        std::fs::copy(&zip_file, &dest_path)?;
    }

    println!("✓ Saved to {}", trud_dir.display());
    write_provenance_json_with_verification(&dest_dir, &release_item, &local_sha256, false)?;
    if is_workspace {
        let workspace_root = find_workspace_root().unwrap_or_else(|| PathBuf::from(DEFAULT_WORKSPACE_DIR));
        ensure_active_release_link(&workspace_root, &release_date)?;
    }
    println!("Done!");
    Ok(())
}

fn write_provenance_json_with_verification(
    release_dir: &std::path::Path,
    release: &TrudReleaseItem,
    _sha256: &str,
    verified: bool,
) -> Result<()> {
    let prov_path = release_dir.join(crate::provenance::PROVENANCE_FILENAME);

    println!("Writing provenance metadata...");
    let trud_dir = release_dir.join("trud");
    let (pub_date, pub_seq, pub_type, pub_source, pub_version, pub_count) = if let Ok(xml_path) = crate::commands::ndjson::find_xml_file(&trud_dir) {
        if let Ok((parsed_prov, _, parsed_map)) = crate::commands::ndjson::parse_single_pass(&xml_path) {
            (
                parsed_prov.publication_date,
                parsed_prov.publication_seq_num,
                parsed_prov.publication_type,
                parsed_prov.publication_source,
                parsed_prov.publication_schema_version,
                parsed_prov.publication_record_count.or(Some(parsed_map.len())),
            )
        } else {
            (None, None, None, None, None, None)
        }
    } else {
        (None, None, None, None, None, None)
    };

    let prov = OdsProvenance {
        type_tag: crate::provenance::NDJSON_TYPE_TAG.to_string(),
        trud_release_name: release.name.clone(),
        trud_release_date: Some(release.release_date.clone()),
        trud_release_url: Some(release.download_url.clone()),
        trud_release_sha256: Some(release.archive_file_sha256.clone()),
        trud_release_sha256_verified: Some(verified),
        trud_release_file: Some(release.archive_file_name.clone()),
        trud_release_filesize_bytes: Some(release.archive_file_size),
        publication_date: pub_date,
        publication_seq_num: pub_seq,
        publication_type: pub_type,
        publication_source: pub_source,
        publication_schema_version: pub_version,
        publication_record_count: pub_count,
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        derived_artifacts: None,
    };

    let json = serde_json::to_string_pretty(&prov)?;
    std::fs::write(&prov_path, json).context("Failed to write _provenance.json")?;
    println!("✓ Saved to {}", prov_path.display());
    Ok(())
}


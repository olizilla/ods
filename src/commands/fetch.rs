use anyhow::{Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crate::progress::{
    format_duration, format_size, render_release_block, Progress, ProgressCaps, ReleaseBlockLink,
    ReleaseBlockParams, ReleaseBlockState,
};
use crate::provenance::{compute_file_sha256, OdsProvenance};
use crate::workspace::Workspace;

pub const TRUD_ODS_ITEM_ID: &str = "341";

#[derive(Parser, Debug, Default, Clone)]
pub struct Args {
    /// Target TRUD release date in YYYY-MM-DD format (defaults to latest available)
    pub release_date: Option<String>,

    /// Fetch all available TRUD releases
    #[arg(long, hide = true)]
    pub all: bool,

    /// Force re-download or re-pull of specified release
    #[arg(long, short = 'f')]
    pub force: bool,

    /// Number of concurrent download workers (max: 8)
    #[arg(long, default_value_t = 4)]
    pub jobs: usize,

    /// Show errors and summary only
    #[arg(long, short = 'q', conflicts_with = "verbose")]
    pub quiet: bool,

    /// Disable interactive live progress animations
    #[arg(long)]
    pub no_progress: bool,

    /// Output format (ndjson for pull outcomes, json for release list)
    #[arg(long)]
    pub format: Option<String>,

    /// TRUD API Key
    #[arg(long, env = "TRUD_API_KEY")]
    pub api_key: Option<String>,

    /// Custom output directory for downloaded archive and provenance (defaults to workspace release dir ./ods_data/releases/<date>/)
    #[arg(long, short = 'o')]
    pub output: Option<PathBuf>,

    /// Verify local archive file checksum against TRUD API without downloading
    #[arg(long)]
    pub verify_only: Option<PathBuf>,

    /// Local archive or directory containing canned TRUD response.json & ZIP for offline testing
    #[arg(long)]
    pub local_archive: Option<PathBuf>,

    /// Workspace directory (defaults to ./ods_data if omitted)
    #[arg(long, short = 'w')]
    pub workspace: Option<PathBuf>,

    /// Print verbose output (URL fetched and raw API response text before deserialization)
    #[arg(long, short = 'v')]
    pub verbose: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
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

    #[serde(rename = "checksumFileUrl", default)]
    pub checksum_file_url: Option<String>,

    #[serde(rename = "checksumFileName", default)]
    pub checksum_file_name: Option<String>,

    #[serde(rename = "signatureFileUrl", default)]
    pub signature_file_url: Option<String>,

    #[serde(rename = "signatureFileName", default)]
    pub signature_file_name: Option<String>,

    #[serde(rename = "publicKeyFileUrl", default)]
    pub public_key_file_url: Option<String>,

    #[serde(rename = "publicKeyFileName", default)]
    pub public_key_file_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TrudApiResponse {
    #[serde(rename = "apiVersion", default)]
    pub _api_version: Option<String>,

    pub releases: Vec<TrudReleaseItem>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ReleaseOutcome {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trud_release_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trud_release_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trud_release_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trud_release_filesize_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trud_release_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trud_release_sha256_verified: Option<crate::provenance::TrudVerificationSource>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ReleaseListItemJson {
    pub date: String,
    pub size_bytes: u64,
    pub status: String,
}

pub trait TrudFetcher: Send + Sync {
    fn fetch_releases(&self) -> Result<Vec<TrudReleaseItem>>;
    fn releases_raw_json(&self) -> Option<String> {
        None
    }
    fn download_archive(
        &self,
        url: &str,
        dest_path: &Path,
        on_bytes: &(dyn Fn(u64) + Send + Sync),
    ) -> Result<()>;
    fn download_file(&self, url: &str, dest_path: &Path) -> Result<()> {
        self.download_archive(url, dest_path, &|_| {})
    }
}

pub struct UreqTrudFetcher {
    pub api_key: String,
    pub agent: ureq::Agent,
    pub verbose: bool,
    pub cached_raw_json: std::sync::Mutex<Option<String>>,
}

impl UreqTrudFetcher {
    pub fn new(api_key: &str, verbose: bool) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(10))
            .timeout_read(std::time::Duration::from_secs(30))
            .build();
        Self {
            api_key: api_key.to_string(),
            agent,
            verbose,
            cached_raw_json: std::sync::Mutex::new(None),
        }
    }
}

impl TrudFetcher for UreqTrudFetcher {
    fn fetch_releases(&self) -> Result<Vec<TrudReleaseItem>> {
        let (releases, raw_json) = fetch_trud_releases_and_raw(&self.api_key, self.verbose)?;
        let safe_json = crate::provenance::sanitize_trud_url(&raw_json, Some(&self.api_key));
        *self.cached_raw_json.lock().unwrap() = Some(safe_json);
        Ok(releases)
    }

    fn releases_raw_json(&self) -> Option<String> {
        self.cached_raw_json.lock().unwrap().clone()
    }

    fn download_archive(
        &self,
        url: &str,
        dest_path: &Path,
        on_bytes: &(dyn Fn(u64) + Send + Sync),
    ) -> Result<()> {
        let real_url = url.replace("<REDACTED_API_KEY>", &self.api_key);
        if self.verbose {
            let safe_url = crate::provenance::sanitize_trud_url(&real_url, Some(&self.api_key));
            eprintln!("[VERBOSE] Download URL: {}", safe_url);
        }

        let resp = self.agent.get(&real_url)
            .call()
            .context("Failed to initiate archive download from TRUD")?;

        if self.verbose {
            eprintln!("[VERBOSE] Download HTTP Status: {}", resp.status());
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
            on_bytes(n as u64);
        }

        writer.flush()?;
        Ok(())
    }

    fn download_file(&self, url: &str, dest_path: &Path) -> Result<()> {
        let real_url = url.replace("<REDACTED_API_KEY>", &self.api_key);
        if self.verbose {
            let safe_url = crate::provenance::sanitize_trud_url(&real_url, Some(&self.api_key));
            eprintln!("[VERBOSE] Download File URL: {}", safe_url);
        }

        let resp = self.agent.get(&real_url)
            .call()
            .context("Failed to download file from TRUD")?;

        let file = File::create(dest_path)?;
        let mut writer = BufWriter::new(file);
        let mut reader = resp.into_reader();
        std::io::copy(&mut reader, &mut writer)?;
        writer.flush()?;
        Ok(())
    }
}

pub fn run(args: Args) -> Result<()> {
    let progress = Progress::stderr(ProgressCaps::detect(args.quiet, args.verbose, args.no_progress));
    let ws = Workspace::open_or_create(args.workspace.as_deref())?;
    let workspace_root = ws.root().to_path_buf();

    if let Some(ref local_path) = args.local_archive {
        return run_local_archive(&args, &workspace_root, local_path, &progress);
    }

    let api_key = match args.api_key {
        Some(ref key) if !key.trim().is_empty() => key.clone(),
        _ => {
            anyhow::bail!(
                "✖ Missing API Key\n  Please set $TRUD_API_KEY environment variable or pass --api-key <KEY>.\n  See: https://isd.digital.nhs.uk/trud/user/authenticated/group/0/pack/341/subpack/160/releases"
            );
        }
    };

    let fetcher = UreqTrudFetcher::new(&api_key, args.verbose);
    run_with_fetcher(args, &workspace_root, &fetcher, &progress)
}

pub fn run_with_fetcher<F: TrudFetcher>(
    args: Args,
    workspace_root: &Path,
    fetcher: &F,
    progress: &Progress,
) -> Result<()> {
    progress.step("Querying NHS TRUD…");
    let releases = match fetcher.fetch_releases() {
        Ok(r) => r,
        Err(e) => {
            progress.error("Failed to connect to NHS TRUD API", &[&e.to_string()]);
            return Err(crate::commands::pull::AlreadyReported.into());
        }
    };

    if args.all {
        return pull_all_trud_releases(&args, workspace_root, fetcher, releases, progress);
    }

    let target_release = if let Some(ref target_date) = args.release_date {
        releases.into_iter().find(|r| r.release_date == *target_date)
            .context(format!("✖ Target Release Not Found\n  No TRUD release found for date {}", target_date))?
    } else {
        releases.into_iter().next()
            .context("✖ Target Release Not Found\n  No TRUD releases returned by API")?
    };

    pull_single_release(&args, workspace_root, fetcher, target_release, progress)
}

fn pull_single_release<F: TrudFetcher>(
    args: &Args,
    workspace_root: &Path,
    fetcher: &F,
    target_release: TrudReleaseItem,
    progress: &Progress,
) -> Result<()> {
    if let Some(ref verify_path) = args.verify_only {
        progress.step(&format!("verifying archive SHA-256 for {}…", target_release.release_date));
        let local_sha256 = compute_file_sha256(verify_path)?;
        if local_sha256.eq_ignore_ascii_case(&target_release.archive_file_sha256) {
            progress.settle(&format!(
                "✓ {}  {}  SHA-256 verified by TRUD API",
                target_release.release_date,
                format_size(target_release.archive_file_size)
            ));
            progress.finish("Done!");
            return Ok(());
        } else {
            let bad_path = mark_bad_sha_file(verify_path);
            progress.error(
                "SHA-256 Checksum Failed!",
                &[
                    &format!("Local SHA-256: {}", local_sha256),
                    &format!("TRUD SHA-256:  {}", target_release.archive_file_sha256),
                    &format!("Renamed bad local file to {}", bad_path.display()),
                ],
            );
            anyhow::bail!(crate::commands::pull::AlreadyReported);
        }
    }

    let (dest_dir, trud_dir, is_workspace) = match &args.output {
        Some(out) => {
            let trud = out.join("trud");
            std::fs::create_dir_all(&trud)?;
            (out.clone(), trud, false)
        }
        None => {
            let ws = Workspace::open_or_create(Some(workspace_root))?;
            let release_dir = ws.prepare_release(&target_release.release_date)?;
            let trud = release_dir.join("trud");
            std::fs::create_dir_all(&trud)?;
            (release_dir, trud, true)
        }
    };

    let dest_path = trud_dir.join(&target_release.archive_file_name);

    let link_target = format!("releases/{}", target_release.release_date);

    if dest_path.exists() && !args.force {
        let local_sha256 = compute_file_sha256(&dest_path)?;
        if local_sha256.eq_ignore_ascii_case(&target_release.archive_file_sha256) {
            write_provenance_json(&dest_dir, &target_release, &local_sha256, false, progress)?;
            let mut pin_moved = false;
            if is_workspace {
                pin_moved = update_active_release_link_if_changed(workspace_root, &target_release.release_date)?;
            }

            let att_result = capture_attestations(
                &trud_dir,
                &target_release,
                fetcher,
                fetcher.releases_raw_json().as_deref(),
                args.api_key.as_deref(),
            );
            let file_count = 2 + att_result.captured.len();
            let link = if is_workspace {
                Some(ReleaseBlockLink {
                    target: &link_target,
                    unchanged: !pin_moved,
                })
            } else {
                None
            };
            let hash_opt = if progress.caps().verbose {
                Some(local_sha256.as_str())
            } else {
                None
            };
            let lines = render_release_block(&ReleaseBlockParams {
                date: &target_release.release_date,
                archive_size: target_release.archive_file_size,
                file_count,
                state: &ReleaseBlockState::Cached,
                verified: "sha256 from TRUD API",
                linked: link,
                hash: hash_opt,
                color: progress.caps().is_tty && !progress.caps().no_color,
            });
            progress.finish_block(&lines);

            let failed_reasons: Vec<_> = att_result
                .missing_reasons
                .iter()
                .filter(|r| r.contains("download failed"))
                .cloned()
                .collect();
            for reason in failed_reasons {
                eprintln!("! attestations: {}", reason);
            }

            if args.format.as_deref() == Some("ndjson") {
                emit_ndjson_outcome(&ReleaseOutcome {
                    trud_release_date: Some(target_release.release_date.clone()),
                    trud_release_name: target_release.name.clone(),
                    trud_release_file: Some(target_release.archive_file_name.clone()),
                    trud_release_filesize_bytes: Some(target_release.archive_file_size),
                    trud_release_sha256: Some(target_release.archive_file_sha256.clone()),
                    trud_release_sha256_verified: Some(crate::provenance::TrudVerificationSource::TrudApi),
                    status: "cached".to_string(),
                    path: Some(dest_dir.display().to_string()),
                    error: None,
                });
            }

            return Ok(());
        } else {
            let bad_path = mark_bad_sha_file(&dest_path);
            if progress.caps().verbose {
                eprintln!("✖ SHA-256 Checksum Mismatch on cached file! Renamed to {}", bad_path.display());
            }
        }
    }

    let initial_file_count = 2
        + (target_release.checksum_file_url.is_some() as usize)
        + (target_release.signature_file_url.is_some() as usize)
        + (target_release.public_key_file_url.is_some() as usize);

    let part_path = trud_dir.join(format!("{}.part", target_release.archive_file_name));
    let downloaded_bytes = Arc::new(AtomicU64::new(0));
    let start_time = Instant::now();

    let dl_counter = downloaded_bytes.clone();
    let p = progress.clone();
    let date = target_release.release_date.clone();
    let total_size = target_release.archive_file_size;
    let link_target_cb = link_target.clone();

    let res = fetcher.download_archive(
        &target_release.download_url,
        &part_path,
        &move |n| {
            let cur = dl_counter.fetch_add(n, Ordering::Relaxed) + n;
            let elapsed = start_time.elapsed().as_secs_f64();
            let rate = if elapsed > 0.0 { cur as f64 / elapsed } else { 0.0 };
            let eta = if rate > 0.0 && total_size > cur {
                Some(std::time::Duration::from_secs_f64((total_size - cur) as f64 / rate))
            } else {
                None
            };
            let state = ReleaseBlockState::Downloading {
                bytes_done: cur,
                rate: Some(rate),
                eta,
            };
            let link = if is_workspace {
                Some(ReleaseBlockLink {
                    target: &link_target_cb,
                    unchanged: false,
                })
            } else {
                None
            };
            let lines = render_release_block(&ReleaseBlockParams {
                date: &date,
                archive_size: total_size,
                file_count: initial_file_count,
                state: &state,
                verified: "-",
                linked: link,
                hash: None,
                color: p.caps().is_tty && !p.caps().no_color,
            });
            p.update_live_block(lines);
        },
    );

    if let Err(e) = res {
        let _ = std::fs::remove_file(&part_path);
        progress.clear_live();
        progress.error(
            &format!("Download failed for {}", target_release.release_date),
            &[&e.to_string()],
        );
        return Err(crate::commands::pull::AlreadyReported.into());
    }

    progress.step(&format!("{}   verifying SHA-256…", target_release.release_date));
    let mut local_sha256 = compute_file_sha256(&part_path)?;

    // Automated 1-retry fallback for remote downloads on hash mismatch
    if !local_sha256.eq_ignore_ascii_case(&target_release.archive_file_sha256) {
        let _ = std::fs::remove_file(&part_path);
        progress.step(&format!("{}   SHA-256 mismatch, retrying download (attempt 2/2)…", target_release.release_date));
        fetcher.download_archive(&target_release.download_url, &part_path, &|_| {})?;
        local_sha256 = compute_file_sha256(&part_path)?;

        if !local_sha256.eq_ignore_ascii_case(&target_release.archive_file_sha256) {
            let bad_path = mark_bad_sha_file(&part_path);
            progress.clear_live();
            progress.error(
                &format!("SHA-256 mismatch for {}", target_release.release_date),
                &[
                    &format!("Expected: {}", target_release.archive_file_sha256),
                    &format!("Got:      {}", local_sha256),
                    &format!("File renamed to {}", bad_path.display()),
                ],
            );
            return Err(crate::commands::pull::AlreadyReported.into());
        }
    }

    // Rename .part to .zip once hash matches
    std::fs::rename(&part_path, &dest_path)?;

    write_provenance_json(&dest_dir, &target_release, &local_sha256, true, progress)?;
    let mut pin_moved = false;
    if is_workspace {
        pin_moved = update_active_release_link_if_changed(workspace_root, &target_release.release_date)?;
    }

    let elapsed = start_time.elapsed();
    let att_result = capture_attestations(
        &trud_dir,
        &target_release,
        fetcher,
        fetcher.releases_raw_json().as_deref(),
        args.api_key.as_deref(),
    );
    let final_file_count = 2 + att_result.captured.len();
    let state = ReleaseBlockState::Done { elapsed };
    let link = if is_workspace {
        Some(ReleaseBlockLink {
            target: &link_target,
            unchanged: !pin_moved,
        })
    } else {
        None
    };
    let hash_opt = if progress.caps().verbose {
        Some(local_sha256.as_str())
    } else {
        None
    };
    let lines = render_release_block(&ReleaseBlockParams {
        date: &target_release.release_date,
        archive_size: target_release.archive_file_size,
        file_count: final_file_count,
        state: &state,
        verified: "sha256 from TRUD API",
        linked: link,
        hash: hash_opt,
        color: progress.caps().is_tty && !progress.caps().no_color,
    });
    progress.finish_block(&lines);

    let failed_reasons: Vec<_> = att_result
        .missing_reasons
        .iter()
        .filter(|r| r.contains("download failed"))
        .cloned()
        .collect();
    for reason in failed_reasons {
        eprintln!("! attestations: {}", reason);
    }

    if args.format.as_deref() == Some("ndjson") {
        emit_ndjson_outcome(&ReleaseOutcome {
            trud_release_date: Some(target_release.release_date.clone()),
            trud_release_name: target_release.name.clone(),
            trud_release_file: Some(target_release.archive_file_name.clone()),
            trud_release_filesize_bytes: Some(target_release.archive_file_size),
            trud_release_sha256: Some(local_sha256),
            trud_release_sha256_verified: Some(crate::provenance::TrudVerificationSource::TrudApi),
            status: "downloaded".to_string(),
            path: Some(dest_dir.display().to_string()),
            error: None,
        });
    }

    Ok(())
}

fn pull_all_trud_releases<F: TrudFetcher>(
    args: &Args,
    workspace_root: &Path,
    fetcher: &F,
    mut releases: Vec<TrudReleaseItem>,
    progress: &Progress,
) -> Result<()> {
    let total_count = releases.len();
    let total_planned_bytes: u64 = releases.iter().map(|r| r.archive_file_size).sum();

    progress.settle(&format!(
        "✓ {} releases available · {}",
        total_count,
        format_size(total_planned_bytes)
    ));

    releases.sort_by(|a, b| b.release_date.cmp(&a.release_date)); // newest to oldest

    // Plan Phase: check cached releases with parallel hashing
    let mut to_download: Vec<TrudReleaseItem> = Vec::new();
    let mut cached_releases: Vec<TrudReleaseItem> = Vec::new();
    let checked_count = Arc::new(AtomicU64::new(0));

    use rayon::prelude::*;
    let checked = checked_count.clone();
    let p_clone = progress.clone();
    let ws_root = workspace_root.to_path_buf();

    progress.bar("Checking local archives…", 0, total_count as u64, None, None);

    let check_results: Vec<(TrudReleaseItem, bool)> = releases
        .par_iter()
        .map(|release| {
            let rel_dir = ws_root.join("releases").join(&release.release_date);
            let dest_path = rel_dir.join("trud").join(&release.archive_file_name);
            let is_cached = if dest_path.exists() && !args.force {
                if let Ok(hash) = compute_file_sha256(&dest_path) {
                    hash.eq_ignore_ascii_case(&release.archive_file_sha256)
                } else {
                    false
                }
            } else {
                false
            };
            let cur = checked.fetch_add(1, Ordering::Relaxed) + 1;
            p_clone.bar("Checking local archives…", cur, total_count as u64, None, None);
            (release.clone(), is_cached)
        })
        .collect();

    for (rel, is_cached) in check_results {
        if is_cached {
            cached_releases.push(rel);
        } else {
            to_download.push(rel);
        }
    }

    let to_download_bytes: u64 = to_download.iter().map(|r| r.archive_file_size).sum();
    let cached_count = cached_releases.len();
    let to_download_count = to_download.len();

    if to_download_count == 0 {
        progress.clear_live();
        progress.settle(&format!(
            "✓ {} cached, SHA-256 verified by TRUD API · nothing to download",
            cached_count
        ));
        if args.format.as_deref() == Some("ndjson") {
            for r in cached_releases {
                emit_ndjson_outcome(&ReleaseOutcome {
                    trud_release_date: Some(r.release_date.clone()),
                    trud_release_name: r.name.clone(),
                    trud_release_file: Some(r.archive_file_name.clone()),
                    trud_release_filesize_bytes: Some(r.archive_file_size),
                    trud_release_sha256: Some(r.archive_file_sha256.clone()),
                    trud_release_sha256_verified: Some(crate::provenance::TrudVerificationSource::TrudApi),
                    status: "cached".to_string(),
                    path: Some(workspace_root.join("releases").join(&r.release_date).display().to_string()),
                    error: None,
                });
            }
        }
        return Ok(());
    }

    progress.clear_live();
    progress.settle(&format!(
        "✓ {} cached · {} to download · {}",
        cached_count,
        to_download_count,
        format_size(to_download_bytes)
    ));

    // Execute Phase: download missing releases
    let num_jobs = args.jobs.clamp(1, 8);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_jobs)
        .build()
        .context("Failed to build rayon thread pool")?;

    let downloaded_count = Arc::new(AtomicU64::new(0));
    let failed_count = Arc::new(AtomicU64::new(0));
    let downloaded_bytes = Arc::new(AtomicU64::new(0));
    let batch_start = Instant::now();

    let outcomes = Arc::new(std::sync::Mutex::new(Vec::new()));
    let failures = Arc::new(std::sync::Mutex::new(Vec::new()));
    let existing_active_date = Workspace::open(Some(workspace_root))
        .ok()
        .and_then(|ws| ws.active_release().ok())
        .map(|(d, _)| d);
    let newest_downloaded_date = Arc::new(std::sync::Mutex::new(None));
    let cached_bytes: u64 = cached_releases.iter().map(|r| r.archive_file_size).sum();

    // Show initial batch progress bar at the bottom immediately
    progress.batch_bar(1, to_download_count, 0, to_download_bytes, None, None);

    use std::sync::atomic::AtomicUsize;
    let next_idx = Arc::new(AtomicUsize::new(0));
    let to_download = Arc::new(to_download);

    struct SettledEntry {
        line: String,
        outcome: ReleaseOutcome,
        is_success: bool,
    }

    struct SettleCoordinator {
        next_idx: usize,
        buffered: HashMap<usize, SettledEntry>,
    }

    impl SettleCoordinator {
        fn insert_and_drain(&mut self, idx: usize, entry: SettledEntry) -> Vec<SettledEntry> {
            self.buffered.insert(idx, entry);
            let mut ready = Vec::new();
            let mut cur = self.next_idx;
            while let Some(e) = self.buffered.remove(&cur) {
                cur += 1;
                ready.push(e);
            }
            self.next_idx = cur;
            ready
        }
    }

    let coordinator = Arc::new(std::sync::Mutex::new(SettleCoordinator {
        next_idx: 0,
        buffered: HashMap::new(),
    }));

    let raw_json = Arc::new(fetcher.releases_raw_json());

    pool.scope(|s| {
        for _ in 0..num_jobs {
            let next_idx = next_idx.clone();
            let to_download = to_download.clone();
            let ws_root = workspace_root.to_path_buf();
            let progress = progress.clone();
            let downloaded_count = downloaded_count.clone();
            let failed_count = failed_count.clone();
            let downloaded_bytes = downloaded_bytes.clone();
            let failures = failures.clone();
            let outcomes = outcomes.clone();
            let newest_downloaded_date = newest_downloaded_date.clone();
            let coordinator = coordinator.clone();
            let raw_json = raw_json.clone();

            s.spawn(move |_| {
                loop {
                    let idx = next_idx.fetch_add(1, Ordering::SeqCst);
                    if idx >= to_download.len() {
                        break;
                    }
                    let release = &to_download[idx];

                    progress.set_in_flight(
                        &release.release_date,
                        &format!("{}  {}  downloading…", release.release_date, format_size(release.archive_file_size)),
                    );

                    let release_dir = match Workspace::open_or_create(Some(&ws_root)).and_then(|ws| ws.prepare_release(&release.release_date)) {
                        Ok(d) => d,
                        Err(e) => {
                            progress.remove_in_flight(&release.release_date);
                            failed_count.fetch_add(1, Ordering::Relaxed);
                            failures.lock().unwrap().push((release.release_date.clone(), e.to_string()));
                            let entry = SettledEntry {
                                line: format!("✖ {}  failed to prepare release directory — {}", release.release_date, e),
                                outcome: ReleaseOutcome {
                                    trud_release_date: Some(release.release_date.clone()),
                                    trud_release_name: release.name.clone(),
                                    trud_release_file: Some(release.archive_file_name.clone()),
                                    trud_release_filesize_bytes: Some(release.archive_file_size),
                                    trud_release_sha256: None,
                                    trud_release_sha256_verified: None,
                                    status: "failed".to_string(),
                                    path: None,
                                    error: Some(e.to_string()),
                                },
                                is_success: false,
                            };
                            let ready = coordinator.lock().unwrap().insert_and_drain(idx, entry);
                            for e in ready {
                                progress.settle(&e.line);
                                if args.format.as_deref() == Some("ndjson") {
                                    emit_ndjson_outcome(&e.outcome);
                                }
                                if e.is_success {
                                    outcomes.lock().unwrap().push(e.outcome);
                                }
                            }
                            continue;
                        }
                    };
                    let trud_dir = release_dir.join("trud");
                    let _ = std::fs::create_dir_all(&trud_dir);
                    let dest_path = trud_dir.join(&release.archive_file_name);
                    let part_path = trud_dir.join(format!("{}.part", release.archive_file_name));

                    let dl_counter = downloaded_bytes.clone();
                    let p_bar = progress.clone();
                    let total_dl = to_download_bytes;
                    let total_items = to_download_count;
                    let d_cnt = downloaded_count.clone();

                    let dl_res = fetcher.download_archive(
                        &release.download_url,
                        &part_path,
                        &move |n| {
                            let cur_bytes = dl_counter.fetch_add(n, Ordering::Relaxed) + n;
                            let cur_item = (d_cnt.load(Ordering::Relaxed) as usize + 1).min(total_items);
                            let elapsed = batch_start.elapsed().as_secs_f64();
                            let rate = if elapsed > 0.0 { cur_bytes as f64 / elapsed } else { 0.0 };
                            let eta = if rate > 0.0 && total_dl > cur_bytes {
                                Some(std::time::Duration::from_secs_f64((total_dl - cur_bytes) as f64 / rate))
                            } else {
                                None
                            };
                            p_bar.batch_bar(cur_item, total_items, cur_bytes, total_dl, Some(rate), eta);
                        },
                    );

                    let entry = match dl_res {
                        Err(e) => {
                            let _ = std::fs::remove_file(&part_path);
                            progress.remove_in_flight(&release.release_date);
                            failed_count.fetch_add(1, Ordering::Relaxed);
                            failures.lock().unwrap().push((release.release_date.clone(), e.to_string()));
                            SettledEntry {
                                line: format!("✖ {}  download failed after 2 attempts — {}", release.release_date, e),
                                outcome: ReleaseOutcome {
                                    trud_release_date: Some(release.release_date.clone()),
                                    trud_release_name: release.name.clone(),
                                    trud_release_file: Some(release.archive_file_name.clone()),
                                    trud_release_filesize_bytes: Some(release.archive_file_size),
                                    trud_release_sha256: None,
                                    trud_release_sha256_verified: None,
                                    status: "failed".to_string(),
                                    path: None,
                                    error: Some(e.to_string()),
                                },
                                is_success: false,
                            }
                        }
                        Ok(_) => {
                            let local_sha = match compute_file_sha256(&part_path) {
                                Ok(h) => h,
                                Err(e) => {
                                    let _ = std::fs::remove_file(&part_path);
                                    progress.remove_in_flight(&release.release_date);
                                    failed_count.fetch_add(1, Ordering::Relaxed);
                                    failures.lock().unwrap().push((release.release_date.clone(), e.to_string()));
                                    let entry = SettledEntry {
                                        line: format!("✖ {}  SHA-256 compute failed — {}", release.release_date, e),
                                        outcome: ReleaseOutcome {
                                            trud_release_date: Some(release.release_date.clone()),
                                            trud_release_name: release.name.clone(),
                                            trud_release_file: Some(release.archive_file_name.clone()),
                                            trud_release_filesize_bytes: Some(release.archive_file_size),
                                            trud_release_sha256: None,
                                            trud_release_sha256_verified: None,
                                            status: "failed".to_string(),
                                            path: None,
                                            error: Some(e.to_string()),
                                        },
                                        is_success: false,
                                    };
                                    let ready = coordinator.lock().unwrap().insert_and_drain(idx, entry);
                                    for e in ready {
                                        progress.settle(&e.line);
                                        if args.format.as_deref() == Some("ndjson") {
                                            emit_ndjson_outcome(&e.outcome);
                                        }
                                        if e.is_success {
                                            outcomes.lock().unwrap().push(e.outcome);
                                        }
                                    }
                                    continue;
                                }
                            };

                            if !local_sha.eq_ignore_ascii_case(&release.archive_file_sha256) {
                                let bad_path = mark_bad_sha_file(&part_path);
                                progress.remove_in_flight(&release.release_date);
                                failed_count.fetch_add(1, Ordering::Relaxed);
                                failures.lock().unwrap().push((release.release_date.clone(), "SHA-256 checksum mismatch".to_string()));
                                SettledEntry {
                                    line: format!(
                                        "✖ {}  SHA-256 checksum mismatch (renamed to {})",
                                        release.release_date,
                                        bad_path.file_name().and_then(|s| s.to_str()).unwrap_or("")
                                    ),
                                    outcome: ReleaseOutcome {
                                        trud_release_date: Some(release.release_date.clone()),
                                        trud_release_name: release.name.clone(),
                                        trud_release_file: Some(release.archive_file_name.clone()),
                                        trud_release_filesize_bytes: Some(release.archive_file_size),
                                        trud_release_sha256: Some(local_sha),
                                        trud_release_sha256_verified: Some(crate::provenance::TrudVerificationSource::Unverified),
                                        status: "failed".to_string(),
                                        path: None,
                                        error: Some("SHA-256 checksum mismatch".to_string()),
                                    },
                                    is_success: false,
                                }
                            } else {
                                let _ = std::fs::rename(&part_path, &dest_path);
                                let _ = write_provenance_json(&release_dir, release, &local_sha, true, &progress);
                                capture_attestations(
                                    &trud_dir,
                                    release,
                                    fetcher,
                                    raw_json.as_ref().as_deref(),
                                    args.api_key.as_deref(),
                                );

                                downloaded_count.fetch_add(1, Ordering::Relaxed);
                                progress.remove_in_flight(&release.release_date);

                                let mut newest = newest_downloaded_date.lock().unwrap();
                                if newest.is_none() || newest.as_ref().unwrap() < &release.release_date {
                                    *newest = Some(release.release_date.clone());
                                }

                                SettledEntry {
                                    line: format!(
                                        "✓ {}  {}  downloaded",
                                        release.release_date,
                                        format_size(release.archive_file_size)
                                    ),
                                    outcome: ReleaseOutcome {
                                        trud_release_date: Some(release.release_date.clone()),
                                        trud_release_name: release.name.clone(),
                                        trud_release_file: Some(release.archive_file_name.clone()),
                                        trud_release_filesize_bytes: Some(release.archive_file_size),
                                        trud_release_sha256: Some(local_sha),
                                        trud_release_sha256_verified: Some(crate::provenance::TrudVerificationSource::TrudApi),
                                        status: "downloaded".to_string(),
                                        path: Some(workspace_root.join("releases").join(&release.release_date).display().to_string()),
                                        error: None,
                                    },
                                    is_success: true,
                                }
                            }
                        }
                    };

                    let ready = coordinator.lock().unwrap().insert_and_drain(idx, entry);
                    for e in ready {
                        progress.settle(&e.line);
                        if args.format.as_deref() == Some("ndjson") {
                            emit_ndjson_outcome(&e.outcome);
                        }
                        if e.is_success {
                            outcomes.lock().unwrap().push(e.outcome);
                        }
                    }
                }
            });
        }
    });

    // Also emit cached ndjson if requested
    if args.format.as_deref() == Some("ndjson") {
        for r in &cached_releases {
            emit_ndjson_outcome(&ReleaseOutcome {
                trud_release_date: Some(r.release_date.clone()),
                trud_release_name: r.name.clone(),
                trud_release_file: Some(r.archive_file_name.clone()),
                trud_release_filesize_bytes: Some(r.archive_file_size),
                trud_release_sha256: Some(r.archive_file_sha256.clone()),
                trud_release_sha256_verified: Some(crate::provenance::TrudVerificationSource::TrudApi),
                status: "cached".to_string(),
                path: Some(workspace_root.join("releases").join(&r.release_date).display().to_string()),
                error: None,
            });
        }
    }

    let d_count = downloaded_count.load(Ordering::Relaxed);
    let f_count = failed_count.load(Ordering::Relaxed);
    let elapsed = batch_start.elapsed();

    // Total size landed (cached bytes + downloaded bytes)
    let successful_downloaded_bytes = outcomes.lock().unwrap().iter().filter_map(|o| o.trud_release_filesize_bytes).sum::<u64>();
    let landed_bytes: u64 = cached_bytes + successful_downloaded_bytes;

    progress.clear_live();
    let symbol = if d_count == 0 && f_count > 0 { "✖" } else { "✓" };
    let summary_line = format!(
        "{} {} downloaded · {} cached · {} failed · {} total  in {}",
        symbol,
        d_count,
        cached_count,
        f_count,
        format_size(landed_bytes),
        format_duration(elapsed)
    );
    progress.settle_summary(&summary_line);

    // If --all is passed, only update current link if a newer item was actually fetched.
    if d_count > 0 {
        let newest_dl = newest_downloaded_date.lock().unwrap().clone();
        if let Some(ref fetched_date) = newest_dl {
            let target_pin = match &existing_active_date {
                Some(existing) => {
                    if fetched_date > existing {
                        Some(fetched_date.clone())
                    } else {
                        None
                    }
                }
                None => {
                    let newest_cached = cached_releases.iter().map(|r| &r.release_date).max();
                    let best = newest_cached.map(|s| s.as_str()).into_iter().chain(Some(fetched_date.as_str())).max().unwrap();
                    Some(best.to_string())
                }
            };

            if let Some(ref target) = target_pin {
                let pin_moved = update_active_release_link_if_changed(&workspace_root, target)?;
                if pin_moved {
                    progress.settle_detail(&format!("current → releases/{}", target));
                }
            }
        }
    }

    let failure_list = failures.lock().unwrap().clone();
    if !failure_list.is_empty() {
        for (f_date, f_err) in failure_list {
            progress.error(&format!("{}  {}", f_date, f_err), &[&format!("Retry with: ods trud pull {}", f_date)]);
        }
        return Err(crate::commands::pull::AlreadyReported.into());
    }

    Ok(())
}

fn emit_ndjson_outcome(outcome: &ReleaseOutcome) {
    if let Ok(json) = serde_json::to_string(outcome) {
        println!("{}", json);
    }
}

fn mark_bad_sha_file(path: &Path) -> PathBuf {
    let bad_path = PathBuf::from(format!("{}.bad-sha", path.display()));
    let _ = std::fs::rename(path, &bad_path);
    bad_path
}

fn write_provenance_json(
    release_dir: &Path,
    release: &TrudReleaseItem,
    sha256: &str,
    force: bool,
    progress: &Progress,
) -> Result<()> {
    let prov_path = release_dir.join(crate::provenance::PROVENANCE_FILENAME);
    if prov_path.exists() && !force {
        return Ok(());
    }
    write_provenance_json_with_verification(
        release_dir,
        release,
        sha256,
        crate::provenance::TrudVerificationSource::TrudApi,
        progress,
    )
}

fn update_active_release_link_if_changed(workspace_root: &Path, release_date: &str) -> Result<bool> {
    let ws = Workspace::open_or_create(Some(workspace_root))?;
    if let Ok((active_date, _)) = ws.active_release() {
        if active_date == release_date {
            return Ok(false);
        }
    }
    ws.set_active(release_date)?;
    Ok(true)
}

pub fn fetch_trud_releases(api_key: &str, verbose: bool) -> Result<Vec<TrudReleaseItem>> {
    fetch_trud_releases_and_raw(api_key, verbose).map(|(rels, _)| rels)
}

pub fn fetch_trud_releases_and_raw(api_key: &str, verbose: bool) -> Result<(Vec<TrudReleaseItem>, String)> {
    let base_url = std::env::var("ODS_TRUD_API_URL")
        .unwrap_or_else(|_| "https://isd.digital.nhs.uk/trud/api/v1".to_string());
    let url = format!(
        "{}/keys/{}/items/{}/releases",
        base_url.trim_end_matches('/'),
        api_key,
        TRUD_ODS_ITEM_ID
    );

    if verbose {
        let safe_url = crate::provenance::sanitize_trud_url(&url, Some(api_key));
        eprintln!("[VERBOSE] Request URL: {}", safe_url);
    }

    let response = ureq::get(&url)
        .call()
        .context("Failed to connect to TRUD REST API")?;

    let status = response.status();
    let raw_body = response.into_string()
        .context("Failed to read response body from TRUD REST API")?;

    if verbose {
        let safe_body = crate::provenance::sanitize_trud_url(&raw_body, Some(api_key));
        eprintln!("[VERBOSE] HTTP Status: {}", status);
        eprintln!("[VERBOSE] Response Body:\n{}", safe_body);
    }

    let resp: TrudApiResponse = serde_json::from_str(&raw_body)
        .context("Failed to parse TRUD REST API response JSON")?;

    // Sanitize download URLs inside release items so API keys are never leaked
    let sanitized_releases = resp.releases.into_iter().map(|mut r| {
        r.download_url = crate::provenance::sanitize_trud_url(&r.download_url, Some(api_key));
        if let Some(ref u) = r.checksum_file_url {
            r.checksum_file_url = Some(crate::provenance::sanitize_trud_url(u, Some(api_key)));
        }
        if let Some(ref u) = r.signature_file_url {
            r.signature_file_url = Some(crate::provenance::sanitize_trud_url(u, Some(api_key)));
        }
        if let Some(ref u) = r.public_key_file_url {
            r.public_key_file_url = Some(crate::provenance::sanitize_trud_url(u, Some(api_key)));
        }
        r
    }).collect();

    Ok((sanitized_releases, raw_body))
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

fn run_local_archive(args: &Args, workspace_root: &Path, local_path: &Path, progress: &Progress) -> Result<()> {
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

    let file_name = zip_file.file_name()
        .and_then(|s| s.to_str())
        .context("Invalid archive filename UTF-8")?
        .to_string();
    let file_size = std::fs::metadata(&zip_file)?.len();

    let (release_date, trud_name) = parse_trud_filename(&file_name)?;

    let (dest_dir, trud_dir, is_workspace) = match &args.output {
        Some(out) => {
            let trud = out.join("trud");
            std::fs::create_dir_all(&trud)?;
            (out.clone(), trud, false)
        }
        None => {
            let ws = Workspace::open_or_create(Some(workspace_root))?;
            let release_dir = ws.prepare_release(&release_date)?;
            let trud = release_dir.join("trud");
            std::fs::create_dir_all(&trud)?;
            (release_dir, trud, true)
        }
    };

    let dest_path = trud_dir.join(&file_name);
    if dest_path != zip_file {
        std::fs::copy(&zip_file, &dest_path)?;
    }

    progress.step(&format!("{}   reading publication metadata…", release_date));
    let local_sha256 = compute_file_sha256(&dest_path)?;

    let release_item = TrudReleaseItem {
        id: TRUD_ODS_ITEM_ID.to_string(),
        name: trud_name,
        release_date: release_date.clone(),
        archive_file_name: file_name,
        archive_file_sha256: local_sha256.clone(),
        archive_file_size: file_size,
        download_url: format!("file://{}", zip_file.display()),
        ..Default::default()
    };

    write_provenance_json_with_verification(
        &dest_dir,
        &release_item,
        &local_sha256,
        crate::provenance::TrudVerificationSource::Unverified,
        progress,
    )?;
    let mut pin_moved = false;
    if is_workspace {
        pin_moved = update_active_release_link_if_changed(workspace_root, &release_date)?;
    }

    progress.settle(&format!(
        "* {}  {}  local archive, no TRUD checksum to compare",
        release_date,
        format_size(file_size)
    ));
    progress.settle_detail("attestations: none — no API response to fetch them from");
    if progress.caps().verbose {
        progress.settle_detail(&format!("SHA-256: {}", local_sha256));
    }
    if pin_moved {
        progress.settle_detail(&format!("current → releases/{}", release_date));
    }

    if args.format.as_deref() == Some("ndjson") {
        emit_ndjson_outcome(&ReleaseOutcome {
            trud_release_date: Some(release_date),
            trud_release_name: release_item.name,
            trud_release_file: Some(release_item.archive_file_name),
            trud_release_filesize_bytes: Some(file_size),
            trud_release_sha256: Some(local_sha256),
            trud_release_sha256_verified: Some(crate::provenance::TrudVerificationSource::Unverified),
            status: "local".to_string(),
            path: Some(dest_dir.display().to_string()),
            error: None,
        });
    }

    progress.finish("Done!");
    Ok(())
}

fn write_provenance_json_with_verification(
    release_dir: &Path,
    release: &TrudReleaseItem,
    _sha256: &str,
    verified: crate::provenance::TrudVerificationSource,
    _progress: &Progress,
) -> Result<()> {
    let prov_path = release_dir.join(crate::provenance::PROVENANCE_FILENAME);
    let trud_dir = release_dir.join("trud");
    let header = crate::ods_xml::extract_manifest_header(&trud_dir)?;

    let prov = OdsProvenance {
        schema: crate::provenance::PROVENANCE_SCHEMA_V1_URL.to_string(),
        trud_release_name: release.name.clone(),
        trud_release_date: Some(release.release_date.clone()),
        trud_release_sha256: Some(release.archive_file_sha256.clone()),
        trud_release_sha256_verified: Some(verified),
        trud_release_file: Some(release.archive_file_name.clone()),
        trud_release_filesize_bytes: Some(release.archive_file_size),
        publication_date: header.publication_date,
        publication_seq_num: header.publication_seq_num,
        publication_type: header.publication_type,
        publication_source: header.publication_source,
        publication_schema_version: header.publication_schema_version,
        publication_record_count: header.publication_record_count,
        tool_version: None,
        tool_git_sha: None,
        tool_git_dirty: None,
        dataset_version: None,
    };

    let json = serde_json::to_string_pretty(&prov)?;
    std::fs::write(&prov_path, json).context("Failed to write _provenance.json")?;
    Ok(())
}

pub fn run_local_archive_with_progress(
    args: &Args,
    workspace_root: &Path,
    local_path: &Path,
    progress: &Progress,
) -> Result<()> {
    run_local_archive(args, workspace_root, local_path, progress)
}

#[derive(Debug, Clone)]
pub struct AttestationCaptureResult {
    pub captured: Vec<&'static str>,
    pub missing_reasons: Vec<&'static str>,
}

impl AttestationCaptureResult {
    pub fn formatted_line(&self) -> String {
        format_attestations_line(&self.captured, &self.missing_reasons)
    }
}

pub fn capture_attestations<F: TrudFetcher>(
    trud_dir: &Path,
    release: &TrudReleaseItem,
    fetcher: &F,
    raw_json: Option<&str>,
    api_key: Option<&str>,
) -> AttestationCaptureResult {
    let _ = std::fs::create_dir_all(trud_dir);

    // 1. Write trud-releases-<release_date>.json
    let releases_json_path = trud_dir.join(format!("trud-releases-{}.json", release.release_date));
    let json_content = if let Some(raw) = raw_json {
        crate::provenance::sanitize_trud_url(raw, api_key)
    } else {
        let mut safe_release = release.clone();
        safe_release.download_url = crate::provenance::sanitize_trud_url(&safe_release.download_url, api_key);
        if let Some(ref u) = safe_release.checksum_file_url {
            safe_release.checksum_file_url = Some(crate::provenance::sanitize_trud_url(u, api_key));
        }
        if let Some(ref u) = safe_release.signature_file_url {
            safe_release.signature_file_url = Some(crate::provenance::sanitize_trud_url(u, api_key));
        }
        if let Some(ref u) = safe_release.public_key_file_url {
            safe_release.public_key_file_url = Some(crate::provenance::sanitize_trud_url(u, api_key));
        }
        let fallback_obj = serde_json::json!({
            "apiVersion": "1",
            "releases": [safe_release]
        });
        serde_json::to_string_pretty(&fallback_obj).unwrap_or_default()
    };
    let _ = std::fs::write(&releases_json_path, json_content);

    let mut captured = Vec::new();
    let mut missing_reasons = Vec::new();

    // 2. Checksum file
    if let Some(ref url) = release.checksum_file_url {
        let filename = release.checksum_file_name.as_deref().unwrap_or_else(|| {
            url.split('?').next().unwrap_or(url).rsplit('/').next().unwrap_or("checksum.xml")
        });
        let dest = trud_dir.join(filename);
        let res = if dest.exists() {
            Ok(())
        } else {
            fetcher.download_file(url, &dest)
        };
        if res.is_ok() {
            captured.push("checksum");
        } else {
            missing_reasons.push("checksum download failed");
        }
    } else {
        missing_reasons.push("checksum not offered for this release");
    }

    // 3. Signature file (stored as .xml.asc)
    if let Some(ref url) = release.signature_file_url {
        let url_fname = url.split('?').next().unwrap_or(url).rsplit('/').next().unwrap_or("");
        let filename = if url_fname.ends_with(".asc") {
            url_fname.to_string()
        } else if let Some(ref sig_name) = release.signature_file_name {
            if sig_name.ends_with(".sig") {
                format!("{}.asc", sig_name.strip_suffix(".sig").unwrap())
            } else {
                sig_name.clone()
            }
        } else {
            format!("{}.asc", release.checksum_file_name.as_deref().unwrap_or("checksum.xml"))
        };
        let dest = trud_dir.join(filename);
        let res = if dest.exists() {
            Ok(())
        } else {
            fetcher.download_file(url, &dest)
        };
        if res.is_ok() {
            captured.push("signature");
        } else {
            missing_reasons.push("signature download failed");
        }
    } else {
        missing_reasons.push("signature not offered for this release");
    }

    // 4. Public key file
    if let Some(ref url) = release.public_key_file_url {
        let filename = release.public_key_file_name.as_deref().unwrap_or_else(|| {
            url.split('?').next().unwrap_or(url).rsplit('/').next().unwrap_or("trud-public-key.pgp")
        });
        let dest = trud_dir.join(filename);
        let res = if dest.exists() {
            Ok(())
        } else {
            fetcher.download_file(url, &dest)
        };
        if res.is_ok() {
            captured.push("public key");
        } else {
            missing_reasons.push("public key download failed");
        }
    } else {
        missing_reasons.push("public key not offered for this release");
    }

    AttestationCaptureResult {
        captured,
        missing_reasons,
    }
}

pub fn format_attestations_line(captured: &[&str], missing_reasons: &[&str]) -> String {
    if missing_reasons.is_empty() {
        format!("attestations: {}", captured.join(", "))
    } else if captured.is_empty() {
        format!("attestations: none — {}", missing_reasons.join(", "))
    } else {
        format!("attestations: {} — {}", captured.join(", "), missing_reasons.join(", "))
    }
}

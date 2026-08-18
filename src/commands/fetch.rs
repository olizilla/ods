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

use crate::progress::{format_duration, format_size, Progress, ProgressCaps};
use crate::provenance::{compute_file_sha256, OdsProvenance};
use crate::workspace::{find_workspace_root, prepare_release_dir, set_active_release, DEFAULT_WORKSPACE_DIR};

pub const TRUD_ODS_ITEM_ID: &str = "341";

#[derive(Parser, Debug, Default, Clone)]
pub struct Args {
    /// Target TRUD release date in YYYY-MM-DD format (defaults to latest available)
    pub release_date: Option<String>,

    /// List available TRUD release versions
    #[arg(long, short = 'l')]
    pub list: bool,

    /// Number of releases to display in listing
    #[arg(long, default_value_t = 10)]
    pub limit: usize,

    /// Fetch all available TRUD releases
    #[arg(long)]
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

    /// TRUD API Key (defaults to $NHS_TRUD_API_KEY if omitted)
    #[arg(long, env = "NHS_TRUD_API_KEY")]
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

    /// Print verbose output (URL fetched and raw API response text before deserialization)
    #[arg(long, short = 'v')]
    pub verbose: bool,
}

#[derive(Debug, Deserialize, Clone)]
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
    pub trud_release_sha256_verified: Option<bool>,
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
    fn download_archive(
        &self,
        url: &str,
        dest_path: &Path,
        on_bytes: &(dyn Fn(u64) + Send + Sync),
    ) -> Result<()>;
}

pub struct UreqTrudFetcher {
    pub api_key: String,
    pub agent: ureq::Agent,
    pub verbose: bool,
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
        }
    }
}

impl TrudFetcher for UreqTrudFetcher {
    fn fetch_releases(&self) -> Result<Vec<TrudReleaseItem>> {
        fetch_trud_releases(&self.api_key, self.verbose)
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
}

pub fn run(args: Args) -> Result<()> {
    let progress = Progress::stderr(ProgressCaps::detect(args.quiet, args.verbose, args.no_progress));
    let workspace_root = find_workspace_root()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_WORKSPACE_DIR));

    if let Some(ref local_path) = args.local_archive {
        let resp_json_path = if local_path.is_dir() {
            local_path.join("response.json")
        } else {
            local_path.with_file_name("response.json")
        };
        if resp_json_path.exists() {
            let text = std::fs::read_to_string(&resp_json_path)?;
            let resp: TrudApiResponse = serde_json::from_str(&text)?;
            if args.list {
                return display_trud_releases(&args, &workspace_root, resp.releases, &progress, &mut std::io::stdout());
            }
        }
        return run_local_archive(&args, &workspace_root, local_path, &progress);
    }

    let api_key = match args.api_key {
        Some(ref key) if !key.trim().is_empty() => key.clone(),
        _ => {
            anyhow::bail!(
                "✖ Missing API Key\n  Please set $NHS_TRUD_API_KEY environment variable or pass --api-key <KEY>.\n  See: https://isd.digital.nhs.uk/trud/user/authenticated/group/0/pack/341/subpack/160/releases"
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
    if args.list {
        progress.step("Querying NHS TRUD…");
        let releases = match fetcher.fetch_releases() {
            Ok(r) => r,
            Err(e) => {
                progress.error("Failed to query NHS TRUD API", &[&e.to_string()]);
                return Err(crate::commands::pull::AlreadyReported.into());
            }
        };
        return display_trud_releases(&args, workspace_root, releases, progress, &mut std::io::stdout());
    }

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

fn display_trud_releases<W: std::io::Write>(
    args: &Args,
    workspace_root: &Path,
    releases: Vec<TrudReleaseItem>,
    progress: &Progress,
    stdout: &mut W,
) -> Result<()> {
    let (active_date, _) = crate::workspace::get_active_release(workspace_root).unwrap_or_default();

    let total_count = releases.len();
    let display_limit = if args.limit > 0 { args.limit } else { 10 };
    let to_display: Vec<&TrudReleaseItem> = releases.iter().take(display_limit).collect();

    if args.format.as_deref() == Some("json") {
        let json_items: Vec<ReleaseListItemJson> = to_display.iter().map(|r| {
            let is_active = !active_date.is_empty() && active_date == r.release_date;
            let release_dir = workspace_root.join("releases").join(&r.release_date).join("trud");
            let archive_file = release_dir.join(&r.archive_file_name);
            let is_local = archive_file.exists();

            let status_str = if is_active {
                "active (local)"
            } else if is_local {
                "local"
            } else {
                "remote"
            };

            ReleaseListItemJson {
                date: r.release_date.clone(),
                size_bytes: r.archive_file_size,
                status: status_str.to_string(),
            }
        }).collect();

        serde_json::to_writer_pretty(&mut *stdout, &json_items)?;
        writeln!(stdout)?;
        return Ok(());
    }

    let mut has_active = false;
    let mut has_local = false;
    let mut has_remote = false;

    progress.clear_live();

    for r in &to_display {
        let is_active = !active_date.is_empty() && active_date == r.release_date;
        let release_dir = workspace_root.join("releases").join(&r.release_date).join("trud");
        let archive_file = release_dir.join(&r.archive_file_name);
        let is_local = archive_file.exists();

        let (status_str, marker) = if is_active {
            has_active = true;
            ("● active (local)", "●")
        } else if is_local {
            has_local = true;
            ("○ local", "○")
        } else {
            has_remote = true;
            ("○ remote", "○")
        };
        let _ = marker;

        writeln!(
            stdout,
            "  {:10}  {:6}  {}",
            r.release_date,
            format_size(r.archive_file_size),
            status_str
        )?;
    }

    if total_count > display_limit {
        eprintln!(
            "\nShowing {} of {} TRUD releases. Use --limit <N> to view more.",
            display_limit, total_count
        );
    }

    if progress.caps().is_tty {
        eprintln!("\nLegend:");
        if has_active {
            eprintln!("  ● active (local)  Active release pin (./ods_data/current)");
        }
        if has_local {
            eprintln!("  ○ local           Cached locally in ./ods_data/releases/");
        }
        if has_remote {
            eprintln!("  ○ remote          Available for pull from TRUD");
        }
        eprintln!("\nTo pull a specific release, run: ods trud pull <YYYY-MM-DD>");
    }

    Ok(())
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
            let release_dir = prepare_release_dir(workspace_root, &target_release.release_date)?;
            let trud = release_dir.join("trud");
            std::fs::create_dir_all(&trud)?;
            (release_dir, trud, true)
        }
    };

    let dest_path = trud_dir.join(&target_release.archive_file_name);

    if dest_path.exists() && !args.force {
        let local_sha256 = compute_file_sha256(&dest_path)?;
        if local_sha256.eq_ignore_ascii_case(&target_release.archive_file_sha256) {
            write_provenance_json(&dest_dir, &target_release, &local_sha256, false, progress)?;
            let mut pin_moved = false;
            if is_workspace {
                pin_moved = update_active_release_link_if_changed(workspace_root, &target_release.release_date)?;
            }

            progress.settle(&format!(
                "* {}  {}  cached, SHA-256 verified by TRUD API",
                target_release.release_date,
                format_size(target_release.archive_file_size)
            ));
            if pin_moved {
                progress.settle_detail(&format!("current → releases/{}", target_release.release_date));
            }

            if args.format.as_deref() == Some("ndjson") {
                emit_ndjson_outcome(&ReleaseOutcome {
                    trud_release_date: Some(target_release.release_date.clone()),
                    trud_release_name: target_release.name.clone(),
                    trud_release_file: Some(target_release.archive_file_name.clone()),
                    trud_release_filesize_bytes: Some(target_release.archive_file_size),
                    trud_release_sha256: Some(target_release.archive_file_sha256.clone()),
                    trud_release_sha256_verified: Some(true),
                    status: "cached".to_string(),
                    path: Some(format!("ods_data/releases/{}", target_release.release_date)),
                    error: None,
                });
            }

            progress.finish("Done!");
            return Ok(());
        } else {
            let bad_path = mark_bad_sha_file(&dest_path);
            if progress.caps().verbose {
                eprintln!("✖ SHA-256 Checksum Mismatch on cached file! Renamed to {}", bad_path.display());
            }
        }
    }

    let part_path = trud_dir.join(format!("{}.part", target_release.archive_file_name));
    let downloaded_bytes = Arc::new(AtomicU64::new(0));
    let start_time = Instant::now();

    let dl_counter = downloaded_bytes.clone();
    let p = progress.clone();
    let date = target_release.release_date.clone();
    let total_size = target_release.archive_file_size;

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
            p.bar(&format!("{}   downloading", date), cur, total_size, Some(rate), eta);
        },
    );

    if let Err(e) = res {
        let _ = std::fs::remove_file(&part_path);
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

    progress.settle(&format!(
        "✓ {}  {}  SHA-256 verified by TRUD API",
        target_release.release_date,
        format_size(target_release.archive_file_size)
    ));
    if progress.caps().verbose {
        progress.settle_detail(&format!("SHA-256: {}", local_sha256));
    }
    if pin_moved {
        progress.settle_detail(&format!("current → releases/{}", target_release.release_date));
    }

    if args.format.as_deref() == Some("ndjson") {
        emit_ndjson_outcome(&ReleaseOutcome {
            trud_release_date: Some(target_release.release_date.clone()),
            trud_release_name: target_release.name.clone(),
            trud_release_file: Some(target_release.archive_file_name.clone()),
            trud_release_filesize_bytes: Some(target_release.archive_file_size),
            trud_release_sha256: Some(local_sha256),
            trud_release_sha256_verified: Some(true),
            status: "downloaded".to_string(),
            path: Some(format!("ods_data/releases/{}", target_release.release_date)),
            error: None,
        });
    }

    progress.finish("Done!");
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
                    trud_release_sha256_verified: Some(true),
                    status: "cached".to_string(),
                    path: Some(format!("ods_data/releases/{}", r.release_date)),
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
    let existing_active_date = crate::workspace::get_active_release(workspace_root).ok().map(|(d, _)| d);
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

                    let release_dir = match prepare_release_dir(&ws_root, &release.release_date) {
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
                                        trud_release_sha256_verified: Some(false),
                                        status: "failed".to_string(),
                                        path: None,
                                        error: Some("SHA-256 checksum mismatch".to_string()),
                                    },
                                    is_success: false,
                                }
                            } else {
                                let _ = std::fs::rename(&part_path, &dest_path);
                                let _ = write_provenance_json(&release_dir, release, &local_sha, true, &progress);

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
                                        trud_release_sha256_verified: Some(true),
                                        status: "downloaded".to_string(),
                                        path: Some(format!("ods_data/releases/{}", release.release_date)),
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
                trud_release_sha256_verified: Some(true),
                status: "cached".to_string(),
                path: Some(format!("ods_data/releases/{}", r.release_date)),
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
    write_provenance_json_with_verification(release_dir, release, sha256, true, progress)
}

fn update_active_release_link_if_changed(workspace_root: &Path, release_date: &str) -> Result<bool> {
    if let Ok((active_date, _)) = crate::workspace::get_active_release(workspace_root) {
        if active_date == release_date {
            return Ok(false);
        }
    }
    set_active_release(workspace_root, release_date)?;
    Ok(true)
}

pub fn fetch_trud_releases(api_key: &str, verbose: bool) -> Result<Vec<TrudReleaseItem>> {
    let url = format!(
        "https://isd.digital.nhs.uk/trud/api/v1/keys/{}/items/{}/releases",
        api_key, TRUD_ODS_ITEM_ID
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
        r
    }).collect();

    Ok(sanitized_releases)
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
            let release_dir = prepare_release_dir(workspace_root, &release_date)?;
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
    };

    write_provenance_json_with_verification(&dest_dir, &release_item, &local_sha256, false, progress)?;
    let mut pin_moved = false;
    if is_workspace {
        pin_moved = update_active_release_link_if_changed(workspace_root, &release_date)?;
    }

    progress.settle(&format!(
        "* {}  {}  local archive, no TRUD checksum to compare",
        release_date,
        format_size(file_size)
    ));
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
            trud_release_sha256_verified: Some(false),
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
    verified: bool,
    _progress: &Progress,
) -> Result<()> {
    let prov_path = release_dir.join(crate::provenance::PROVENANCE_FILENAME);
    let trud_dir = release_dir.join("trud");
    let header = crate::commands::ndjson::extract_manifest_header(&trud_dir)?;

    let prov = OdsProvenance {
        type_tag: crate::provenance::NDJSON_TYPE_TAG.to_string(),
        trud_release_name: release.name.clone(),
        trud_release_date: Some(release.release_date.clone()),
        trud_release_url: Some(release.download_url.clone()),
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
        primary_role_scope: header.primary_role_scope,
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        tool_parquet_version: Some(env!("ODS_TOOL_PARQUET_VERSION").to_string()),
        tool_arrow_version: Some(env!("ODS_TOOL_ARROW_VERSION").to_string()),
        tool_zstd_level: Some(3),
        dataset_doi: None,
        derived_artifacts: None,
    };

    let json = serde_json::to_string_pretty(&prov)?;
    std::fs::write(&prov_path, json).context("Failed to write _provenance.json")?;
    Ok(())
}

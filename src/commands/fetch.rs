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
use crate::provenance::compute_file_sha256;
use crate::workspace::Workspace;

pub const TRUD_ODS_ITEM_ID: &str = "341";

#[derive(Parser, Debug, Default, Clone)]
pub struct Args {
    /// Target TRUD release date in YYYY-MM-DD format (defaults to latest available)
    pub release_date: Option<String>,

    /// Fetch all available TRUD releases
    #[arg(long, hide = true)]
    pub all: bool,

    /// Re-fetch NHS's checksum, signature and key, and download the archive again only if it's
    /// missing or doesn't match TRUD's hash
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

    /// Workspace directory (default: the one found from the current directory)
    #[arg(long, short = 'w')]
    pub workspace: Option<PathBuf>,

    /// Print verbose output (URL fetched and raw API response text before deserialization)
    #[arg(long, short = 'v')]
    pub verbose: bool,

    /// Read the release index from this path or URL instead of the network
    #[arg(long, hide = true)]
    pub index: Option<String>,
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

    // `-o` means only there: the archive lands under it and nothing else is created
    // or resolved. Without `-o`, a workspace is opened (or made) as before.
    let workspace_root: PathBuf = if args.output.is_some() {
        Workspace::open(args.workspace.as_deref())
            .map(|ws| ws.root().to_path_buf())
            .unwrap_or_else(|_| {
                args.workspace.clone().unwrap_or_else(|| PathBuf::from(crate::workspace::DEFAULT_WORKSPACE_DIR))
            })
    } else {
        Workspace::open_or_create(args.workspace.as_deref())?.root().to_path_buf()
    };

    let trud_fetcher = args.api_key.as_ref().filter(|k| !k.trim().is_empty()).map(|k| UreqTrudFetcher::new(k, args.verbose));
    let oci_fetcher = crate::commands::pull::HttpOciFetcher;

    if let Some(ref local_path) = args.local_archive {
        return run_local_archive_with_fetchers(&args, &workspace_root, local_path, &progress, trud_fetcher.as_ref(), Some(&oci_fetcher));
    }

    if let Some(ref verify_path) = args.verify_only {
        return run_verify_only_with_fetchers(&args, &workspace_root, verify_path, &progress, trud_fetcher.as_ref(), Some(&oci_fetcher));
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
    let oci_fetcher = crate::commands::pull::HttpOciFetcher;
    if let Some(ref local_path) = args.local_archive {
        return run_local_archive_with_fetchers(&args, workspace_root, local_path, progress, Some(fetcher), Some(&oci_fetcher));
    }

    if let Some(ref verify_path) = args.verify_only {
        return run_verify_only_with_fetchers(&args, workspace_root, verify_path, progress, Some(fetcher), Some(&oci_fetcher));
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

fn pull_single_release<F: TrudFetcher>(
    args: &Args,
    workspace_root: &Path,
    fetcher: &F,
    target_release: TrudReleaseItem,
    progress: &Progress,
) -> Result<()> {

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

    if dest_path.exists() {
        let local_sha256 = compute_file_sha256(&dest_path)?;
        if local_sha256.eq_ignore_ascii_case(&target_release.archive_file_sha256) {
            // The zip is the expensive part, and the one thing TRUD vouches for: keep it, and
            // make the rest of the directory whole from it.
            let healed = match heal_release_dir(&dest_dir, &trud_dir, &target_release, fetcher, args.force) {
                Ok(healed) => healed,
                Err(e) => {
                    if args.format.as_deref() == Some("ndjson") {
                        emit_ndjson_outcome(&held_zip_outcome(
                            &target_release,
                            &dest_dir,
                            &HashMap::new(),
                            &[(target_release.release_date.clone(), e.to_string())],
                        ));
                    }
                    return Err(e);
                }
            };
            let mut pin_moved = false;
            if is_workspace {
                pin_moved = update_active_release_link_if_changed(workspace_root, &target_release.release_date)?;
            }

            let file_count = 2 + healed.attestations.captured.len();
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
            let state = match healed.describe(args.force) {
                Some(what) => ReleaseBlockState::Healed { what },
                None => ReleaseBlockState::Cached,
            };
            let lines = render_release_block(&ReleaseBlockParams {
                date: &target_release.release_date,
                archive_size: target_release.archive_file_size,
                file_count,
                state: &state,
                dataset: None,
                verified: "sha256 from TRUD API",
                linked: link,
                from: None,
                hash: hash_opt,
                hash_label: "sha256",
                color: progress.caps().is_tty && !progress.caps().no_color,
            });
            progress.finish_block(&lines);

            let failed_reasons: Vec<_> = healed
                .attestations
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
                    trud_release_filesize_bytes: Some(target_release.archive_file_size),
                    trud_release_sha256: Some(target_release.archive_file_sha256.clone()),
                    trud_release_sha256_verified: Some(crate::provenance::TrudVerificationSource::TrudApi),
                    status: healed.status(args.force).to_string(),
                    path: Some(dest_dir.display().to_string()),
                    error: None,
                });
            }

            return Ok(());
        } else if !args.force {
            progress.error(
                &format!("SHA-256 mismatch for {}", target_release.release_date),
                &[
                    &format!("The archive held: {}", local_sha256),
                    &format!("TRUD's hash:      {}", target_release.archive_file_sha256),
                    &format!("Download it again with: ods trud pull {} --force", target_release.release_date),
                ],
            );
            return Err(crate::commands::pull::AlreadyReported.into());
        } else {
            // `--force`: the zip is downloaded again below, and this one is kept aside.
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
                dataset: None,
                verified: "-",
                linked: link,
                from: None,
                hash: None,
                hash_label: "sha256",
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

    write_provenance_json(&dest_dir, &target_release, true)?;
    let mut pin_moved = false;
    if is_workspace {
        pin_moved = update_active_release_link_if_changed(workspace_root, &target_release.release_date)?;
    }

    let elapsed = start_time.elapsed();
    let att_result = capture_attestations(&trud_dir, &target_release, fetcher, args.force);
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
        dataset: None,
        verified: "sha256 from TRUD API",
        linked: link,
        from: None,
        hash: hash_opt,
        hash_label: "sha256",
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

    // Plan Phase: what each release directory already holds, checked with parallel hashing
    enum Held {
        Absent,
        Matches,
        Differs(String),
    }

    let mut to_download: Vec<TrudReleaseItem> = Vec::new();
    let mut cached_releases: Vec<TrudReleaseItem> = Vec::new();
    // A zip that isn't TRUD's, without `--force`: named at the end, and the batch carries on
    let mut refused: Vec<(String, String)> = Vec::new();
    let checked_count = Arc::new(AtomicU64::new(0));

    use rayon::prelude::*;
    let checked = checked_count.clone();
    let p_clone = progress.clone();
    let ws_root = workspace_root.to_path_buf();

    progress.bar("Checking local archives…", 0, total_count as u64, None, None);

    let check_results: Vec<(TrudReleaseItem, Held)> = releases
        .par_iter()
        .map(|release| {
            let rel_dir = ws_root.join("releases").join(&release.release_date);
            let dest_path = rel_dir.join("trud").join(&release.archive_file_name);
            let held = if dest_path.exists() {
                match compute_file_sha256(&dest_path) {
                    Ok(hash) if hash.eq_ignore_ascii_case(&release.archive_file_sha256) => Held::Matches,
                    Ok(hash) => Held::Differs(hash),
                    Err(e) => Held::Differs(format!("unreadable ({e})")),
                }
            } else {
                Held::Absent
            };
            let cur = checked.fetch_add(1, Ordering::Relaxed) + 1;
            p_clone.bar("Checking local archives…", cur, total_count as u64, None, None);
            (release.clone(), held)
        })
        .collect();

    for (rel, held) in check_results {
        match held {
            Held::Matches => cached_releases.push(rel),
            Held::Absent => to_download.push(rel),
            // `--force` downloads the zip again
            Held::Differs(_) if args.force => to_download.push(rel),
            Held::Differs(local) => {
                let message = format!(
                    "SHA-256 mismatch: the archive held is {}, TRUD's is {}",
                    local, rel.archive_file_sha256
                );
                refused.push((rel.release_date.clone(), message));
            }
        }
    }

    let num_jobs = args.jobs.clamp(1, 8);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_jobs)
        .build()
        .context("Failed to build rayon thread pool")?;

    // Repair Phase: every zip already held is kept. What's missing beside it is fetched from
    // NHS, and `--force` fetches NHS's files again. A zip is never downloaded here.
    let heal_results: Vec<(String, Result<Healed>)> = pool.install(|| {
        cached_releases
            .par_iter()
            .map(|release| {
                let release_dir = workspace_root.join("releases").join(&release.release_date);
                let trud_dir = release_dir.join("trud");
                (
                    release.release_date.clone(),
                    heal_release_dir(&release_dir, &trud_dir, release, fetcher, args.force),
                )
            })
            .collect()
    });
    let mut heal_failures: Vec<(String, String)> = Vec::new();
    let mut heal_status: HashMap<String, &'static str> = HashMap::new();
    progress.clear_live();
    for (date, result) in heal_results {
        match result {
            Ok(healed) => {
                heal_status.insert(date.clone(), healed.status(args.force));
                if let Some(what) = healed.describe(args.force) {
                    progress.settle(&format!("✓ {}  {}", date, what));
                }
            }
            Err(e) => heal_failures.push((date, e.to_string())),
        }
    }

    let to_download_bytes: u64 = to_download.iter().map(|r| r.archive_file_size).sum();
    // A release whose repair failed isn't counted as cached
    let cached_count = cached_releases.len() - heal_failures.len();
    let to_download_count = to_download.len();

    if to_download_count == 0 {
        progress.clear_live();
        progress.settle(&format!(
            "✓ {} cached, SHA-256 verified by TRUD API · nothing to download",
            cached_count
        ));
        if args.format.as_deref() == Some("ndjson") {
            for r in &cached_releases {
                emit_ndjson_outcome(&held_zip_outcome(r, &workspace_root.join("releases").join(&r.release_date), &heal_status, &heal_failures));
            }
        }
        if refused.is_empty() && heal_failures.is_empty() {
            return Ok(());
        }
        return report_batch_failures(progress, Vec::new(), &refused, &heal_failures);
    }

    progress.clear_live();
    progress.settle(&format!(
        "✓ {} cached · {} to download · {}",
        cached_count,
        to_download_count,
        format_size(to_download_bytes)
    ));

    // Execute Phase: download missing releases
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
    let cached_bytes: u64 = cached_releases
        .iter()
        .filter(|r| !heal_failures.iter().any(|(date, _)| *date == r.release_date))
        .map(|r| r.archive_file_size)
        .sum();

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
                                let _ = write_provenance_json(&release_dir, release, true);
                                capture_attestations(&trud_dir, release, fetcher, args.force);

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
            emit_ndjson_outcome(&held_zip_outcome(r, &workspace_root.join("releases").join(&r.release_date), &heal_status, &heal_failures));
        }
    }

    let d_count = downloaded_count.load(Ordering::Relaxed);
    let f_count = failed_count.load(Ordering::Relaxed) + (refused.len() + heal_failures.len()) as u64;
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
    if !failure_list.is_empty() || !refused.is_empty() || !heal_failures.is_empty() {
        return report_batch_failures(progress, failure_list, &refused, &heal_failures);
    }

    Ok(())
}

/// Prints each failure of a batch as a block with its retry command, and exits 1.
fn report_batch_failures(
    progress: &Progress,
    download_failures: Vec<(String, String)>,
    refused: &[(String, String)],
    heal_failures: &[(String, String)],
) -> Result<()> {
    for (date, err) in download_failures.iter().chain(heal_failures.iter()) {
        progress.error(&format!("{}  {}", date, err), &[&format!("Retry with: ods trud pull {}", date)]);
    }
    for (date, err) in refused {
        progress.error(&format!("{}  {}", date, err), &[&format!("Retry with: ods trud pull {} --force", date)]);
    }
    Err(crate::commands::pull::AlreadyReported.into())
}

/// The ndjson outcome of a release whose zip was already held: what its repair did (`cached`,
/// `repaired` or `refreshed`), or `failed` with the error, as a failed download reports.
fn held_zip_outcome(
    release: &TrudReleaseItem,
    release_dir: &Path,
    heal_status: &HashMap<String, &'static str>,
    heal_failures: &[(String, String)],
) -> ReleaseOutcome {
    let failure = heal_failures.iter().find(|(date, _)| *date == release.release_date);
    ReleaseOutcome {
        trud_release_date: Some(release.release_date.clone()),
        trud_release_filesize_bytes: Some(release.archive_file_size),
        trud_release_sha256: Some(release.archive_file_sha256.clone()),
        trud_release_sha256_verified: Some(crate::provenance::TrudVerificationSource::TrudApi),
        status: match failure {
            Some(_) => "failed",
            None => heal_status.get(&release.release_date).copied().unwrap_or("cached"),
        }
        .to_string(),
        path: Some(release_dir.display().to_string()),
        error: failure.map(|(_, e)| e.clone()),
    }
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

/// Writes `_provenance.json` when it is missing, unreadable (an older format, say) or `force`d.
/// Returns whether it wrote one.
fn write_provenance_json(release_dir: &Path, release: &TrudReleaseItem, force: bool) -> Result<bool> {
    let prov_path = release_dir.join(crate::provenance::PROVENANCE_FILENAME);
    if prov_path.exists()
        && !force
        && matches!(
            crate::provenance::OdsProvenance::load_from_file(&prov_path),
            crate::provenance::ProvenanceLoad::Read(..)
        )
    {
        return Ok(false);
    }
    write_provenance_file(release_dir, release)?;
    Ok(true)
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

fn resolve_zip_file(local_path: &Path) -> Result<PathBuf> {
    if local_path.is_dir() {
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
        zip.context(format!("No .zip file found in local archive dir {}", local_path.display()))
    } else {
        Ok(local_path.to_path_buf())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveVerificationOutcome {
    VerifiedPublished {
        expected_sha256: String,
        file_size: u64,
    },
    VerifiedTrud {
        expected_sha256: String,
        file_size: u64,
    },
    Unverified,
    TrudApiError(String),
    Mismatch {
        source_name: &'static str,
        expected_sha256: String,
        actual_sha256: String,
    },
}

impl ArchiveVerificationOutcome {
    pub fn verification_source(&self) -> Option<crate::provenance::TrudVerificationSource> {
        match self {
            Self::VerifiedPublished { .. } => Some(crate::provenance::TrudVerificationSource::PublishedRelease),
            Self::VerifiedTrud { .. } => Some(crate::provenance::TrudVerificationSource::TrudApi),
            Self::Unverified | Self::TrudApiError(_) => Some(crate::provenance::TrudVerificationSource::Unverified),
            Self::Mismatch { .. } => None,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn verify_archive<F: TrudFetcher, OF: crate::commands::pull::OciBlobFetcher>(
    release_date: &str,
    local_sha256: &str,
    workspace_root: &Path,
    index_override: Option<&str>,
    allow_network: bool,
    persist_index: bool,
    trud_fetcher: Option<&F>,
    oci_fetcher: Option<&OF>,
) -> Result<ArchiveVerificationOutcome> {
    let default_oci = crate::commands::pull::HttpOciFetcher;
    let oci: &dyn crate::commands::pull::OciBlobFetcher = match oci_fetcher {
        Some(f) => f as &dyn crate::commands::pull::OciBlobFetcher,
        None => &default_oci,
    };

    // 1. Release index lookup
    let index_res = if allow_network {
        if let Some(override_path) = index_override {
            crate::commands::pull::resolve_index(
                workspace_root,
                Some(override_path),
                true,
                persist_index,
                oci,
            )
            .map(|(idx, _)| Some(idx))
        } else {
            // First check cached + baked copy offline
            let (offline_idx, _) = crate::commands::pull::resolve_index(
                workspace_root,
                None,
                false,
                false,
                oci,
            )?;
            if offline_idx.releases.iter().any(|r| r.trud_release_date == release_date) {
                Ok(Some(offline_idx))
            } else {
                // Fetch published index
                match crate::commands::pull::resolve_index(
                    workspace_root,
                    None,
                    true,
                    persist_index,
                    oci,
                ) {
                    Ok((idx, _)) => Ok(Some(idx)),
                    Err(_) => Ok(Some(offline_idx)),
                }
            }
        }
    } else {
        // Offline only: workspace cache then baked index
        crate::commands::pull::resolve_index(
            workspace_root,
            None,
            false,
            false,
            oci,
        )
        .map(|(idx, _)| Some(idx))
    };

    if let Ok(Some(index)) = index_res {
        if let Some(rel) = index.releases.iter().find(|r| r.trud_release_date == release_date) {
            if local_sha256.eq_ignore_ascii_case(&rel.trud_release_sha256) {
                return Ok(ArchiveVerificationOutcome::VerifiedPublished {
                    expected_sha256: rel.trud_release_sha256.clone(),
                    file_size: rel.trud_release_filesize_bytes,
                });
            } else {
                return Ok(ArchiveVerificationOutcome::Mismatch {
                    source_name: "Index",
                    expected_sha256: rel.trud_release_sha256.clone(),
                    actual_sha256: local_sha256.to_string(),
                });
            }
        }
    } else if let Err(e) = index_res {
        return Err(e);
    }

    // 2. TRUD API lookup
    if allow_network {
        if let Some(fetcher) = trud_fetcher {
            match fetcher.fetch_releases() {
                Ok(releases) => {
                    if let Some(trud_rel) = releases.into_iter().find(|r| r.release_date == release_date) {
                        if local_sha256.eq_ignore_ascii_case(&trud_rel.archive_file_sha256) {
                            return Ok(ArchiveVerificationOutcome::VerifiedTrud {
                                expected_sha256: trud_rel.archive_file_sha256,
                                file_size: trud_rel.archive_file_size,
                            });
                        } else {
                            return Ok(ArchiveVerificationOutcome::Mismatch {
                                source_name: "TRUD",
                                expected_sha256: trud_rel.archive_file_sha256,
                                actual_sha256: local_sha256.to_string(),
                            });
                        }
                    }
                }
                Err(e) => {
                    return Ok(ArchiveVerificationOutcome::TrudApiError(e.to_string()));
                }
            }
        }
    }

    // 3. Neither
    Ok(ArchiveVerificationOutcome::Unverified)
}

#[derive(Debug, Clone)]
pub struct MatchedTrudArchive {
    pub release_date: String,
    pub trud_release_sha256: String,
    pub trud_release_filesize_bytes: u64,
    pub trud_name: Option<String>,
    pub source: crate::provenance::TrudVerificationSource,
}

pub struct MatchArchiveOptions<'a, F, OF> {
    pub index_override: Option<&'a str>,
    pub allow_network: bool,
    pub persist_index: bool,
    pub trud_fetcher: Option<&'a F>,
    pub oci_fetcher: Option<&'a OF>,
    pub progress: Option<&'a Progress>,
}

pub fn match_archive_by_hash<F: TrudFetcher, OF: crate::commands::pull::OciBlobFetcher>(
    local_sha256: &str,
    workspace_root: &Path,
    opts: MatchArchiveOptions<'_, F, OF>,
) -> Result<Option<MatchedTrudArchive>> {
    let default_oci = crate::commands::pull::HttpOciFetcher;
    let oci: &dyn crate::commands::pull::OciBlobFetcher = match opts.oci_fetcher {
        Some(f) => f as &dyn crate::commands::pull::OciBlobFetcher,
        None => &default_oci,
    };

    // 1. Release index lookup
    let index_res = if opts.allow_network {
        if let Some(override_path) = opts.index_override {
            crate::commands::pull::resolve_index(
                workspace_root,
                Some(override_path),
                true,
                opts.persist_index,
                oci,
            )
            .map(|(idx, _)| Some(idx))
        } else {
            // First check cached + baked copy offline
            let (offline_idx, _) =
                crate::commands::pull::resolve_index(workspace_root, None, false, false, oci)?;
            if offline_idx
                .releases
                .iter()
                .any(|r| r.trud_release_sha256.eq_ignore_ascii_case(local_sha256))
            {
                Ok(Some(offline_idx))
            } else {
                // Fetch published index
                match crate::commands::pull::resolve_index(
                    workspace_root,
                    None,
                    true,
                    opts.persist_index,
                    oci,
                ) {
                    Ok((idx, _)) => Ok(Some(idx)),
                    Err(_) => Ok(Some(offline_idx)),
                }
            }
        }
    } else {
        // Offline only: index_override if provided, else workspace cache then baked index
        crate::commands::pull::resolve_index(workspace_root, opts.index_override, false, false, oci)
            .map(|(idx, _)| Some(idx))
    };

    if let Ok(Some(index)) = index_res {
        if let Some(rel) = index
            .releases
            .iter()
            .find(|r| r.trud_release_sha256.eq_ignore_ascii_case(local_sha256))
        {
            return Ok(Some(MatchedTrudArchive {
                release_date: rel.trud_release_date.clone(),
                trud_release_sha256: rel.trud_release_sha256.clone(),
                trud_release_filesize_bytes: rel.trud_release_filesize_bytes,
                trud_name: None,
                source: crate::provenance::TrudVerificationSource::PublishedRelease,
            }));
        }
    } else if let Err(e) = index_res {
        return Err(e);
    }

    // 2. TRUD API lookup
    if let Some(fetcher) = opts.trud_fetcher {
        match fetcher.fetch_releases() {
            Ok(releases) => {
                if let Some(trud_rel) = releases
                    .into_iter()
                    .find(|r| r.archive_file_sha256.eq_ignore_ascii_case(local_sha256))
                {
                    return Ok(Some(MatchedTrudArchive {
                        release_date: trud_rel.release_date,
                        trud_release_sha256: trud_rel.archive_file_sha256,
                        trud_release_filesize_bytes: trud_rel.archive_file_size,
                        trud_name: trud_rel.name,
                        source: crate::provenance::TrudVerificationSource::TrudApi,
                    }));
                }
            }
            Err(e) => {
                if let Some(prog) = opts.progress {
                    prog.settle(&format!("! TRUD API check failed: {}", e));
                }
            }
        }
    }

    Ok(None)
}

fn run_local_archive(
    args: &Args,
    workspace_root: &Path,
    local_path: &Path,
    progress: &Progress,
) -> Result<()> {
    let oci_fetcher = crate::commands::pull::HttpOciFetcher;
    let trud_fetcher = args.api_key.as_ref().filter(|k| !k.trim().is_empty()).map(|k| UreqTrudFetcher::new(k, args.verbose));
    run_local_archive_with_fetchers(args, workspace_root, local_path, progress, trud_fetcher.as_ref(), Some(&oci_fetcher))
}

pub fn run_local_archive_with_fetchers<F: TrudFetcher, OF: crate::commands::pull::OciBlobFetcher>(
    args: &Args,
    workspace_root: &Path,
    local_path: &Path,
    progress: &Progress,
    trud_fetcher: Option<&F>,
    oci_fetcher: Option<&OF>,
) -> Result<()> {
    let zip_file = resolve_zip_file(local_path)?;
    let file_name = zip_file.file_name()
        .and_then(|s| s.to_str())
        .context("Invalid archive filename UTF-8")?
        .to_string();
    let file_size = std::fs::metadata(&zip_file)?.len();

    let local_sha256 = compute_file_sha256(&zip_file)?;

    let matched_opt = match_archive_by_hash(
        &local_sha256,
        workspace_root,
        MatchArchiveOptions {
            index_override: args.index.as_deref(),
            allow_network: true,
            persist_index: args.output.is_none(),
            trud_fetcher,
            oci_fetcher,
            progress: Some(progress),
        },
    )?;

    let matched = match matched_opt {
        Some(m) => m,
        None => {
            progress.clear_live();
            eprintln!(
                "{}",
                crate::provenance::format_unmatched_archive_refusal(&file_name, &local_sha256)
            );
            return Err(crate::commands::pull::AlreadyReported.into());
        }
    };

    let release_date = matched.release_date;

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

    let _prov = crate::provenance::write_provenance(
        &dest_dir,
        &release_date,
        &matched.trud_release_sha256,
        matched.trud_release_filesize_bytes,
    )?;

    let mut pin_moved = false;
    if is_workspace {
        pin_moved = update_active_release_link_if_changed(workspace_root, &release_date)?;
    }

    match matched.source {
        crate::provenance::TrudVerificationSource::PublishedRelease => {
            progress.settle(&format!(
                "✓ {}  {}  local archive, SHA-256 verified by ods release index",
                release_date,
                format_size(file_size)
            ));
        }
        crate::provenance::TrudVerificationSource::TrudApi => {
            progress.settle(&format!(
                "✓ {}  {}  local archive, SHA-256 verified by TRUD API",
                release_date,
                format_size(file_size)
            ));
        }
        _ => {}
    }

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
            trud_release_filesize_bytes: Some(file_size),
            trud_release_sha256: Some(local_sha256),
            trud_release_sha256_verified: Some(matched.source),
            status: "local".to_string(),
            path: Some(dest_dir.display().to_string()),
            error: None,
        });
    }

    progress.finish("Done!");
    Ok(())
}

pub fn run_verify_only_with_fetchers<F: TrudFetcher, OF: crate::commands::pull::OciBlobFetcher>(
    args: &Args,
    workspace_root: &Path,
    verify_path: &Path,
    progress: &Progress,
    trud_fetcher: Option<&F>,
    oci_fetcher: Option<&OF>,
) -> Result<()> {
    let zip_file = resolve_zip_file(verify_path)?;
    let file_name = zip_file.file_name()
        .and_then(|s| s.to_str())
        .context("Invalid archive filename UTF-8")?
        .to_string();
    let file_size = std::fs::metadata(&zip_file)?.len();

    let (release_date, _) = parse_trud_filename(&file_name).unwrap_or_else(|_| (String::new(), None));
    if release_date.is_empty() {
        anyhow::bail!(
            "Unrecognized TRUD ZIP filename format '{}'. Expected format: hscorgrefdataxml_data_<VERSION>_<YYYYMMDD...>.zip",
            file_name
        );
    }

    progress.step(&format!("verifying archive SHA-256 for {}…", release_date));
    let local_sha256 = compute_file_sha256(&zip_file)?;

    // `--verify-only` never installs, so it always had its own workspace to check
    // against; out of this brief's scope, so its caching behaviour is unchanged.
    let outcome = verify_archive(
        &release_date,
        &local_sha256,
        workspace_root,
        args.index.as_deref(),
        true,
        true,
        trud_fetcher,
        oci_fetcher,
    )?;

    match outcome {
        ArchiveVerificationOutcome::VerifiedPublished { .. } => {
            progress.settle(&format!(
                "✓ {}  {}  SHA-256 verified by ods release index",
                release_date,
                format_size(file_size)
            ));
            progress.finish("");
            Ok(())
        }
        ArchiveVerificationOutcome::VerifiedTrud { .. } => {
            progress.settle(&format!(
                "✓ {}  {}  SHA-256 verified by TRUD API",
                release_date,
                format_size(file_size)
            ));
            progress.finish("");
            Ok(())
        }
        ArchiveVerificationOutcome::TrudApiError(ref err) => {
            progress.settle(&format!("! TRUD API check failed: {}", err));
            progress.error(
                &format!("{}  no published SHA-256 to compare against", release_date),
                &[&format!(
                    "The ods release index has no entry for {}. TRUD API check failed: {}",
                    release_date, err
                )],
            );
            anyhow::bail!(crate::commands::pull::AlreadyReported);
        }
        ArchiveVerificationOutcome::Unverified => {
            progress.error(
                &format!("{}  no published SHA-256 to compare against", release_date),
                &[&format!(
                    "The ods release index has no entry for {}. Set TRUD_API_KEY to check it against TRUD.",
                    release_date
                )],
            );
            anyhow::bail!(crate::commands::pull::AlreadyReported);
        }
        ArchiveVerificationOutcome::Mismatch { source_name, expected_sha256, actual_sha256 } => {
            let bad_path = mark_bad_sha_file(&zip_file);
            progress.error(
                "SHA-256 Checksum Failed!",
                &[
                    &format!("Local SHA-256: {}", actual_sha256),
                    &format!("{} SHA-256: {}", source_name, expected_sha256),
                    &format!("Renamed bad local file to {}", bad_path.display()),
                ],
            );
            anyhow::bail!(crate::commands::pull::AlreadyReported);
        }
    }
}

fn write_provenance_file(release_dir: &Path, release: &TrudReleaseItem) -> Result<()> {
    crate::provenance::write_provenance(
        release_dir,
        &release.release_date,
        &release.archive_file_sha256,
        release.archive_file_size,
    )?;
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
    /// NHS's files the release directory holds now.
    pub captured: Vec<&'static str>,
    /// The subset of `captured` this call downloaded: the others were already on disk.
    pub fetched: Vec<&'static str>,
    pub missing_reasons: Vec<&'static str>,
}

impl AttestationCaptureResult {
    pub fn formatted_line(&self) -> String {
        format_attestations_line(&self.captured, &self.missing_reasons)
    }
}

/// Downloads `url` to `dest` by way of a `.part` file, so a failed refresh never leaves a
/// good file half overwritten.
fn download_file_atomically<F: TrudFetcher>(fetcher: &F, url: &str, dest: &Path) -> Result<()> {
    let mut part_name = dest.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    part_name.push(".part");
    let part = dest.with_file_name(part_name);
    if let Err(e) = fetcher.download_file(url, &part) {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    std::fs::rename(&part, dest)?;
    Ok(())
}

/// What an older `ods trud pull` saved beside the archive: TRUD's whole listing on the day of
/// the pull, named `<prefix>-<date>.json`. Nothing writes it now.
const LEGACY_LISTING_PREFIX: &str = "trud-releases";

/// Removes a listing an older pull left in a release's `trud/`.
fn remove_legacy_listing(trud_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(trud_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(LEGACY_LISTING_PREFIX) && name.ends_with(".json") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Makes sure the release's `trud/` holds NHS's checksum, signature and public key, and
/// removes the TRUD listing an older `ods trud pull` saved there: it was the one file in the
/// directory that isn't NHS's bytes, and it differed between pulls. Files already on disk
/// are left alone unless `force`, which fetches all three again.
pub fn capture_attestations<F: TrudFetcher>(
    trud_dir: &Path,
    release: &TrudReleaseItem,
    fetcher: &F,
    force: bool,
) -> AttestationCaptureResult {
    let _ = std::fs::create_dir_all(trud_dir);
    remove_legacy_listing(trud_dir);

    // Fetches one of NHS's files unless it is already held (and not `force`d)
    fn take<F: TrudFetcher>(
        fetcher: &F,
        force: bool,
        name: &'static str,
        url: &str,
        dest: &Path,
        failed: &'static str,
        out: &mut (Vec<&'static str>, Vec<&'static str>, Vec<&'static str>),
    ) {
        let need = force || !dest.exists();
        if !need || download_file_atomically(fetcher, url, dest).is_ok() {
            out.0.push(name);
            if need {
                out.1.push(name);
            }
        } else {
            out.2.push(failed);
        }
    }
    let mut out = (Vec::new(), Vec::new(), Vec::new());

    // 1. Checksum file
    if let Some(ref url) = release.checksum_file_url {
        let filename = release.checksum_file_name.as_deref().unwrap_or_else(|| {
            url.split('?').next().unwrap_or(url).rsplit('/').next().unwrap_or("checksum.xml")
        });
        take(fetcher, force, "checksum", url, &trud_dir.join(filename), "checksum download failed", &mut out);
    } else {
        out.2.push("checksum not offered for this release");
    }

    // 2. Signature file (stored as .xml.asc)
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
        take(fetcher, force, "signature", url, &trud_dir.join(filename), "signature download failed", &mut out);
    } else {
        out.2.push("signature not offered for this release");
    }

    // 3. Public key file
    if let Some(ref url) = release.public_key_file_url {
        let filename = release.public_key_file_name.as_deref().unwrap_or_else(|| {
            url.split('?').next().unwrap_or(url).rsplit('/').next().unwrap_or("trud-public-key.pgp")
        });
        take(fetcher, force, "public key", url, &trud_dir.join(filename), "public key download failed", &mut out);
    } else {
        out.2.push("public key not offered for this release");
    }

    let (captured, fetched, missing_reasons) = out;
    AttestationCaptureResult {
        captured,
        fetched,
        missing_reasons,
    }
}

/// What a pull did to a release directory whose zip it already held.
struct Healed {
    /// What was fetched or rewritten, in the words the release block uses.
    done: Vec<&'static str>,
    attestations: AttestationCaptureResult,
}

impl Healed {
    /// `repaired: checksum, signature, key`, or `refreshed: …` under `--force`. `None` when the
    /// directory was already whole.
    fn describe(&self, force: bool) -> Option<String> {
        if self.done.is_empty() {
            return None;
        }
        let verb = if force { "refreshed" } else { "repaired" };
        let names: Vec<&str> = self.done.iter().map(|n| if *n == "public key" { "key" } else { *n }).collect();
        Some(format!("{}: {}", verb, names.join(", ")))
    }

    /// The `--format ndjson` status: `repaired`, `refreshed` under `--force`, or `cached` when
    /// the directory was already whole.
    fn status(&self, force: bool) -> &'static str {
        match (self.done.is_empty(), force) {
            (true, _) => "cached",
            (false, true) => "refreshed",
            (false, false) => "repaired",
        }
    }
}

/// Makes a release directory whole from the zip it holds, never downloading the zip: fetches
/// whichever of NHS's three files are missing (all three under `force`), writes
/// `_provenance.json` if it is missing or unreadable (always under `force`), and removes a
/// leftover TRUD listing.
fn heal_release_dir<F: TrudFetcher>(
    release_dir: &Path,
    trud_dir: &Path,
    release: &TrudReleaseItem,
    fetcher: &F,
    force: bool,
) -> Result<Healed> {
    let attestations = capture_attestations(trud_dir, release, fetcher, force);
    let mut done = attestations.fetched.clone();
    if write_provenance_json(release_dir, release, force)? {
        done.push("provenance");
    }
    Ok(Healed { done, attestations })
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

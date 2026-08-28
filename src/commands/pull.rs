use anyhow::{Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;
use sha2::Digest;

use crate::progress::{format_duration, format_size, Progress, ProgressCaps};
use crate::provenance::compute_file_sha256;
use crate::workspace::{
    ensure_workspace_gitignore, find_workspace_root, generate_workspace_readme,
    is_release_dir_verified, list_releases, set_active_release, DEFAULT_WORKSPACE_DIR,
};

#[derive(Parser, Debug, Default, Clone)]
pub struct Args {
    /// Target release date in YYYY-MM-DD format (defaults to latest available release)
    pub release_date: Option<String>,

    /// List all available remote and local release versions
    #[arg(long, short = 'l')]
    pub list: bool,

    /// Fetch all available dataset releases
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

    /// Print verbose pull output
    #[arg(long, short = 'v')]
    pub verbose: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GithubRelease {
    pub tag_name: String,
    pub name: Option<String>,
    pub draft: bool,
    pub prerelease: bool,
    pub assets: Vec<GithubAsset>,
    pub body: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GithubAsset {
    pub name: String,
    pub browser_download_url: String,
    pub size: u64,
}

#[derive(Debug, Serialize, Clone)]
pub struct ReleaseOutcome {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filesize_bytes: Option<u64>,
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

pub trait ReleaseFetcher: Send + Sync {
    fn fetch_releases(&self) -> Result<Vec<GithubRelease>>;
    fn download_asset(&self, url: &str) -> Result<Vec<u8>>;
    fn fetch_release_index(&self) -> Result<Option<crate::index::OdsReleaseIndex>> {
        Ok(None)
    }
}

pub struct UreqReleaseFetcher {
    pub repo: String,
}

impl UreqReleaseFetcher {
    pub fn new(repo: &str) -> Self {
        Self {
            repo: repo.to_string(),
        }
    }
}

impl ReleaseFetcher for UreqReleaseFetcher {
    fn fetch_releases(&self) -> Result<Vec<GithubRelease>> {
        let url = format!("https://api.github.com/repos/{}/releases", self.repo);
        let resp = ureq::get(&url)
            .set("User-Agent", "ods-cli")
            .call();

        match resp {
            Ok(response) => {
                let releases: Vec<GithubRelease> = response.into_json()?;
                Ok(releases)
            }
            Err(ureq::Error::Status(code, response)) => {
                if code == 403 || code == 429 {
                    anyhow::bail!(
                        "✖ GitHub API rate limit exceeded (HTTP {}). Please try again later.",
                        code
                    );
                } else {
                    anyhow::bail!(
                        "✖ GitHub API request failed (HTTP {}): {}",
                        code,
                        response.status_text()
                    );
                }
            }
            Err(e) => {
                anyhow::bail!("✖ Failed to connect to GitHub Releases API: {}", e);
            }
        }
    }

    fn download_asset(&self, url: &str) -> Result<Vec<u8>> {
        let resp = ureq::get(url)
            .set("User-Agent", "ods-cli")
            .call();

        match resp {
            Ok(response) => {
                let mut bytes = Vec::new();
                response.into_reader().read_to_end(&mut bytes)?;
                Ok(bytes)
            }
            Err(e) => anyhow::bail!("✖ Download failed from {}: {}", url, e),
        }
    }

    fn fetch_release_index(&self) -> Result<Option<crate::index::OdsReleaseIndex>> {
        let url = format!("https://raw.githubusercontent.com/{}/main/data/releases.json", self.repo);
        let resp = match ureq::get(&url)
            .set("User-Agent", "ods-cli")
            .timeout(std::time::Duration::from_secs(3))
            .call()
        {
            Ok(r) => r,
            Err(_) => return Ok(None),
        };
        let mut bytes = Vec::new();
        resp.into_reader().read_to_end(&mut bytes)?;
        let index: crate::index::OdsReleaseIndex = serde_json::from_slice(&bytes)?;
        Ok(Some(index))
    }
}

pub fn run(args: Args) -> Result<()> {
    let workspace_root =
        find_workspace_root().unwrap_or_else(|| PathBuf::from(DEFAULT_WORKSPACE_DIR));
    let fetcher = UreqReleaseFetcher::new("olizilla/ods");
    run_with_fetcher(args, &workspace_root, &fetcher)
}

pub fn run_with_fetcher<F: ReleaseFetcher>(
    args: Args,
    workspace_root: &Path,
    fetcher: &F,
) -> Result<()> {
    run_with_fetcher_and_index(args, workspace_root, fetcher, None)
}

pub fn run_with_fetcher_and_index<F: ReleaseFetcher>(
    args: Args,
    workspace_root: &Path,
    fetcher: &F,
    base_index: Option<crate::index::OdsReleaseIndex>,
) -> Result<()> {
    let progress = Progress::stderr(ProgressCaps::detect(args.quiet, args.verbose, args.no_progress));

    // 1. Start with baked release index (or injected test index)
    let baked_index = match base_index {
        Some(idx) => idx,
        None => crate::index::OdsReleaseIndex::baked().unwrap_or_else(|_| crate::index::OdsReleaseIndex {
            type_tag: "ods_release_index".to_string(),
            index_version: 2,
            concept_doi: None,
            mirrors: vec![],
            releases: vec![],
        }),
    };

    // 2. Fetch remote index if available, and merge (catching security contradictions)
    let active_index = match fetcher.fetch_release_index() {
        Ok(Some(fetched)) => {
            match baked_index.merge(&fetched) {
                Ok(m) => m,
                Err(e) => {
                    progress.error(&e.to_string(), &[]);
                    return Err(e);
                }
            }
        }
        _ => baked_index,
    };

    if args.list {
        return run_list(workspace_root, fetcher, &args, &progress, &mut std::io::stdout(), &mut std::io::stderr());
    }

    if args.all {
        return pull_all_releases(workspace_root, &args, fetcher, &progress);
    }

    // 3. Check if requested release or latest release is handled by OCI index
    let resolved_entry = if let Some(ref target_date) = args.release_date {
        let is_known = active_index.releases.iter().any(|r| &r.trud_release_date == target_date);
        if is_known {
            Some(active_index.resolve(Some(target_date))?)
        } else {
            None
        }
    } else if !active_index.releases.is_empty() {
        Some(active_index.resolve(None)?)
    } else {
        None
    };

    if let Some(entry) = resolved_entry {
        return pull_oci_release(workspace_root, entry, &active_index.mirrors, &args, fetcher, &progress);
    }

    // 4. Fallback to legacy GitHub releases pull if not in OCI release index
    if let Some(ref target_date) = args.release_date {
        let release_dir = workspace_root.join("releases").join(target_date);
        if !args.force && release_dir.exists() && is_release_dir_verified(&release_dir) {
            progress.settle(&format!("* Release {} already local, verified", target_date));
            if !args.all {
                let pin_moved = update_active_release_link_if_changed(workspace_root, target_date)?;
                generate_workspace_readme(workspace_root, target_date, None, None)?;
                if pin_moved {
                    progress.settle_detail(&format!("current → releases/{}", target_date));
                }
            }
            return Ok(());
        }
        return pull_specific_release(workspace_root, target_date, &args, fetcher, &progress);
    }

    pull_latest_release(workspace_root, &args, fetcher, &progress)
}

fn pull_oci_release<F: ReleaseFetcher>(
    workspace_root: &Path,
    entry: &crate::index::ReleaseIndexEntry,
    mirrors: &[crate::index::MirrorEntry],
    args: &Args,
    fetcher: &F,
    progress: &Progress,
) -> Result<()> {
    let release_date = &entry.trud_release_date;
    let release_dir = workspace_root.join("releases").join(release_date);

    if !args.force && release_dir.exists() && is_release_dir_verified(&release_dir) {
        progress.settle(&format!("* Release {} already local, verified", release_date));
        if !args.all {
            let pin_moved = update_active_release_link_if_changed(workspace_root, release_date)?;
            generate_workspace_readme(workspace_root, release_date, None, None)?;
            if pin_moved {
                progress.settle_detail(&format!("current → releases/{}", release_date));
            }
        }
        return Ok(());
    }

    fs::create_dir_all(&release_dir)?;

    let default_mirror = crate::index::MirrorEntry {
        operator: "github".to_string(),
        kind: "files".to_string(),
        url: "https://github.com/olizilla/ods/releases/download/data/{release}_{version}/{file}".to_string(),
    };
    let mirror = mirrors.first().unwrap_or(&default_mirror);

    progress.step(&format!("Fetching manifest for {} v{}…", release_date, entry.dataset_version));

    // 1. Fetch manifest.json
    let manifest_url = mirror.expand_url(release_date, &entry.dataset_version, "manifest.json");
    let manifest_bytes = fetcher.download_asset(&manifest_url)?;

    // 2. VERIFY BEFORE PARSE
    let manifest_sha = format!("sha256:{:x}", sha2::Sha256::digest(&manifest_bytes));
    if manifest_sha != entry.manifest_digest {
        anyhow::bail!(
            "✖ Manifest digest mismatch for {}\n  Expected: {}\n  Actual:   {}",
            entry.tag,
            entry.manifest_digest,
            manifest_sha
        );
    }

    // 3. Parse manifest
    let manifest: crate::oci::OciManifest = serde_json::from_slice(&manifest_bytes)
        .context("Failed to parse verified OCI manifest")?;

    // 4. Download layers by org.opencontainers.image.title
    for layer in &manifest.layers {
        let title = layer
            .annotations
            .as_ref()
            .and_then(|a| a.get("org.opencontainers.image.title"))
            .ok_or_else(|| anyhow::anyhow!("Layer missing title annotation"))?;

        progress.step(&format!("Downloading {}…", title));
        let layer_url = mirror.expand_url(release_date, &entry.dataset_version, title);
        let layer_bytes = fetcher.download_asset(&layer_url)?;

        // Verify layer digest
        let layer_sha = format!("sha256:{:x}", sha2::Sha256::digest(&layer_bytes));
        if layer_sha != layer.digest {
            anyhow::bail!(
                "✖ Layer digest mismatch for {}\n  Expected: {}\n  Actual:   {}",
                title,
                layer.digest,
                layer_sha
            );
        }

        let out_file = release_dir.join(title);
        fs::write(&out_file, &layer_bytes)?;
    }

    // 5. Write _release.json
    let release_json = serde_json::to_string_pretty(entry)?;
    fs::write(release_dir.join("_release.json"), format!("{}\n", release_json))?;

    // 6. Pin current, write README.md, ensure gitignore
    ensure_workspace_gitignore(workspace_root)?;

    if !args.all {
        let pin_moved = update_active_release_link_if_changed(workspace_root, release_date)?;
        generate_workspace_readme(workspace_root, release_date, None, None)?;

        progress.settle(&format!(
            "✓ {} v{} downloaded and verified (manifest {})",
            release_date, entry.dataset_version, manifest_sha
        ));
        if pin_moved {
            progress.settle_detail(&format!("current → releases/{}", release_date));
        }
        progress.finish("Done!");
    } else {
        progress.settle(&format!(
            "✓ {} v{} downloaded and verified (manifest {})",
            release_date, entry.dataset_version, manifest_sha
        ));
        progress.settle_detail(&format!("releases/{}", release_date));
    }

    Ok(())
}

/// Marker error for a command that has already written its own diagnostics.
#[derive(Debug)]
pub struct AlreadyReported;

impl std::fmt::Display for AlreadyReported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "command failed; see the messages above")
    }
}

impl std::error::Error for AlreadyReported {}

pub fn run_list<F: ReleaseFetcher, W1: std::io::Write, W2: std::io::Write>(
    workspace_root: &Path,
    fetcher: &F,
    args: &Args,
    progress: &Progress,
    stdout: &mut W1,
    stderr: &mut W2,
) -> Result<()> {
    if progress.caps().is_tty && args.format.is_none() {
        writeln!(stderr, "Querying available ODS dataset releases...\n")?;
    }

    let (remote_releases, fetch_err) = match fetcher.fetch_releases() {
        Ok(rel) => (rel, None),
        Err(e) => (Vec::new(), Some(e)),
    };

    let remote_dates: Vec<(String, u64)> = remote_releases
        .iter()
        .filter(|r| !r.draft && r.tag_name.starts_with("data/"))
        .map(|r| {
            let date = r.tag_name.trim_start_matches("data/").to_string();
            let size: u64 = r.assets.iter().map(|a| a.size).sum();
            (date, size)
        })
        .collect();

    let local_releases = if workspace_root.exists() {
        list_releases(workspace_root).unwrap_or_default()
    } else {
        vec![]
    };

    let (active_date, _) = crate::workspace::get_active_release(workspace_root).unwrap_or_default();

    let mut all_dates_map: BTreeMap<String, (u64, bool, bool)> = BTreeMap::new();
    for (d, sz) in &remote_dates {
        all_dates_map.insert(d.clone(), (*sz, false, true));
    }
    for r in &local_releases {
        let entry = all_dates_map.entry(r.date.clone()).or_insert((0, true, false));
        entry.1 = true;
    }

    let mut sorted_dates: Vec<(String, u64, bool, bool)> = all_dates_map
        .into_iter()
        .map(|(d, (sz, is_loc, is_rem))| (d, sz, is_loc, is_rem))
        .collect();
    sorted_dates.sort_by(|a, b| b.0.cmp(&a.0));

    if args.format.as_deref() == Some("json") {
        let json_items: Vec<ReleaseListItemJson> = sorted_dates.iter().map(|(date, sz, is_loc, _)| {
            let is_act = !active_date.is_empty() && active_date == *date;
            let status_str = if is_act {
                "active (local)"
            } else if *is_loc {
                "local"
            } else {
                "remote"
            };
            ReleaseListItemJson {
                date: date.clone(),
                size_bytes: *sz,
                status: status_str.to_string(),
            }
        }).collect();

        serde_json::to_writer_pretty(&mut *stdout, &json_items)?;
        writeln!(stdout)?;
        return Ok(());
    }

    if let Some(ref err) = fetch_err {
        writeln!(stderr, "{}\n", err)?;
    }

    let mut has_active = false;
    let mut has_local = false;
    let mut has_remote = false;

    if sorted_dates.is_empty() {
        if fetch_err.is_some() {
            writeln!(stderr, "  (No local dataset releases cached)")?;
        } else {
            writeln!(stderr, "  (No dataset releases found)")?;
        }
    } else {
        for (date, size, is_loc, is_rem) in &sorted_dates {
            let is_act = !active_date.is_empty() && active_date == *date;
            let status_str = if is_act {
                has_active = true;
                "● active (local)"
            } else if *is_loc {
                has_local = true;
                "○ local"
            } else if *is_rem {
                has_remote = true;
                "○ remote"
            } else {
                "○ unknown"
            };

            let size_str = if *size > 0 { format_size(*size) } else { "".to_string() };
            writeln!(stdout, "  {:10}  {:6}  {}", date, size_str, status_str)?;
        }
    }

    if progress.caps().is_tty && !sorted_dates.is_empty() {
        writeln!(stderr, "\nLegend:")?;
        if has_active {
            writeln!(stderr, "  ● active (local)  Active release pin (./ods_data/current)")?;
        }
        if has_local {
            writeln!(stderr, "  ○ local           Cached locally in ./ods_data/releases/")?;
        }
        if has_remote {
            writeln!(stderr, "  ○ remote          Available for pull from upstream")?;
        }
        writeln!(stderr, "\nTo pull a specific release, run: ods pull <YYYY-MM-DD>")?;
    }

    if fetch_err.is_some() {
        return Err(AlreadyReported.into());
    }

    Ok(())
}

fn pull_latest_release<F: ReleaseFetcher>(
    workspace_root: &Path,
    args: &Args,
    fetcher: &F,
    progress: &Progress,
) -> Result<()> {
    progress.step("Querying available ODS dataset releases…");
    let releases = match fetcher.fetch_releases() {
        Ok(r) => r,
        Err(e) => {
            let local_releases = if workspace_root.exists() {
                list_releases(workspace_root).unwrap_or_default()
            } else {
                vec![]
            };

            progress.clear_live();
            eprintln!("{}", e);
            if !local_releases.is_empty() {
                let local_dates: Vec<String> = local_releases.into_iter().map(|r| r.date).collect();
                let local_dates_str = local_dates.join(", ");
                let latest_local = local_dates.first().cloned().unwrap_or_default();
                eprintln!("  Locally available: {}", local_dates_str);
                eprintln!("  Pull one by date to use it offline: ods pull {}", latest_local);
            }
            return Err(AlreadyReported.into());
        }
    };
    let mut data_releases: Vec<(String, GithubRelease)> = releases
        .into_iter()
        .filter(|r| !r.draft && r.tag_name.starts_with("data/"))
        .map(|r| {
            let date = r.tag_name.trim_start_matches("data/").to_string();
            (date, r)
        })
        .collect();

    if data_releases.is_empty() {
        anyhow::bail!("✖ No pre-built dataset releases found upstream in `data/*` namespace.");
    }

    data_releases.sort_by(|a, b| b.0.cmp(&a.0));
    let (latest_date, release) = data_releases.remove(0);

    pull_release_by_meta(workspace_root, &latest_date, &release, args, fetcher, progress)
}

fn pull_all_releases<F: ReleaseFetcher>(
    workspace_root: &Path,
    args: &Args,
    fetcher: &F,
    progress: &Progress,
) -> Result<()> {
    progress.step("Querying available ODS dataset releases…");
    let releases = fetcher.fetch_releases()?;
    let mut data_releases: Vec<(String, GithubRelease)> = releases
        .into_iter()
        .filter(|r| !r.draft && r.tag_name.starts_with("data/"))
        .map(|r| {
            let date = r.tag_name.trim_start_matches("data/").to_string();
            (date, r)
        })
        .collect();

    if data_releases.is_empty() {
        anyhow::bail!("✖ No pre-built dataset releases found upstream in `data/*` namespace.");
    }

    data_releases.sort_by(|a, b| b.0.cmp(&a.0)); // newest to oldest

    let total_count = data_releases.len();
    let total_bytes: u64 = data_releases
        .iter()
        .flat_map(|(_, r)| r.assets.iter().map(|a| a.size))
        .sum();

    progress.settle(&format!(
        "✓ {} releases available · {}",
        total_count,
        format_size(total_bytes)
    ));

    // Plan Phase
    let mut to_download: Vec<(String, GithubRelease)> = Vec::new();
    let mut cached_count = 0;
    let mut cached_bytes = 0;
    let mut latest_cached_date: Option<String> = None;

    for (date, release) in data_releases {
        let release_dir = workspace_root.join("releases").join(&date);
        let rel_size: u64 = release.assets.iter().map(|a| a.size).sum();
        if release_dir.exists() && !args.force && is_release_dir_verified(&release_dir) {
            cached_count += 1;
            cached_bytes += rel_size;
            if latest_cached_date.is_none() || latest_cached_date.as_ref().unwrap() < &date {
                latest_cached_date = Some(date.clone());
            }
        } else {
            to_download.push((date, release));
        }
    }

    let to_download_count = to_download.len();
    let to_download_bytes: u64 = to_download
        .iter()
        .flat_map(|(_, r)| r.assets.iter().map(|a| a.size))
        .sum();

    if to_download_count == 0 {
        progress.settle(&format!(
            "✓ {} cached, verified · nothing to download",
            cached_count
        ));
        return Ok(());
    }

    progress.clear_live();
    progress.settle(&format!(
        "✓ {} cached · {} to download · {}",
        cached_count,
        to_download_count,
        format_size(to_download_bytes)
    ));

    // Execute Phase
    let mut downloaded_count = 0;
    let mut downloaded_bytes = 0;
    let mut failed_count = 0;
    let mut failures: Vec<(String, String)> = Vec::new();
    let existing_active_date = crate::workspace::get_active_release(workspace_root).ok().map(|(d, _)| d);
    let mut newest_downloaded_date: Option<String> = None;
    let batch_start = Instant::now();

    progress.batch_bar(1, to_download_count, 0, to_download_bytes, None, None);

    for (date, release) in to_download {
        let rel_size: u64 = release.assets.iter().map(|a| a.size).sum();
        progress.set_in_flight(&date, &format!("{}  {}  downloading…", date, format_size(rel_size)));
        let res = pull_release_by_meta(workspace_root, &date, &release, args, fetcher, progress);
        progress.remove_in_flight(&date);
        match res {
            Ok(_) => {
                downloaded_count += 1;
                downloaded_bytes += rel_size;
                if newest_downloaded_date.is_none() || newest_downloaded_date.as_ref().unwrap() < &date {
                    newest_downloaded_date = Some(date.clone());
                }
                progress.settle(&format!("✓ {}  {}  downloaded", date, format_size(rel_size)));
                if args.format.as_deref() == Some("ndjson") {
                    emit_ndjson_outcome(&ReleaseOutcome {
                        release_date: Some(date.clone()),
                        tag_name: Some(release.tag_name.clone()),
                        filesize_bytes: Some(rel_size),
                        status: "downloaded".to_string(),
                        path: Some(format!("ods_data/releases/{}", date)),
                        error: None,
                    });
                }
            }
            Err(e) => {
                failed_count += 1;
                progress.settle(&format!("✖ {}  download failed — {}", date, e));
                failures.push((date.clone(), e.to_string()));
                if args.format.as_deref() == Some("ndjson") {
                    emit_ndjson_outcome(&ReleaseOutcome {
                        release_date: Some(date.clone()),
                        tag_name: Some(release.tag_name.clone()),
                        filesize_bytes: Some(rel_size),
                        status: "failed".to_string(),
                        path: None,
                        error: Some(e.to_string()),
                    });
                }
            }
        }
    }

    let elapsed = batch_start.elapsed();
    let landed_bytes = cached_bytes + downloaded_bytes;
    progress.clear_live();
    let symbol = if downloaded_count == 0 && failed_count > 0 { "✖" } else { "✓" };
    progress.settle_summary(&format!(
        "{} {} downloaded · {} cached · {} failed · {} total  in {}",
        symbol,
        downloaded_count,
        cached_count,
        failed_count,
        format_size(landed_bytes),
        format_duration(elapsed)
    ));

    // If --all is passed, only update current link if a newer item was actually fetched.
    if downloaded_count > 0 {
        if let Some(ref fetched_date) = newest_downloaded_date {
            let target_pin = match &existing_active_date {
                Some(existing) => {
                    if fetched_date > existing {
                        Some(fetched_date.clone())
                    } else {
                        None
                    }
                }
                None => {
                    let newest_cached = latest_cached_date.as_deref();
                    let best = newest_cached.into_iter().chain(Some(fetched_date.as_str())).max().unwrap();
                    Some(best.to_string())
                }
            };

            if let Some(ref target) = target_pin {
                let pin_moved = update_active_release_link_if_changed(workspace_root, target)?;
                generate_workspace_readme(workspace_root, target, None, None)?;
                if pin_moved {
                    progress.settle_detail(&format!("current → releases/{}", target));
                }
            }
        }
    }

    if !failures.is_empty() {
        for (f_date, f_err) in failures {
            progress.error(&format!("{}  {}", f_date, f_err), &[&format!("Retry with: ods pull {}", f_date)]);
        }
        return Err(AlreadyReported.into());
    }

    Ok(())
}

fn pull_specific_release<F: ReleaseFetcher>(
    workspace_root: &Path,
    target_date: &str,
    args: &Args,
    fetcher: &F,
    progress: &Progress,
) -> Result<()> {
    progress.step("Querying available ODS dataset releases…");
    let releases = fetcher.fetch_releases()?;
    let tag = format!("data/{}", target_date);

    let found = releases
        .iter()
        .cloned()
        .find(|r| !r.draft && (r.tag_name == tag || r.tag_name == target_date));

    match found {
        Some(release) => pull_release_by_meta(workspace_root, target_date, &release, args, fetcher, progress),
        None => {
            let mut available: Vec<String> = releases
                .iter()
                .filter(|r| !r.draft && r.tag_name.starts_with("data/"))
                .map(|r| r.tag_name.trim_start_matches("data/").to_string())
                .collect();
            available.sort_by(|a, b| b.cmp(a));

            let avail_str = if available.is_empty() {
                "  (None available)".to_string()
            } else {
                available
                    .iter()
                    .map(|d| format!("    • {}", d))
                    .collect::<Vec<_>>()
                    .join("\n")
            };

            anyhow::bail!(
                "✖ Release {} not found upstream.\nAvailable remote releases:\n{}",
                target_date,
                avail_str
            );
        }
    }
}

fn pull_release_by_meta<F: ReleaseFetcher>(
    workspace_root: &Path,
    date: &str,
    release: &GithubRelease,
    args: &Args,
    fetcher: &F,
    progress: &Progress,
) -> Result<()> {
    let release_dir = workspace_root.join("releases").join(date);

    if release_dir.exists() && !args.force {
        if is_release_dir_verified(&release_dir) {
            progress.settle(&format!("* Release {} already local, verified", date));
            let pin_moved = update_active_release_link_if_changed(workspace_root, date)?;
            generate_workspace_readme(workspace_root, date, None, None)?;
            if pin_moved {
                progress.settle_detail(&format!("current → releases/{}", date));
            }
            return Ok(());
        }
    }

    let total_bytes: u64 = release.assets.iter().map(|a| a.size).sum();

    let sha_asset = release
        .assets
        .iter()
        .find(|a| a.name == "SHA256SUMS")
        .context("✖ Release asset `SHA256SUMS` is missing from upstream release")?;

    let temp_dir_path = workspace_root
        .join("releases")
        .join(format!(".tmp_pull_{}_{}", date, std::process::id()));
    if temp_dir_path.exists() {
        let _ = fs::remove_dir_all(&temp_dir_path);
    }
    fs::create_dir_all(&temp_dir_path)?;

    progress.step(&format!("{}   downloading SHA256SUMS…", date));
    let sha256sums_bytes = fetcher.download_asset(&sha_asset.browser_download_url)?;
    fs::write(temp_dir_path.join("SHA256SUMS"), &sha256sums_bytes)?;

    let sha256sums_str = String::from_utf8_lossy(&sha256sums_bytes);
    let expected_hashes = crate::workspace::parse_sha256sums(&sha256sums_str);

    for (file_name, _expected_hash) in &expected_hashes {
        if file_name == "SHA256SUMS" {
            continue;
        }

        progress.step(&format!("{}   downloading {}…", date, file_name));

        let file_bytes = if let Some(asset) = release.assets.iter().find(|a| a.name == *file_name) {
            fetcher.download_asset(&asset.browser_download_url)?
        } else {
            let url = format!(
                "https://github.com/olizilla/ods/releases/download/{}/{}",
                release.tag_name, file_name
            );
            fetcher.download_asset(&url)?
        };

        let file_path = temp_dir_path.join(file_name);
        fs::write(&file_path, file_bytes)?;
    }

    progress.step(&format!("{}   verifying SHA-256 checksums…", date));
    for (file_name, expected_hash) in &expected_hashes {
        let file_path = temp_dir_path.join(file_name);
        if !file_path.exists() {
            anyhow::bail!("✖ Missing file listed in SHA256SUMS: {}", file_name);
        }

        let actual_hash = compute_file_sha256(&file_path)?;
        if actual_hash.to_lowercase() != expected_hash.to_lowercase() {
            let bad_path = temp_dir_path.join(format!("{}.bad-sha", file_name));
            let _ = fs::rename(&file_path, &bad_path);

            anyhow::bail!(
                "✖ Checksum verification failed for {}\n  Expected SHA-256: {}\n  Actual SHA-256:   {}\nRenamed corrupted file to {}",
                file_name,
                expected_hash,
                actual_hash,
                bad_path.display()
            );
        }
    }


    if release_dir.exists() {
        let _ = fs::remove_dir_all(&release_dir);
    }
    fs::rename(&temp_dir_path, &release_dir)?;

    ensure_workspace_gitignore(workspace_root)?;

    if !args.all {
        let pin_moved = update_active_release_link_if_changed(workspace_root, date)?;
        generate_workspace_readme(workspace_root, date, None, None)?;

        progress.settle(&format!(
            "✓ {}  {}  SHA-256 verified",
            date,
            format_size(total_bytes)
        ));
        if progress.caps().verbose {
            for (f, h) in &expected_hashes {
                progress.settle_detail(&format!("SHA-256 ({}): {}", f, h));
            }
        }
        if pin_moved {
            progress.settle_detail(&format!("current → releases/{}", date));
        }
        progress.finish("Done!");
    }

    Ok(())
}

fn emit_ndjson_outcome(outcome: &ReleaseOutcome) {
    if let Ok(json) = serde_json::to_string(outcome) {
        println!("{}", json);
    }
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




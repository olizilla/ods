use anyhow::{Context, Result};
use clap::Parser;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::provenance::{compute_file_sha256, OdsProvenance, PROVENANCE_FILENAME};
use crate::workspace::{
    ensure_workspace_gitignore, find_workspace_root, generate_workspace_readme, list_releases,
    set_active_release, DEFAULT_WORKSPACE_DIR,
};

#[derive(Parser, Debug, Default)]
pub struct Args {
    /// Target release date in YYYY-MM-DD format (defaults to latest available release)
    pub release_date: Option<String>,

    /// List all available remote and local release versions
    #[arg(long, short = 'l')]
    pub list: bool,

    /// Force re-download or re-pull of specified release
    #[arg(long, short = 'f')]
    pub force: bool,

    /// TRUD API key if pulling directly from TRUD
    #[arg(long, env = "NHS_TRUD_API_KEY")]
    pub api_key: Option<String>,

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

pub trait ReleaseFetcher {
    fn fetch_releases(&self) -> Result<Vec<GithubRelease>>;
    fn download_asset(&self, url: &str) -> Result<Vec<u8>>;
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
    if args.list {
        return run_list(workspace_root, fetcher, &mut std::io::stdout(), &mut std::io::stderr());
    }

    if let Some(ref target_date) = args.release_date {
        return pull_specific_release(workspace_root, target_date, args.force, fetcher);
    }

    pull_latest_release(workspace_root, args.force, fetcher)
}

/// Marker error for a command that has already written its own diagnostics.
///
/// `main` turns this into a non-zero exit status without printing anything
/// further, so a carefully formatted `✖` line is not followed by a redundant
/// `Error:` repeat of the same thing.
#[derive(Debug)]
pub struct AlreadyReported;

impl std::fmt::Display for AlreadyReported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Not normally surfaced — main intercepts this before printing.
        write!(f, "command failed; see the messages above")
    }
}

impl std::error::Error for AlreadyReported {}

pub fn run_list<F: ReleaseFetcher, W1: std::io::Write, W2: std::io::Write>(
    workspace_root: &Path,
    fetcher: &F,
    stdout: &mut W1,
    stderr: &mut W2,
) -> Result<()> {
    writeln!(stderr, "Querying available ODS dataset releases...\n")?;

    let (remote_dates, fetch_err) = match fetcher.fetch_releases() {
        Ok(remote_releases) => {
            let mut dates: Vec<String> = remote_releases
                .iter()
                .filter(|r| !r.draft && r.tag_name.starts_with("data/"))
                .map(|r| r.tag_name.trim_start_matches("data/").to_string())
                .collect();
            dates.sort_by(|a, b| b.cmp(a));
            (dates, None)
        }
        Err(e) => (Vec::new(), Some(e)),
    };

    if let Some(ref err) = fetch_err {
        writeln!(stderr, "{}\n", err)?;
    }

    let local_releases = if workspace_root.exists() {
        list_releases(workspace_root).unwrap_or_default()
    } else {
        vec![]
    };

    let active_date = local_releases.iter().find(|r| r.is_active).map(|r| r.date.clone());

    let mut all_dates: Vec<String> = Vec::new();
    for d in &remote_dates {
        if !all_dates.contains(d) {
            all_dates.push(d.clone());
        }
    }
    for r in &local_releases {
        if !all_dates.contains(&r.date) {
            all_dates.push(r.date.clone());
        }
    }
    all_dates.sort_by(|a, b| b.cmp(a));

    if all_dates.is_empty() {
        if fetch_err.is_some() {
            writeln!(stderr, "  (No local dataset releases cached)")?;
        } else {
            writeln!(stderr, "  (No dataset releases found)")?;
        }
    } else {
        for date in &all_dates {
            let is_active = active_date.as_deref() == Some(date.as_str());
            let is_local = local_releases.iter().any(|r| r.date == *date);
            let is_remote = remote_dates.contains(date);

            let status_str = if is_active {
                "● active (local)"
            } else if is_local {
                "○ local"
            } else if is_remote {
                "○ remote"
            } else {
                "○ unknown"
            };

            writeln!(stdout, "  {:14} {}", date, status_str)?;
        }
    }

    writeln!(stderr, "\nLegend:")?;
    writeln!(stderr, "  ● active (local)  Active release pin (./ods_data/current)")?;
    writeln!(stderr, "  ○ local           Cached locally in ./ods_data/releases/")?;
    writeln!(stderr, "  ○ remote          Available for pull from upstream")?;
    writeln!(stderr, "\nTo pull a specific release, run: ods pull <YYYY-MM-DD>")?;

    // Listing is primarily a question about what exists upstream — local
    // releases are visible with `ls`. If the upstream half could not be
    // answered the listing is incomplete, and what it most likely omits is a
    // newer release, which is the usual reason to run this at all.
    //
    // So exit non-zero even when local releases were listed successfully: the
    // human output above is complete and honest, but the exit status is the
    // only signal a script can read, and it must not report a partial view as
    // a whole one. This matches `git fetch` and `apt update`, which fail on an
    // unreachable remote regardless of what is cached.
    if fetch_err.is_some() {
        return Err(AlreadyReported.into());
    }

    Ok(())
}

fn pull_latest_release<F: ReleaseFetcher>(
    workspace_root: &Path,
    force: bool,
    fetcher: &F,
) -> Result<()> {
    eprintln!("Querying available ODS dataset releases...");
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

    data_releases.sort_by(|a, b| b.0.cmp(&a.0));
    let (latest_date, release) = data_releases.remove(0);

    pull_release_by_meta(workspace_root, &latest_date, &release, force, fetcher)
}

fn pull_specific_release<F: ReleaseFetcher>(
    workspace_root: &Path,
    target_date: &str,
    force: bool,
    fetcher: &F,
) -> Result<()> {
    eprintln!("Querying available ODS dataset releases...");
    let releases = fetcher.fetch_releases()?;
    let tag = format!("data/{}", target_date);

    let found = releases
        .iter()
        .cloned()
        .find(|r| !r.draft && (r.tag_name == tag || r.tag_name == target_date));

    match found {
        Some(release) => pull_release_by_meta(workspace_root, target_date, &release, force, fetcher),
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
    force: bool,
    fetcher: &F,
) -> Result<()> {
    let release_dir = workspace_root.join("releases").join(date);

    if release_dir.exists() && !force {
        if is_release_dir_verified(&release_dir) {
            eprintln!("* Release {} is already cached locally.", date);
            set_active_release(workspace_root, date)?;
            generate_workspace_readme(workspace_root, date, None, None)?;
            eprintln!("✓ Linked to releases/{}", date);
            eprintln!("Done!");
            return Ok(());
        }
    }

    let total_bytes: u64 = release.assets.iter().map(|a| a.size).sum();
    let total_mb = total_bytes / (1024 * 1024);
    eprintln!(
        "✓ Found release {} ({} files, {} MB)",
        date,
        release.assets.len(),
        total_mb
    );

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

    let sha256sums_bytes = fetcher.download_asset(&sha_asset.browser_download_url)?;
    fs::write(temp_dir_path.join("SHA256SUMS"), &sha256sums_bytes)?;

    let sha256sums_str = String::from_utf8_lossy(&sha256sums_bytes);
    let expected_hashes = parse_sha256sums(&sha256sums_str);

    for (file_name, _expected_hash) in &expected_hashes {
        if file_name == "SHA256SUMS" {
            continue;
        }

        eprintln!("Downloading {}...", file_name);

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

    eprintln!("Verifying checksums...");
    let mut verified_count = 0;
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
        verified_count += 1;
    }

    eprintln!("✓ {}/{} files match SHA256SUMS", verified_count, expected_hashes.len());

    let prov_path = temp_dir_path.join(PROVENANCE_FILENAME);
    if prov_path.exists() {
        if let Ok(content) = fs::read_to_string(&prov_path) {
            if let Ok(mut prov) = serde_json::from_str::<OdsProvenance>(&content) {
                if prov.dataset_doi.is_none() {
                    if let Some(ref body) = release.body {
                        if let Some(doi) = extract_doi_from_text(body) {
                            prov.dataset_doi = Some(doi);
                            if let Ok(updated_json) = serde_json::to_string_pretty(&prov) {
                                let _ = fs::write(&prov_path, updated_json);
                            }
                        }
                    }
                }
            }
        }
    }

    if release_dir.exists() {
        let _ = fs::remove_dir_all(&release_dir);
    }
    fs::rename(&temp_dir_path, &release_dir)?;

    eprintln!("Updating current release link...");
    ensure_workspace_gitignore(workspace_root)?;
    set_active_release(workspace_root, date)?;
    generate_workspace_readme(workspace_root, date, None, None)?;
    eprintln!("✓ Linked to releases/{}", date);
    eprintln!("Done!");

    Ok(())
}

fn parse_sha256sums(content: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            let hash = parts[0].to_string();
            let file_name = parts[1].trim_start_matches('*').to_string();
            map.insert(file_name, hash);
        }
    }
    map
}

fn is_release_dir_verified(release_dir: &Path) -> bool {
    let sums_path = release_dir.join("SHA256SUMS");
    if !sums_path.exists() {
        return false;
    }

    let content = match fs::read_to_string(&sums_path) {
        Ok(c) => c,
        Err(_) => return false,
    };

    let expected = parse_sha256sums(&content);
    if expected.is_empty() {
        return false;
    }

    for (file_name, exp_hash) in &expected {
        let file_path = release_dir.join(file_name);
        if !file_path.exists() {
            return false;
        }
        let act_hash = match compute_file_sha256(&file_path) {
            Ok(h) => h,
            Err(_) => return false,
        };
        if act_hash.to_lowercase() != exp_hash.to_lowercase() {
            return false;
        }
    }

    true
}

fn extract_doi_from_text(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.contains("10.5281/zenodo.") || line.contains("doi.org/10.") {
            let words: Vec<&str> = line.split_whitespace().collect();
            for word in words {
                let clean = word.trim_matches(|c: char| c == '(' || c == ')' || c == '[' || c == ']' || c == ',' || c == '.');
                if clean.contains("10.5281/zenodo.") || clean.contains("doi.org/10.") {
                    return Some(clean.to_string());
                }
            }
        }
    }
    None
}

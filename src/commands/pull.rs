use anyhow::{bail, Context, Result};
use clap::Parser;
use serde::Serialize;
use sha2::Digest;
use std::fs;
use std::io::Read;
use std::path::Path;

use crate::index::{MirrorEntry, OdsReleaseIndex, ReleaseIndexEntry};
use crate::oci::*;
use crate::progress::{Progress, ProgressCaps};
use crate::workspace::{verify_release_dir, Workspace};

#[derive(Debug)]
pub struct AlreadyReported;

impl std::fmt::Display for AlreadyReported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "already reported")
    }
}

impl std::error::Error for AlreadyReported {}

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

    /// Show errors and summary only
    #[arg(long, short = 'q', conflicts_with = "verbose")]
    pub quiet: bool,

    /// Disable interactive live progress animations
    #[arg(long)]
    pub no_progress: bool,

    /// Output format (json for release list)
    #[arg(long)]
    pub format: Option<String>,

    /// Print verbose pull output
    #[arg(long, short = 'v')]
    pub verbose: bool,
}

#[derive(Debug, Serialize)]
pub struct ReleaseListItemJson {
    pub date: String,
    pub version: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_digest: Option<String>,
}

pub trait OciBlobFetcher: Send + Sync {
    fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>>;
    fn fetch_release_index(&self) -> Result<Option<OdsReleaseIndex>> {
        if let Some(bytes) = self.fetch_release_index_raw()? {
            if let Ok(idx) = serde_json::from_slice::<OdsReleaseIndex>(&bytes) {
                return Ok(Some(idx));
            }
        }
        Ok(None)
    }
    fn fetch_release_index_raw(&self) -> Result<Option<Vec<u8>>> {
        Ok(None)
    }
}

pub struct HttpOciFetcher;

impl OciBlobFetcher for HttpOciFetcher {
    fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>> {
        download_bytes_with_auth(url, None)
    }

    fn fetch_release_index_raw(&self) -> Result<Option<Vec<u8>>> {
        // Test/dev hook: allow pointing release index fetcher to a custom URL (e.g. local Miniflare test server)
        let custom_url = std::env::var("ODS_RELEASE_INDEX_URL").ok();
        let default_urls = [
            "https://ods.fyi/releases.json",
            "https://raw.githubusercontent.com/olizilla/ods/main/data/releases.json",
        ];
        let mut index_urls: Vec<&str> = Vec::new();
        if let Some(ref u) = custom_url {
            index_urls.push(u.as_str());
        } else {
            index_urls.extend_from_slice(&default_urls);
        }
        for url in &index_urls {
            if let Ok(bytes) = download_bytes_with_auth(url, None) {
                if serde_json::from_slice::<OdsReleaseIndex>(&bytes).is_ok() {
                    return Ok(Some(bytes));
                }
            }
        }
        Ok(None)
    }

    fn fetch_release_index(&self) -> Result<Option<OdsReleaseIndex>> {
        if let Some(bytes) = self.fetch_release_index_raw()? {
            if let Ok(idx) = serde_json::from_slice::<OdsReleaseIndex>(&bytes) {
                return Ok(Some(idx));
            }
        }
        Ok(None)
    }
}

fn parse_www_authenticate(header: &str) -> (Option<String>, Option<String>, Option<String>) {
    let mut realm = None;
    let mut service = None;
    let mut scope = None;

    let trimmed = header.trim_start_matches("Bearer ").trim_start_matches("bearer ");
    for part in trimmed.split(',') {
        let part = part.trim();
        if let Some((k, v)) = part.split_once('=') {
            let val = v.trim_matches('"').to_string();
            match k.trim() {
                "realm" => realm = Some(val),
                "service" => service = Some(val),
                "scope" => scope = Some(val),
                _ => {}
            }
        }
    }
    (realm, service, scope)
}

fn fetch_token(realm: &str, service: Option<&str>, scope: Option<&str>) -> Result<String> {
    let mut req = ureq::get(realm).set("User-Agent", "ods-cli");
    if let Some(s) = service {
        req = req.query("service", s);
    }
    if let Some(sc) = scope {
        req = req.query("scope", sc);
    }
    let resp = req.call().context("fetching auth token")?;
    let val: serde_json::Value = resp.into_json().context("parsing token json")?;
    if let Some(tok) = val.get("token").and_then(|t| t.as_str()) {
        Ok(tok.to_string())
    } else if let Some(tok) = val.get("access_token").and_then(|t| t.as_str()) {
        Ok(tok.to_string())
    } else {
        bail!("No token found in auth response");
    }
}

pub fn download_bytes_with_auth(url: &str, initial_token: Option<&str>) -> Result<Vec<u8>> {
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .build();

    let mut current_url = url.to_string();
    let mut current_token = initial_token.map(|s| s.to_string());
    let mut redirect_count = 0;
    let mut auth_retried = false;

    loop {
        let mut req = agent.get(&current_url).set("User-Agent", "ods-cli");
        if let Some(ref tok) = current_token {
            req = req.set("Authorization", &format!("Bearer {}", tok));
        }

        match req.call() {
            Ok(resp) => {
                let mut bytes = Vec::new();
                resp.into_reader().read_to_end(&mut bytes)?;
                return Ok(bytes);
            }
            Err(ureq::Error::Status(401, resp)) => {
                if auth_retried {
                    bail!("401 Unauthorized (token rejected) for URL: {}", current_url);
                }
                if let Some(auth_header) = resp.header("www-authenticate") {
                    let (realm, service, scope) = parse_www_authenticate(auth_header);
                    if let Some(realm_url) = realm {
                        auth_retried = true;
                        let new_token = fetch_token(&realm_url, service.as_deref(), scope.as_deref())?;
                        current_token = Some(new_token);
                        continue;
                    }
                }
                bail!("401 Unauthorized for URL: {}", current_url);
            }
            Err(ureq::Error::Status(301..=308, resp)) => {
                if redirect_count >= 10 {
                    bail!("Too many redirects for {}", url);
                }
                redirect_count += 1;
                let location = resp.header("location")
                    .ok_or_else(|| anyhow::anyhow!("Redirect missing Location header"))?;

                let orig_host = current_url.split("://").nth(1).unwrap_or("").split('/').next().unwrap_or("");
                let new_host = location.split("://").nth(1).unwrap_or("").split('/').next().unwrap_or("");
                if orig_host != new_host {
                    current_token = None;
                }
                current_url = location.to_string();
            }
            Err(e) => return Err(e.into()),
        }
    }
}

pub fn run(args: Args) -> Result<()> {
    let ws = Workspace::open_or_create(None)?;
    let fetcher = HttpOciFetcher;
    run_with_fetcher(args, ws.root(), &fetcher)
}

pub fn run_with_fetcher<F: OciBlobFetcher>(
    args: Args,
    workspace_root: &Path,
    fetcher: &F,
) -> Result<()> {
    run_with_fetcher_and_baked(args, workspace_root, fetcher, None)
}

pub fn run_with_fetcher_and_baked<F: OciBlobFetcher>(
    args: Args,
    workspace_root: &Path,
    fetcher: &F,
    baked_override: Option<OdsReleaseIndex>,
) -> Result<()> {
    let caps = ProgressCaps::detect(args.quiet, args.verbose, args.no_progress);
    let progress = Progress::stderr(caps);

    let _ws = Workspace::open_or_create(Some(workspace_root))?;

    // Step 1: Resolve index
    let (index, _) = resolve_index_with_baked(workspace_root, fetcher, baked_override)?;

    if args.list {
        return list_releases_cmd(workspace_root, &index, &args, &progress);
    }

    if args.all {
        return pull_all_releases_cmd(workspace_root, &index, &args, fetcher, &progress);
    }

    let target_entry = index.resolve(args.release_date.as_deref())?;
    pull_single_release(workspace_root, &index, target_entry, &args, fetcher, &progress)
}

pub fn resolve_index_with_baked<F: OciBlobFetcher>(
    workspace_root: &Path,
    fetcher: &F,
    baked_override: Option<OdsReleaseIndex>,
) -> Result<(OdsReleaseIndex, Option<String>)> {
    let baked = baked_override.unwrap_or_else(|| OdsReleaseIndex::baked().unwrap_or_default());

    // 1. Try fetching remote index
    match fetcher.fetch_release_index() {
        Ok(Some(fetched)) => {
            match baked.merge(&fetched) {
                Ok(merged) => {
                    let raw = fetcher.fetch_release_index_raw()?.unwrap_or_else(|| {
                        serde_json::to_vec_pretty(&merged).unwrap_or_default()
                    });
                    let _ = crate::index::OdsReleaseIndex::save_to_workspace_bytes(&raw, workspace_root);
                    return Ok((merged, Some("just now".to_string())));
                }
                Err(e) => {
                    // Security contradiction on a baked release MUST abort immediately!
                    if e.downcast_ref::<crate::index::SecurityError>().is_some() || e.to_string().contains("Security error") {
                        return Err(e);
                    }
                    // Non-security fetch/parse/validation errors fall back to cache/baked
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
    if let Ok(Some(loaded)) = OdsReleaseIndex::load_from_workspace(workspace_root) {
        match baked.merge(&loaded) {
            Ok(merged) => return Ok((merged, None)),
            Err(e) => {
                if e.downcast_ref::<crate::index::SecurityError>().is_some() || e.to_string().contains("Security error") {
                    return Err(e);
                }
            }
        }
    }

    // 3. Fall back to baked index
    let _ = crate::workspace::ensure_workspace_marker(workspace_root);
    Ok((baked, None))
}

fn list_releases_cmd(
    workspace_root: &Path,
    index: &OdsReleaseIndex,
    args: &Args,
    progress: &Progress,
) -> Result<()> {
    let (local_releases, active_date) = if let Ok(ws) = Workspace::open(Some(workspace_root)) {
        let rels = ws.releases().unwrap_or_default();
        let active = ws.active_release().map(|(d, _)| d).unwrap_or_default();
        (rels, active)
    } else {
        (vec![], String::new())
    };

    let mut items: Vec<ReleaseListItemJson> = Vec::new();

    for r in &index.releases {
        let is_act = !active_date.is_empty() && active_date == r.trud_release_date;
        let is_loc = local_releases.iter().any(|lr| lr.date == r.trud_release_date);
        let status = if let Some(ref reason) = r.withdrawn {
            format!("withdrawn ({})", reason)
        } else if is_act {
            "active (local)".to_string()
        } else if is_loc {
            "local".to_string()
        } else {
            "remote".to_string()
        };

        items.push(ReleaseListItemJson {
            date: r.trud_release_date.clone(),
            version: r.dataset_version.clone(),
            status,
            manifest_digest: Some(r.manifest_digest.clone()),
        });
    }

    if args.format.as_deref() == Some("json") {
        println!("{}", serde_json::to_string_pretty(&items)?);
        return Ok(());
    }

    if progress.caps().is_tty {
        eprintln!("Available ODS dataset releases:\n");
    }

    if items.is_empty() {
        println!("(No dataset releases available in index)");
    } else {
        for item in &items {
            let icon = if item.status.starts_with("active") {
                "●"
            } else if item.status.starts_with("withdrawn") {
                "✖"
            } else {
                "○"
            };
            println!("  {} {:10}  v{:6}  {}", icon, item.date, item.version, item.status);
        }
    }

    if progress.caps().is_tty {
        eprintln!("\nTo pull a release: ods pull <YYYY-MM-DD>");
    }

    Ok(())
}

fn pull_all_releases_cmd<F: OciBlobFetcher>(
    workspace_root: &Path,
    index: &OdsReleaseIndex,
    args: &Args,
    fetcher: &F,
    progress: &Progress,
) -> Result<()> {
    let mut valid_releases: Vec<&ReleaseIndexEntry> = index
        .releases
        .iter()
        .filter(|r| r.withdrawn.is_none())
        .collect();

    if valid_releases.is_empty() {
        bail!("No valid releases available to pull");
    }

    // Sort by trud_release_date and dataset_version ascending
    valid_releases.sort_by(|a, b| {
        a.trud_release_date
            .cmp(&b.trud_release_date)
            .then_with(|| {
                crate::index::parse_semver(&a.dataset_version)
                    .ok()
                    .cmp(&crate::index::parse_semver(&b.dataset_version).ok())
            })
    });

    let mut succeeded = Vec::new();
    let mut failed = Vec::new();

    for entry in &valid_releases {
        match pull_single_release(workspace_root, index, entry, args, fetcher, progress) {
            Ok(()) => succeeded.push(entry.trud_release_date.clone()),
            Err(e) => failed.push((entry.trud_release_date.clone(), e.to_string())),
        }
    }

    if let Some(newest_date) = succeeded.last() {
        let ws = Workspace::open_or_create(Some(workspace_root))?;
        ws.set_active(newest_date)?;
    }

    if !failed.is_empty() && succeeded.is_empty() {
        bail!("Failed to pull all releases: {:?}", failed);
    }

    Ok(())
}

fn pull_single_release<F: OciBlobFetcher>(
    workspace_root: &Path,
    index: &OdsReleaseIndex,
    entry: &ReleaseIndexEntry,
    args: &Args,
    fetcher: &F,
    progress: &Progress,
) -> Result<()> {
    let rel_dir = workspace_root.join("releases").join(&entry.trud_release_date);

    // Cache hit & self-healing check
    if rel_dir.exists() && !args.force {
        let outcome = verify_release_dir(&rel_dir, Some(index));
        match outcome {
            crate::workspace::VerificationOutcome::VerifiedPublished { ref date, ref version, ref digest } => {
                if digest == &entry.manifest_digest {
                    let ws = Workspace::open_or_create(Some(workspace_root))?;
                    ws.set_active(&entry.trud_release_date)?;
                    eprintln!(
                        "✓ {} ({}) verified (cache hit)",
                        date, version
                    );
                    eprintln!("  current → releases/{}", entry.trud_release_date);
                    return Ok(());
                } else {
                    eprintln!(
                        "* existing releases/{} digest mismatch; re-fetching from registry...",
                        entry.trud_release_date
                    );
                    let _ = fs::remove_dir_all(&rel_dir);
                }
            }
            _ => {
                eprintln!(
                    "* existing releases/{} corrupted or invalid; re-fetching from registry...",
                    entry.trud_release_date
                );
                let _ = fs::remove_dir_all(&rel_dir);
            }
        }
    }

    let default_mirrors = vec![
        MirrorEntry {
            url: "https://ods.fyi/v2/ods-data".to_string(),
        },
        MirrorEntry {
            url: "https://ghcr.io/v2/olizilla/ods-data".to_string(),
        },
    ];
    let mirrors = if index.mirrors.is_empty() {
        &default_mirrors[..]
    } else {
        &index.mirrors[..]
    };

    let is_baked = if let Ok(baked) = OdsReleaseIndex::baked() {
        baked
            .releases
            .iter()
            .any(|r| r.trud_release_date == entry.trud_release_date && r.dataset_version == entry.dataset_version)
    } else {
        false
    };

    let mut corroboration_status = None;

    // Task 5: If release is frontier (newer than binary), corroborate with second mirror
    if !is_baked && mirrors.len() >= 2 {
        let mirror1 = &mirrors[0];
        let mirror2 = &mirrors[1];

        let m2_tag_url = mirror2.manifest_url(&entry.tag);
        if let Ok(m2_bytes) = fetcher.fetch_bytes(&m2_tag_url) {
            let m2_digest = format!("sha256:{:x}", sha2::Sha256::digest(&m2_bytes));
            if m2_digest != entry.manifest_digest {
                eprintln!(
                    "✖ {} and {} disagree on {}\n  {:<8} {}\n  {:<8} {}\n  A published (date, version) names one set of bytes. Not proceeding.",
                    mirror1.host(),
                    mirror2.host(),
                    entry.tag,
                    mirror1.host(),
                    entry.manifest_digest,
                    mirror2.host(),
                    m2_digest
                );
                bail!("Manifest digests disagree between mirrors");
            }
            corroboration_status = Some(format!(
                "new release. hash verified by {} and {}",
                mirror1.host(),
                mirror2.host()
            ));
        } else {
            corroboration_status = Some(format!(
                "new release. hash from {} only, not corroborated",
                mirror1.host()
            ));
        }
    } else if !is_baked {
        corroboration_status = Some(format!(
            "new release. hash from {} only, not corroborated",
            mirrors[0].host()
        ));
    }

    let scratch_dir = workspace_root.join("scratch");
    fs::create_dir_all(&scratch_dir)?;
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let temp_path = scratch_dir.join(format!("pull_{}_{}", entry.trud_release_date, timestamp));
    if temp_path.exists() {
        let _ = fs::remove_dir_all(&temp_path);
    }
    fs::create_dir_all(&temp_path)?;

    let mut last_err = None;

    for mirror in mirrors {
        progress.step(&format!(
            "{} ({}) fetching manifest from {}…",
            entry.trud_release_date,
            entry.dataset_version,
            mirror.host()
        ));

        // 1. Fetch manifest
        let manifest_url = mirror.manifest_url(&entry.manifest_digest);
        let manifest_bytes = match fetcher.fetch_bytes(&manifest_url) {
            Ok(b) => b,
            Err(e) => {
                last_err = Some(format!("{}: {}", mirror.host(), e));
                continue;
            }
        };

        let computed_digest = format!("sha256:{:x}", sha2::Sha256::digest(&manifest_bytes));
        if computed_digest != entry.manifest_digest {
            last_err = Some(format!(
                "{}: manifest digest mismatch: computed {}, expected {}",
                mirror.host(),
                computed_digest,
                entry.manifest_digest
            ));
            continue;
        }

        let manifest: OciManifest = match serde_json::from_slice(&manifest_bytes) {
            Ok(m) => m,
            Err(e) => {
                last_err = Some(format!("{}: invalid manifest JSON: {}", mirror.host(), e));
                continue;
            }
        };

        // 2. Fetch each layer
        let mut failed_layer = false;
        for layer in &manifest.layers {
            let filename = match layer
                .annotations
                .as_ref()
                .and_then(|a| a.get(ANNOTATION_TITLE))
            {
                Some(f) => f,
                None => {
                    failed_layer = true;
                    last_err = Some(format!("{}: layer missing title annotation", mirror.host()));
                    break;
                }
            };

            progress.step(&format!(
                "{} ({}) downloading {} from {}…",
                entry.trud_release_date,
                entry.dataset_version,
                filename,
                mirror.host()
            ));

            let blob_url = mirror.blob_url(&layer.digest);
            let layer_bytes = match fetcher.fetch_bytes(&blob_url) {
                Ok(b) => b,
                Err(e) => {
                    failed_layer = true;
                    last_err = Some(format!("{}: failed to download {}: {}", mirror.host(), filename, e));
                    break;
                }
            };

            let computed_layer_digest = format!("sha256:{:x}", sha2::Sha256::digest(&layer_bytes));
            if computed_layer_digest != layer.digest {
                failed_layer = true;
                last_err = Some(format!(
                    "{}: layer {} checksum mismatch: computed {}, expected {}",
                    mirror.host(),
                    filename,
                    computed_layer_digest,
                    layer.digest
                ));
                break;
            }

            if let Err(e) = fs::write(temp_path.join(filename), &layer_bytes) {
                failed_layer = true;
                last_err = Some(format!("Failed to write {}: {}", filename, e));
                break;
            }
        }

        if failed_layer {
            continue;
        }

        // 3. Verify assembled directory
        let outcome = verify_release_dir(&temp_path, Some(index));
        if !outcome.is_verified() {
            last_err = Some(format!("{}: assembled release directory failed verification", mirror.host()));
            continue;
        }

        // 4. Atomic install
        fs::create_dir_all(workspace_root.join("releases"))?;
        if rel_dir.exists() {
            fs::remove_dir_all(&rel_dir)?;
        }
        fs::rename(&temp_path, &rel_dir)?;

        // 5. Pin current and generate README
        let ws = Workspace::open_or_create(Some(workspace_root))?;
        ws.set_active(&entry.trud_release_date)?;

        let total_size: u64 = manifest.layers.iter().map(|l| l.size).sum();
        let size_mb = (total_size as f64) / (1024.0 * 1024.0);

        let verification_msg = if is_baked {
            format!("hash verified by ods {}", env!("CARGO_PKG_VERSION"))
        } else {
            corroboration_status.unwrap_or_else(|| format!("from {}", mirror.host()))
        };

        eprintln!(
            "✓ {} ({})  {:.0}MB  {}",
            entry.trud_release_date,
            entry.dataset_version,
            size_mb,
            verification_msg
        );
        eprintln!("  current → releases/{}", entry.trud_release_date);

        return Ok(());
    }

    bail!(
        "✖ All mirrors failed to pull release {} ({}): {}",
        entry.trud_release_date,
        entry.dataset_version,
        last_err.unwrap_or_else(|| "no reachable mirrors".to_string())
    )
}

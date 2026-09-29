use anyhow::{bail, Context, Result};
use clap::Parser;
use serde::Serialize;
use sha2::Digest;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::index::{Dataset, MirrorEntry, OdsReleaseIndex, Release};
use crate::oci::*;
use crate::progress::{
    render_release_block, Progress, ProgressCaps, ReleaseBlockLink, ReleaseBlockParams,
    ReleaseBlockState,
};
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

    /// Read the release index from this path or URL instead of the network
    #[arg(long, hide = true)]
    pub index: Option<String>,

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
    /// The release's `source.version`: TRUD's release date.
    pub source_version: String,
    /// The dataset's `version`, `<source version>_<dataset version>`.
    pub version: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexOrigin {
    Flag(String),
    Fetched(String),
    WorkspaceCache(PathBuf),
    BuiltIn,
}

impl IndexOrigin {
    /// The two default endpoints `resolve_index` tries when no `--index` and no
    /// `ODS_RELEASE_INDEX_URL` override it: the published index, referred to by its
    /// filename rather than a URL, since that's the common case.
    const DEFAULT_FETCH_URLS: [&'static str; 2] = [
        "https://ods.fyi/releases.json",
        "https://raw.githubusercontent.com/olizilla/ods/main/data/releases.json",
    ];

    /// The `verified` row's value: what index the digests came from. Renders the
    /// origin rather than inventing wording for it — `sha256 from releases.json` for
    /// the published index fetched from one of its two default endpoints, and the
    /// path or URL verbatim for anything else (a `--index` value, a custom
    /// `ODS_RELEASE_INDEX_URL`, or the workspace's own cache), so a pull checked
    /// against a local index reads differently from a published one.
    pub fn verified_row(&self) -> String {
        let source = match self {
            Self::Flag(v) => v.clone(),
            Self::Fetched(url) if Self::DEFAULT_FETCH_URLS.contains(&url.as_str()) => "releases.json".to_string(),
            Self::Fetched(url) => url.clone(),
            Self::WorkspaceCache(p) => crate::workspace::relative_to_cwd(p).display().to_string(),
            Self::BuiltIn => "the index built into ods".to_string(),
        };
        format!("sha256 from {}", source)
    }
}

impl std::fmt::Display for IndexOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Flag(v) => write!(f, "--index {}", v),
            Self::Fetched(url) => write!(f, "{}", url),
            Self::WorkspaceCache(p) => write!(f, "{}", crate::workspace::relative_to_cwd(p).display()),
            Self::BuiltIn => write!(f, "built-in"),
        }
    }
}

pub trait OciBlobFetcher: Send + Sync {
    fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>>;
    /// Like `fetch_bytes`, reporting each chunk's length to `on_bytes` as it arrives, so a
    /// caller can drive a bar mid-download. Defaults to the plain fetch, reporting the
    /// whole thing in one call once it's done.
    fn fetch_bytes_with_progress(
        &self,
        url: &str,
        on_bytes: &(dyn Fn(u64) + Send + Sync),
    ) -> Result<Vec<u8>> {
        let bytes = self.fetch_bytes(url)?;
        on_bytes(bytes.len() as u64);
        Ok(bytes)
    }
    fn fetch_release_index(&self) -> Result<Option<OdsReleaseIndex>> {
        if let Some((bytes, _)) = self.fetch_release_index_raw()? {
            if let Ok(idx) = OdsReleaseIndex::from_slice(&bytes) {
                return Ok(Some(idx));
            }
        }
        Ok(None)
    }
    fn fetch_release_index_raw(&self) -> Result<Option<(Vec<u8>, String)>> {
        if let Some(idx) = self.fetch_release_index()? {
            return Ok(Some((serde_json::to_vec_pretty(&idx)?, "remote".to_string())));
        }
        Ok(None)
    }
}

pub struct HttpOciFetcher;

impl OciBlobFetcher for HttpOciFetcher {
    fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>> {
        download_bytes_with_auth(url, None)
    }

    fn fetch_bytes_with_progress(
        &self,
        url: &str,
        on_bytes: &(dyn Fn(u64) + Send + Sync),
    ) -> Result<Vec<u8>> {
        download_bytes_with_auth_and_progress(url, None, on_bytes)
    }

    fn fetch_release_index_raw(&self) -> Result<Option<(Vec<u8>, String)>> {
        let custom_url = std::env::var("ODS_RELEASE_INDEX_URL").ok();
        let mut index_urls: Vec<&str> = Vec::new();
        if let Some(ref u) = custom_url {
            index_urls.push(u.as_str());
        } else {
            index_urls.extend_from_slice(&IndexOrigin::DEFAULT_FETCH_URLS);
        }
        for url in &index_urls {
            if let Ok(bytes) = download_bytes_with_auth(url, None) {
                let valid = OdsReleaseIndex::from_slice(&bytes)
                    .map(|idx| idx.validate().is_ok())
                    .unwrap_or(false);
                if valid {
                    return Ok(Some((bytes, url.to_string())));
                } else if crate::index::is_old_format(&bytes) {
                    eprintln!("! Ignoring {}: {}", url, crate::index::OldFormatIndex);
                } else {
                    eprintln!("! Ignoring {}: it isn't a release index this ods can read", url);
                }
            }
        }
        Ok(None)
    }

    fn fetch_release_index(&self) -> Result<Option<OdsReleaseIndex>> {
        if let Some((bytes, _)) = self.fetch_release_index_raw()? {
            if let Ok(idx) = OdsReleaseIndex::from_slice(&bytes) {
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
    download_bytes_with_auth_and_progress(url, initial_token, &|_| {})
}

/// Like `download_bytes_with_auth`, reporting each chunk's length to `on_bytes` as it
/// arrives. A redirect or the 401 token retry restarts the loop before any body is read,
/// so `on_bytes` only ever sees bytes that count toward this call's own final response —
/// never double-counted across a retry.
pub fn download_bytes_with_auth_and_progress(
    url: &str,
    initial_token: Option<&str>,
    on_bytes: &(dyn Fn(u64) + Send + Sync),
) -> Result<Vec<u8>> {
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .build();

    let mut current_url = url.to_string();
    let mut current_token = initial_token.map(|s| s.to_string());
    let mut redirect_count = 0;
    let mut auth_retried = false;

    loop {
        let mut req = agent.get(&current_url).set("User-Agent", "ods-cli");
        // A registry serves an OCI manifest only to a client that says it accepts one: ghcr.io
        // answers 404 MANIFEST_UNKNOWN to ureq's default `Accept: */*`. Blob requests keep it.
        if current_url.contains("/manifests/") {
            req = req.set("Accept", crate::oci::MEDIA_TYPE_MANIFEST);
        }
        if let Some(ref tok) = current_token {
            req = req.set("Authorization", &format!("Bearer {}", tok));
        }

        match req.call() {
            // ureq returns a 3xx as Ok when it isn't following redirects itself (this agent has
            // redirects(0)). ghcr.io answers every blob with a 307 to its storage host.
            Ok(resp) | Err(ureq::Error::Status(_, resp)) if (300..400).contains(&resp.status()) => {
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
            Ok(resp) => {
                let mut bytes = Vec::new();
                let mut reader = resp.into_reader();
                let mut buf = [0u8; 64 * 1024];
                loop {
                    let n = reader.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buf[..n]);
                    on_bytes(n as u64);
                }
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
            Err(e) => return Err(e.into()),
        }
    }
}

pub fn run(args: Args) -> Result<()> {
    let fetcher = HttpOciFetcher;
    // Find a workspace without creating one: `--list` only reads, and
    // `run_with_fetcher_and_writer` earns a workspace itself when installing.
    let workspace_root = Workspace::open(None)
        .map(|ws| ws.root().to_path_buf())
        .unwrap_or_else(|_| PathBuf::from(crate::workspace::DEFAULT_WORKSPACE_DIR));
    run_with_fetcher(args, &workspace_root, &fetcher)
}

pub fn run_with_fetcher<F: OciBlobFetcher>(
    args: Args,
    workspace_root: &Path,
    fetcher: &F,
) -> Result<()> {
    let mut stderr = std::io::stderr();
    run_with_fetcher_and_writer(args, workspace_root, fetcher, &mut stderr)
}

pub fn run_with_fetcher_and_writer<F: OciBlobFetcher, W: Write>(
    args: Args,
    workspace_root: &Path,
    fetcher: &F,
    writer: &mut W,
) -> Result<()> {
    let caps = ProgressCaps::detect(args.quiet, args.verbose, args.no_progress);
    let progress = Progress::stderr(caps);

    // `--list` only reads: earn a workspace when installing a release, not to answer
    // a question about what's available. When one already exists, behave as today.
    let workspace_exists = Workspace::open(Some(workspace_root)).is_ok();
    if !args.list {
        Workspace::open_or_create(Some(workspace_root))?;
    }

    // Step 1: Resolve index
    let (index, origin) = resolve_index(
        workspace_root,
        args.index.as_deref(),
        true,
        !args.list || workspace_exists,
        fetcher,
    )?;

    let mut ctx = PullContext {
        args: &args,
        fetcher,
        progress: &progress,
        writer,
        origin: &origin,
    };

    if args.list {
        return list_releases_cmd(workspace_root, &index, &args, &progress);
    }

    if args.all {
        return pull_all_releases_cmd(workspace_root, &index, &mut ctx);
    }

    let resolved = index.resolve(args.release_date.as_deref())?;
    for (skipped_rel, skipped_ds) in &resolved.skipped {
        let reason = skipped_ds.withdrawn.as_deref().unwrap_or("withdrawn by maintainer");
        writeln!(
            ctx.writer,
            "! {} ({}) was withdrawn: {}\n  Pulling {} instead",
            skipped_rel.source.version,
            skipped_ds.dataset_version(),
            reason,
            resolved.release.source.version,
        )?;
    }
    pull_single_release(
        workspace_root,
        &index,
        resolved.release,
        resolved.dataset,
        &mut ctx,
    )
}

pub fn resolve_index<F: OciBlobFetcher + ?Sized>(
    workspace_root: &Path,
    index_arg: Option<&str>,
    allow_fetch: bool,
    create: bool,
    fetcher: &F,
) -> Result<(OdsReleaseIndex, IndexOrigin)> {
    // 1. --index <val>
    if let Some(val) = index_arg {
        let is_http = val.starts_with("http://") || val.starts_with("https://");
        let bytes = if is_http {
            match fetcher.fetch_bytes(val) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("✖ Cannot read release index '{}': {}", val, e);
                    return Err(AlreadyReported.into());
                }
            }
        } else {
            match fs::read(val) {
                Ok(b) => b,
                Err(e) => {
                    let err_str = e.to_string();
                    let clean_err = err_str.split(" (os error").next().unwrap_or(&err_str);
                    eprintln!("✖ Cannot read release index '{}': {}", val, clean_err);
                    return Err(AlreadyReported.into());
                }
            }
        };

        let fetched_index: OdsReleaseIndex = match OdsReleaseIndex::from_slice(&bytes) {
            Ok(idx) => idx,
            Err(e) if e.downcast_ref::<crate::index::OldFormatIndex>().is_some() => {
                eprintln!("{}", format_old_format_index_flag(val));
                return Err(AlreadyReported.into());
            }
            Err(_) => {
                eprintln!("✖ Cannot parse release index '{}' as an ODS release index", val);
                eprintln!("  Expected $schema https://ods.fyi/schema/releases.v1.json");
                return Err(AlreadyReported.into());
            }
        };

        if let Err(e) = fetched_index.validate() {
            eprintln!("✖ Cannot parse release index '{}' as an ODS release index", val);
            if fetched_index.schema != crate::index::RELEASES_SCHEMA_V1_URL {
                eprintln!("  Expected $schema https://ods.fyi/schema/releases.v1.json");
            } else {
                eprintln!("  {}", e);
            }
            return Err(AlreadyReported.into());
        }

        return Ok((fetched_index, IndexOrigin::Flag(val.to_string())));
    }

    // 2. Remote fetch (if allowed)
    if allow_fetch {
        if let Ok(Some((raw_bytes, url))) = fetcher.fetch_release_index_raw() {
            if let Ok(fetched) = OdsReleaseIndex::from_slice(&raw_bytes) {
                if fetched.validate().is_ok() {
                    if create {
                        let _ = crate::index::OdsReleaseIndex::save_to_workspace_bytes(&raw_bytes, workspace_root);
                    }
                    return Ok((fetched, IndexOrigin::Fetched(url)));
                } else {
                    eprintln!("! Ignoring {}: it isn't a release index this ods can read", url);
                }
            }
        }
    }

    // 3. Workspace cache
    let cache_path = workspace_root.join(crate::index::RELEASES_JSON_FILENAME);
    if cache_path.exists() {
        let cache_bytes = fs::read(&cache_path).ok();
        if cache_bytes.as_deref().is_some_and(crate::index::is_old_format) {
            eprintln!("{}", format_old_format_cache(&cache_path));
            return Err(AlreadyReported.into());
        }
        let valid_cache = match cache_bytes {
            Some(bytes) => match OdsReleaseIndex::from_slice(&bytes) {
                Ok(idx) if idx.validate().is_ok() => Some(idx),
                _ => None,
            },
            None => None,
        };
        if let Some(loaded) = valid_cache {
            return Ok((loaded, IndexOrigin::WorkspaceCache(cache_path)));
        } else {
            crate::workspace::emit_cache_notice_if_needed(workspace_root);
        }
    }

    // 4. Built-in baked index
    if create {
        let _ = crate::workspace::ensure_workspace_marker(workspace_root);
    }
    let baked = OdsReleaseIndex::baked()?;
    Ok((baked, IndexOrigin::BuiltIn))
}

/// The refusal for `--index <file>` in the old format.
pub fn format_old_format_index_flag(val: &str) -> String {
    format!(
        "✖ Cannot read release index '{}': it's in the old format, with trud_release_date and dataset_version\n  This ods reads the current format: name, source, and releases[].source.version, .hash and .bytes",
        val
    )
}

/// The refusal for a workspace cache in the old format: `ods pull` replaces it.
pub fn format_old_format_cache(path: &Path) -> String {
    format!(
        "✖ Cannot read {}: it's a release index in the old format\n  Refresh it: ods pull",
        crate::workspace::relative_to_cwd(path).display()
    )
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
        for ds in &r.datasets {
            let is_act = !active_date.is_empty() && active_date == r.source.version;
            let is_loc = local_releases.iter().any(|lr| lr.date == r.source.version);
            let status = if let Some(ref reason) = ds.withdrawn {
                format!("withdrawn ({})", reason)
            } else if is_act {
                "active (local)".to_string()
            } else if is_loc {
                "local".to_string()
            } else {
                "remote".to_string()
            };

            items.push(ReleaseListItemJson {
                source_version: r.source.version.clone(),
                version: ds.version.clone(),
                status,
                manifest_digest: Some(ds.manifest_digest.clone()),
            });
        }
    }

    if args.format.as_deref() == Some("json") {
        println!("{}", serde_json::to_string_pretty(&items)?);
        return Ok(());
    }

    println!("Available ODS dataset releases:\n");

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
            let dataset_version = item.version.split_once('_').map_or(item.version.as_str(), |(_, v)| v);
            println!("  {} {:10}  v{:6}  {}", icon, item.source_version, dataset_version, item.status);
        }
    }

    if progress.caps().is_tty {
        eprintln!("\nTo pull a release: ods pull <YYYY-MM-DD>");
    }

    Ok(())
}

fn format_fetch_error(err: &anyhow::Error) -> String {
    if let Some(ureq_err) = err.downcast_ref::<ureq::Error>() {
        match ureq_err {
            ureq::Error::Status(code, _) => return format!("HTTP {}", code),
            ureq::Error::Transport(t) => return format!("transport error: {}", t),
        }
    }
    err.to_string()
}

/// Writes a settled release block through `ctx.writer`, the way every other line
/// `pull_single_release` prints does — rather than through `Progress`'s own writer, which
/// in production is real stderr regardless, but in an in-process test is not the buffer a
/// caller passed to `run_with_fetcher_and_writer`. Clears any live TTY animation first.
fn write_block<W: Write>(writer: &mut W, progress: &Progress, lines: &[String]) -> Result<()> {
    progress.clear_live();
    for line in lines {
        writeln!(writer, "{}", line)?;
    }
    Ok(())
}

/// Names each known issue with a release's source under its block, with the page that says
/// how `ods` resolves it (docs/source-issues/).
fn write_source_issues<W: Write>(writer: &mut W, release: &Release) -> Result<()> {
    for id in &release.source.issues {
        writeln!(writer, "{}", crate::index::source_issue_line(id))?;
    }
    Ok(())
}

/// Moves the `current` pin to `release_date` if it isn't there already, reporting
/// whether it moved — the same check `ods trud pull` uses to render `(unchanged)`.
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

/// Counts the files a pulled release directory holds, the way the manifest is rebuilt from it:
/// its Parquet files, so a cache-hit block reports the same file count a fresh pull would.
fn count_release_files(release_dir: &Path) -> usize {
    crate::provenance::release_parquet_files(release_dir).map(|names| names.len()).unwrap_or(0)
}

struct PullContext<'a, F, W> {
    pub args: &'a Args,
    pub fetcher: &'a F,
    pub progress: &'a Progress,
    pub writer: &'a mut W,
    pub origin: &'a IndexOrigin,
}

/// Whether `release_dir` already holds `ds`, verified against `index` — the same check
/// `pull_single_release`'s own cache hit makes, used here to plan `--all` without touching
/// the network.
fn release_is_cached(workspace_root: &Path, index: &OdsReleaseIndex, rel: &Release, ds: &Dataset, force: bool) -> bool {
    if force {
        return false;
    }
    let rel_dir = workspace_root.join("releases").join(&rel.source.version);
    if !rel_dir.exists() {
        return false;
    }
    matches!(
        verify_release_dir(&rel_dir, index),
        crate::workspace::VerificationOutcome::VerifiedPublished { ref digest, .. } if *digest == ds.manifest_digest
    )
}

/// An incomplete block's own `current is still …` / `current is not set` line is computed
/// against the workspace's active release at the moment `pull_single_release` runs — which,
/// inside `--all`, is before the batch's one pin move. Once the pin has actually moved,
/// swap that one line for the truth rather than replaying a stale one.
fn recompute_current_line(text: &str, active_date: Option<&str>) -> String {
    let fresh = match active_date {
        Some(d) => format!("  current is still releases/{}", d),
        None => "  current is not set".to_string(),
    };
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        if line.starts_with("  current is still releases/") || line == "  current is not set" {
            out.push_str(&fresh);
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

enum BatchFailure {
    /// Arrived incomplete: `pull_single_release` already rendered its `✖ … arrived
    /// incomplete` block into a buffer instead of `ctx.writer`, captured here verbatim
    /// except for its `current is still …` line, patched once the pin is decided.
    Incomplete { text: String },
    /// Every mirror failed before a single byte was verified: the error `pull_single_release`
    /// returned, printed with the same retry line the incomplete block uses.
    Unreachable { date: String, message: String },
}

fn pull_all_releases_cmd<F: OciBlobFetcher, W: Write>(
    workspace_root: &Path,
    index: &OdsReleaseIndex,
    ctx: &mut PullContext<'_, F, W>,
) -> Result<()> {
    let mut valid_releases: Vec<(&Release, &Dataset)> = Vec::new();
    for r in &index.releases {
        for ds in &r.datasets {
            if ds.withdrawn.is_none() {
                valid_releases.push((r, ds));
            }
        }
    }

    if valid_releases.is_empty() {
        bail!("No valid releases available to pull");
    }

    // Sort by source version, then dataset version, ascending
    valid_releases.sort_by(|(r_a, ds_a), (r_b, ds_b)| {
        r_a.source.version
            .cmp(&r_b.source.version)
            .then_with(|| {
                ds_a.semver().cmp(&ds_b.semver())
            })
    });

    let cached_flags: Vec<bool> = valid_releases
        .iter()
        .map(|(r, ds)| release_is_cached(workspace_root, index, r, ds, ctx.args.force))
        .collect();
    let cached_count = cached_flags.iter().filter(|c| **c).count();
    let to_pull_count = valid_releases.len() - cached_count;
    let to_pull_bytes: u64 = valid_releases
        .iter()
        .zip(&cached_flags)
        .filter(|(_, cached)| !**cached)
        .map(|((_, ds), _)| ds.bytes)
        .sum();

    writeln!(
        ctx.writer,
        "{} cached · {} to pull · {}",
        cached_count,
        to_pull_count,
        crate::progress::format_size(to_pull_bytes)
    )?;
    writeln!(ctx.writer)?;

    let batch_start = std::time::Instant::now();
    let mut succeeded: Vec<String> = Vec::new();
    let mut pulled_count = 0usize;
    let mut pulled_bytes = 0u64;
    let mut failures: Vec<BatchFailure> = Vec::new();

    for (rel, ds) in &valid_releases {
        let mut buffer: Vec<u8> = Vec::new();
        let mut nested_ctx = PullContext {
            args: ctx.args,
            fetcher: ctx.fetcher,
            progress: ctx.progress,
            writer: &mut buffer,
            origin: ctx.origin,
        };
        let result = pull_single_release(workspace_root, index, rel, ds, &mut nested_ctx);
        let text = String::from_utf8_lossy(&buffer).into_owned();
        match result {
            Ok(()) => {
                succeeded.push(rel.source.version.clone());
                if !text.is_empty() {
                    write!(ctx.writer, "{}", text)?;
                    pulled_count += 1;
                    pulled_bytes += ds.bytes;
                }
            }
            Err(e) => {
                if e.chain().any(|c| c.downcast_ref::<AlreadyReported>().is_some()) {
                    failures.push(BatchFailure::Incomplete { text });
                } else {
                    failures.push(BatchFailure::Unreachable {
                        date: rel.source.version.clone(),
                        message: e.to_string(),
                    });
                }
            }
        }
    }

    // The pin moves once, onto the newest release that verified in full — cached or freshly
    // pulled, since `valid_releases` runs oldest to newest.
    let pin_target = succeeded.last().cloned();
    if let Some(ref target) = pin_target {
        update_active_release_link_if_changed(workspace_root, target)?;
    }
    let final_active = pin_target.clone().or_else(|| {
        Workspace::open(Some(workspace_root))
            .ok()
            .and_then(|ws| ws.active_release().ok())
            .map(|(d, _)| d)
    });

    let elapsed = batch_start.elapsed();
    let second_count = if failures.is_empty() {
        format!("{} cached", cached_count)
    } else {
        format!("{} failed", failures.len())
    };
    writeln!(ctx.writer)?;
    writeln!(
        ctx.writer,
        "  {:<14}{} releases · {} · {}  in {}",
        "pulled",
        pulled_count,
        second_count,
        crate::progress::format_size(pulled_bytes),
        crate::progress::format_elapsed(elapsed)
    )?;
    if let Some(ref d) = final_active {
        writeln!(ctx.writer, "  {:<14}current → releases/{}", "linked", d)?;
    }

    for failure in &failures {
        match failure {
            BatchFailure::Incomplete { text } => {
                write!(ctx.writer, "{}", recompute_current_line(text, final_active.as_deref()))?;
            }
            BatchFailure::Unreachable { date, message } => {
                writeln!(ctx.writer, "{}", message)?;
                writeln!(ctx.writer, "  Retry: ods pull {}", date)?;
            }
        }
    }

    if !failures.is_empty() {
        return Err(AlreadyReported.into());
    }

    Ok(())
}

fn pull_single_release<F: OciBlobFetcher, W: Write>(
    workspace_root: &Path,
    index: &OdsReleaseIndex,
    release: &Release,
    dataset: &Dataset,
    ctx: &mut PullContext<'_, F, W>,
) -> Result<()> {
    let progress = ctx.progress;
    let rel_dir = workspace_root.join("releases").join(&release.source.version);
    let link_target = format!("releases/{}", release.source.version);
    let dataset_coord = format!("ods-data/{}", dataset.version);
    let color = progress.caps().is_tty && !progress.caps().no_color;

    // Cache hit & self-healing check
    if rel_dir.exists() && !ctx.args.force {
        let outcome = verify_release_dir(&rel_dir, index);
        match outcome {
            crate::workspace::VerificationOutcome::VerifiedPublished { digest, .. } => {
                if digest == dataset.manifest_digest {
                    // `--all` moves the pin once, after the whole batch, and doesn't list a
                    // cached release at all (it's already counted in the plan line).
                    let pin_moved = if ctx.args.all {
                        false
                    } else {
                        update_active_release_link_if_changed(workspace_root, &release.source.version)?
                    };
                    if !ctx.args.all {
                        let hash_opt = if ctx.args.verbose { Some(dataset.manifest_digest.as_str()) } else { None };
                        let lines = render_release_block(&ReleaseBlockParams {
                            date: &release.source.version,
                            archive_size: dataset.bytes,
                            file_count: count_release_files(&rel_dir),
                            state: &ReleaseBlockState::Cached,
                            dataset: Some(&dataset_coord),
                            verified: &ctx.origin.verified_row(),
                            linked: Some(ReleaseBlockLink { target: &link_target, unchanged: !pin_moved }),
                            from: None,
                            hash: hash_opt,
                            hash_label: "manifest",
                            color,
                        });
                        write_block(ctx.writer, progress, &lines)?;
                        write_source_issues(ctx.writer, release)?;
                    }
                    if let Some(ref reason) = dataset.withdrawn {
                        writeln!(
                            ctx.writer,
                            "✖ {} ({}) was withdrawn: {}\n  Pull a valid release: ods pull",
                            release.source.version,
                            dataset.dataset_version(),
                            reason
                        )?;
                        return Err(AlreadyReported.into());
                    }
                    return Ok(());
                } else {
                    writeln!(
                        ctx.writer,
                        "* existing releases/{} digest mismatch; re-fetching from registry...",
                        release.source.version
                    )?;
                    let _ = fs::remove_dir_all(&rel_dir);
                }
            }
            _ => {
                writeln!(
                    ctx.writer,
                    "* existing releases/{} corrupted or invalid; re-fetching from registry...",
                    release.source.version
                )?;
                let _ = fs::remove_dir_all(&rel_dir);
            }
        }
    }

    let default_mirrors = [
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

    let scratch_dir = workspace_root.join("scratch");
    fs::create_dir_all(&scratch_dir)?;
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let temp_path = scratch_dir.join(format!("pull_{}_{}", release.source.version, timestamp));
    if temp_path.exists() {
        let _ = fs::remove_dir_all(&temp_path);
    }
    fs::create_dir_all(&temp_path)?;

    struct StagingGuard<'a> {
        path: &'a Path,
        installed: bool,
    }
    impl<'a> Drop for StagingGuard<'a> {
        fn drop(&mut self) {
            if !self.installed && self.path.exists() {
                let _ = fs::remove_dir_all(self.path);
            }
        }
    }
    let mut staging_guard = StagingGuard {
        path: &temp_path,
        installed: false,
    };

    // 1. Fetch manifest from first mirror whose manifest hashes to dataset.manifest_digest
    let mut manifest_opt = None;
    let mut manifest_mirror = None;
    let mut manifest_err = None;

    for mirror in mirrors {
        let lines = render_release_block(&ReleaseBlockParams {
            date: &release.source.version,
            archive_size: dataset.bytes,
            file_count: 0,
            state: &ReleaseBlockState::Preparing {
                what: format!("fetching manifest from {}…", mirror.host()),
            },
            dataset: Some(&dataset_coord),
            verified: "",
            linked: Some(ReleaseBlockLink { target: &link_target, unchanged: false }),
            from: None,
            hash: None,
            hash_label: "manifest",
            color,
        });
        progress.update_live_block(lines);

        let manifest_url = mirror.manifest_url(&dataset.manifest_digest);
        let manifest_bytes = match ctx.fetcher.fetch_bytes(&manifest_url) {
            Ok(b) => b,
            Err(e) => {
                manifest_err = Some(format!("{}: {}", mirror.host(), e));
                continue;
            }
        };

        let computed_digest = format!("sha256:{:x}", sha2::Sha256::digest(&manifest_bytes));
        if computed_digest != dataset.manifest_digest {
            manifest_err = Some(format!(
                "{}: manifest digest mismatch: computed {}, expected {}",
                mirror.host(),
                computed_digest,
                dataset.manifest_digest
            ));
            continue;
        }

        let manifest: OciManifest = match serde_json::from_slice(&manifest_bytes) {
            Ok(m) => m,
            Err(e) => {
                manifest_err = Some(format!("{}: invalid manifest JSON: {}", mirror.host(), e));
                continue;
            }
        };

        manifest_opt = Some(manifest);
        manifest_mirror = Some(mirror);
        break;
    }

    let manifest = match (manifest_opt, manifest_mirror) {
        (Some(m), Some(_)) => m,
        _ => {
            bail!(
                "✖ All mirrors failed to pull release {} ({}): {}",
                release.source.version,
                dataset.dataset_version(),
                manifest_err.unwrap_or_else(|| "no reachable mirrors".to_string())
            );
        }
    };

    // 2. Fetch each layer from any mirror that serves it correctly
    struct FailedLayer {
        filename: String,
        expected_digest: String,
        mirror_attempts: Vec<(String, String)>,
        bad_bytes: Option<Vec<u8>>,
    }

    let mut failed_layers = Vec::new();
    // Summed across every layer, not reset per-layer, so the bar measures the whole
    // release rather than the one in hand. A redirect or the 401 retry inside
    // `fetch_bytes_with_progress` restarts before any body is read, so a retried blob
    // is never double-counted.
    let bytes_downloaded = std::sync::atomic::AtomicU64::new(0);
    let mut served_from: Vec<String> = Vec::new();
    let download_start = std::time::Instant::now();
    let file_count = manifest.layers.len();

    for layer in &manifest.layers {
        let filename = match layer
            .annotations
            .as_ref()
            .and_then(|a| a.get(ANNOTATION_TITLE))
        {
            Some(f) => f.to_string(),
            None => {
                failed_layers.push(FailedLayer {
                    filename: "unknown".to_string(),
                    expected_digest: layer.digest.clone(),
                    mirror_attempts: vec![("all".to_string(), "layer missing title annotation".to_string())],
                    bad_bytes: None,
                });
                continue;
            }
        };

        let mut layer_verified = false;
        let mut mirror_attempts = Vec::new();
        let mut first_bad_bytes = None;

        for mirror in mirrors {
            let blob_url = mirror.blob_url(&layer.digest);
            let layer_bytes_before = bytes_downloaded.load(std::sync::atomic::Ordering::Relaxed);
            let result = ctx.fetcher.fetch_bytes_with_progress(&blob_url, &|n| {
                let cur = bytes_downloaded.fetch_add(n, std::sync::atomic::Ordering::Relaxed) + n;
                let elapsed = download_start.elapsed().as_secs_f64();
                let rate = if elapsed > 0.0 { cur as f64 / elapsed } else { 0.0 };
                let total = dataset.bytes;
                let eta = if rate > 0.0 && total > cur {
                    Some(std::time::Duration::from_secs_f64((total - cur) as f64 / rate))
                } else {
                    None
                };
                let lines = render_release_block(&ReleaseBlockParams {
                    date: &release.source.version,
                    archive_size: total,
                    file_count,
                    state: &ReleaseBlockState::Downloading {
                        bytes_done: cur,
                        rate: Some(rate),
                        eta,
                    },
                    dataset: Some(&dataset_coord),
                    verified: "",
                    linked: Some(ReleaseBlockLink { target: &link_target, unchanged: false }),
                    from: None,
                    hash: None,
                    hash_label: "manifest",
                    color,
                });
                progress.update_live_block(lines);
            });
            match result {
                Ok(bytes) => {
                    let computed = format!("sha256:{:x}", sha2::Sha256::digest(&bytes));
                    if computed == layer.digest {
                        if let Err(e) = fs::write(temp_path.join(&filename), &bytes) {
                            mirror_attempts.push((mirror.host().to_string(), format!("write error: {}", e)));
                            let this_attempt = bytes_downloaded.load(std::sync::atomic::Ordering::Relaxed) - layer_bytes_before;
                            bytes_downloaded.fetch_sub(this_attempt, std::sync::atomic::Ordering::Relaxed);
                            continue;
                        }
                        layer_verified = true;
                        served_from.push(mirror.host());
                        break;
                    } else {
                        mirror_attempts.push((mirror.host().to_string(), computed));
                        if first_bad_bytes.is_none() {
                            first_bad_bytes = Some(bytes);
                        }
                        // Wrong content: don't let a failed mirror's bytes count toward
                        // the total, so it still equals the manifest's layer sizes.
                        let this_attempt = bytes_downloaded.load(std::sync::atomic::Ordering::Relaxed) - layer_bytes_before;
                        bytes_downloaded.fetch_sub(this_attempt, std::sync::atomic::Ordering::Relaxed);
                    }
                }
                Err(e) => {
                    let this_attempt = bytes_downloaded.load(std::sync::atomic::Ordering::Relaxed) - layer_bytes_before;
                    bytes_downloaded.fetch_sub(this_attempt, std::sync::atomic::Ordering::Relaxed);
                    mirror_attempts.push((mirror.host().to_string(), format_fetch_error(&e)));
                }
            }
        }

        if !layer_verified {
            failed_layers.push(FailedLayer {
                filename,
                expected_digest: layer.digest.clone(),
                mirror_attempts,
                bad_bytes: first_bad_bytes,
            });
        }
    }

    // 3. All verified
    if failed_layers.is_empty() {
        let expected_total: u64 = manifest.layers.iter().map(|l| l.size).sum();
        debug_assert_eq!(
            bytes_downloaded.load(std::sync::atomic::Ordering::Relaxed),
            expected_total,
            "bytes counted during download must equal the manifest's layer sizes"
        );

        // The files carry their own provenance, so the manifest isn't stored: the release is
        // verified by rebuilding it from the files, and every later reader does the same.
        let outcome = verify_release_dir(&temp_path, index);
        match outcome {
            crate::workspace::VerificationOutcome::VerifiedPublished { .. } => {}
            crate::workspace::VerificationOutcome::DifferentBytes { published_digest, reconstructed_digest, .. } => bail!(
                "✖ {} ({}) failed verification: its files rebuild a different manifest\n  {:<10}  {}\n  {:<10}  {}\n  Report it: https://github.com/olizilla/ods/issues",
                release.source.version,
                dataset.dataset_version(),
                "index",
                published_digest,
                "the files",
                reconstructed_digest
            ),
            crate::workspace::VerificationOutcome::Corrupted(block) => bail!("{}", block),
            other => bail!(
                "✖ {} ({}) failed verification: its files don't name the release the index does ({:?})",
                release.source.version,
                dataset.dataset_version(),
                other
            ),
        }
        // The readable view beside the files. `ods` never reads a value back from it.
        crate::datapackage::write_view(&temp_path)
            .context("writing datapackage.json into the release directory")?;

        fs::create_dir_all(workspace_root.join("releases"))?;
        if rel_dir.exists() {
            fs::remove_dir_all(&rel_dir)?;
        }
        fs::rename(&temp_path, &rel_dir)?;
        staging_guard.installed = true;

        // `--all` moves the pin once, after the whole batch, and lists only the bar row —
        // `dataset`, `verified` and `linked` don't change release to release.
        let pin_moved = if ctx.args.all {
            false
        } else {
            update_active_release_link_if_changed(workspace_root, &release.source.version)?
        };

        let mut unique_hosts: Vec<String> = Vec::new();
        for h in &served_from {
            if !unique_hosts.contains(h) {
                unique_hosts.push(h.clone());
            }
        }
        let from_label = match unique_hosts.len() {
            1 => unique_hosts[0].clone(),
            n => format!("{} mirrors", n),
        };
        let hash_opt = if ctx.args.verbose { Some(dataset.manifest_digest.as_str()) } else { None };
        let lines = render_release_block(&ReleaseBlockParams {
            date: &release.source.version,
            archive_size: expected_total,
            file_count: manifest.layers.len(),
            state: &ReleaseBlockState::Done { elapsed: download_start.elapsed() },
            dataset: Some(&dataset_coord),
            verified: &ctx.origin.verified_row(),
            linked: Some(ReleaseBlockLink { target: &link_target, unchanged: !pin_moved }),
            from: Some(&from_label),
            hash: hash_opt,
            hash_label: "manifest",
            color,
        });
        let out_lines: &[String] = if ctx.args.all { &lines[..1] } else { &lines };
        write_block(ctx.writer, progress, out_lines)?;
        write_source_issues(ctx.writer, release)?;

        // `--verbose` names the host that served each layer once more than one mirror was
        // needed — the tail's `from 2 mirrors` says that happened, but not which layer went
        // where.
        if ctx.args.verbose && !ctx.args.all && unique_hosts.len() > 1 {
            let filenames: Vec<&str> = manifest
                .layers
                .iter()
                .filter_map(|l| l.annotations.as_ref().and_then(|a| a.get(ANNOTATION_TITLE)))
                .map(|s| s.as_str())
                .collect();
            let width = filenames.iter().map(|f| f.len()).max().unwrap_or(0);
            for (filename, host) in filenames.iter().zip(served_from.iter()) {
                writeln!(ctx.writer, "  {:<width$}  {}", filename, host, width = width)?;
            }
        }

        if let Some(ref reason) = dataset.withdrawn {
            writeln!(
                ctx.writer,
                "✖ {} ({}) was withdrawn: {}\n  Pull a valid release: ods pull",
                release.source.version,
                dataset.dataset_version(),
                reason
            )?;
            return Err(AlreadyReported.into());
        }

        return Ok(());
    }

    // 4. One or more layers failed verification on every mirror
    for failed in &failed_layers {
        if let Some(ref bad_bytes) = failed.bad_bytes {
            let bad_path = temp_path.join(format!("{}.bad-sha", failed.filename));
            fs::write(&bad_path, bad_bytes)?;
        }
    }

    let has_verified_existing =
        rel_dir.exists() && verify_release_dir(&rel_dir, index).is_verified();

    let mut max_width = 8;
    for failed in &failed_layers {
        for (host, _) in &failed.mirror_attempts {
            max_width = max_width.max(host.len());
        }
    }

    if has_verified_existing {
        if temp_path.exists() {
            let _ = fs::remove_dir_all(&temp_path);
        }
        staging_guard.installed = true;

        progress.clear_live();
        writeln!(
            ctx.writer,
            "✖ {} ({}) arrived incomplete: {} of {} files failed verification on every mirror",
            release.source.version,
            dataset.dataset_version(),
            failed_layers.len(),
            manifest.layers.len()
        )?;
        for failed in &failed_layers {
            writeln!(ctx.writer, "  {}", failed.filename)?;
            writeln!(
                ctx.writer,
                "    {:<width$}  {}",
                "expected",
                failed.expected_digest,
                width = max_width
            )?;
            for (host, result) in &failed.mirror_attempts {
                writeln!(
                    ctx.writer,
                    "    {:<width$}  {}",
                    host,
                    result,
                    width = max_width
                )?;
            }
        }
        writeln!(
            ctx.writer,
            "  Kept the verified copy already in releases/{}/",
            release.source.version
        )?;
        writeln!(
            ctx.writer,
            "  Report it: https://github.com/olizilla/ods/issues"
        )?;

        if let Some(ref reason) = dataset.withdrawn {
            writeln!(
                ctx.writer,
                "✖ {} ({}) was withdrawn: {}\n  Pull a valid release: ods pull",
                release.source.version,
                dataset.dataset_version(),
                reason
            )?;
        }

        return Err(AlreadyReported.into());
    }

    // Normal incomplete install
    fs::create_dir_all(workspace_root.join("releases"))?;
    if rel_dir.exists() {
        let _ = fs::remove_dir_all(&rel_dir);
    }
    fs::rename(&temp_path, &rel_dir)?;
    staging_guard.installed = true;

    progress.clear_live();
    writeln!(
        ctx.writer,
        "✖ {} ({}) arrived incomplete: {} of {} files failed verification on every mirror",
        release.source.version,
        dataset.dataset_version(),
        failed_layers.len(),
        manifest.layers.len()
    )?;
    for failed in &failed_layers {
        if failed.bad_bytes.is_some() {
            writeln!(
                ctx.writer,
                "  {} → releases/{}/{}.bad-sha",
                failed.filename, release.source.version, failed.filename
            )?;
        } else {
            writeln!(ctx.writer, "  {}  not downloaded", failed.filename)?;
        }
        writeln!(
            ctx.writer,
            "    {:<width$}  {}",
            "expected",
            failed.expected_digest,
            width = max_width
        )?;
        for (host, result) in &failed.mirror_attempts {
            writeln!(
                ctx.writer,
                "    {:<width$}  {}",
                host,
                result,
                width = max_width
            )?;
        }
    }

    let verified_count = manifest.layers.len() - failed_layers.len();
    if verified_count == 1 {
        writeln!(
            ctx.writer,
            "  The other 1 file is verified, in releases/{}/",
            release.source.version
        )?;
    } else if verified_count > 1 {
        writeln!(
            ctx.writer,
            "  The other {} files are verified, in releases/{}/",
            verified_count, release.source.version
        )?;
    }

    let current_line = if let Ok(ws) = Workspace::open(Some(workspace_root)) {
        if let Ok((active_date, _)) = ws.active_release() {
            format!("  current is still releases/{}", active_date)
        } else {
            "  current is not set".to_string()
        }
    } else {
        "  current is not set".to_string()
    };
    writeln!(ctx.writer, "{}", current_line)?;
    writeln!(ctx.writer, "  Retry: ods pull {}", release.source.version)?;
    writeln!(ctx.writer, "  Use it anyway: ods use {}", release.source.version)?;
    writeln!(ctx.writer, "  Report it: https://github.com/olizilla/ods/issues")?;

    if let Some(ref reason) = dataset.withdrawn {
        writeln!(
            ctx.writer,
            "✖ {} ({}) was withdrawn: {}\n  Pull a valid release: ods pull",
            release.source.version,
            dataset.dataset_version(),
            reason
        )?;
    }

    Err(AlreadyReported.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verified_row_names_every_index_origin() {
        assert_eq!(
            IndexOrigin::Fetched("https://ods.fyi/releases.json".to_string()).verified_row(),
            "sha256 from releases.json"
        );
        assert_eq!(
            IndexOrigin::Flag("rehearsal.json".to_string()).verified_row(),
            "sha256 from rehearsal.json"
        );
        assert_eq!(
            IndexOrigin::Fetched("https://example.org/releases.json".to_string()).verified_row(),
            "sha256 from https://example.org/releases.json"
        );
        assert_eq!(IndexOrigin::BuiltIn.verified_row(), "sha256 from the index built into ods");
    }
}

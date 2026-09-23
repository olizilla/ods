use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub const DEFAULT_WORKSPACE_DIR: &str = "ods_data";

#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    pub date: String,
    pub path: PathBuf,
    pub is_active: bool,
    pub has_parquet: bool,
}

/// Validates whether a directory contains a valid `_releases.json` file.
pub fn validate_releases_json(dir: &Path) -> bool {
    let path = dir.join(crate::index::RELEASES_JSON_FILENAME);
    if !path.is_file() {
        return false;
    }
    let Ok(bytes) = fs::read(&path) else {
        return false;
    };
    if let Ok(idx) = serde_json::from_slice::<crate::index::OdsReleaseIndex>(&bytes) {
        return idx.validate().is_ok();
    }
    false
}

/// Checks the `_releases.json` marker in `dir`:
/// - Returns Ok(false) if file does not exist.
/// - Returns Ok(true) if file exists and validates as OdsReleaseIndex.
/// - Returns Err with actionable diagnostic if file exists and fails validation.
pub fn check_releases_json(dir: &Path) -> Result<bool> {
    let path = dir.join(crate::index::RELEASES_JSON_FILENAME);
    if !path.is_file() {
        return Ok(false);
    }
    let bytes = fs::read(&path)
        .with_context(|| format!("reading {}", path.display()))?;
    match serde_json::from_slice::<crate::index::OdsReleaseIndex>(&bytes) {
        Ok(idx) => {
            if idx.validate().is_ok() {
                Ok(true)
            } else {
                anyhow::bail!(
                    "✖ {} isn't a release index this ods can read\n  Expected $schema https://ods.fyi/schema/releases.v1.json\n  Delete it and run `ods pull` to replace it.",
                    path.display()
                )
            }
        }
        Err(_) => {
            anyhow::bail!(
                "✖ {} isn't a release index this ods can read\n  Expected $schema https://ods.fyi/schema/releases.v1.json\n  Delete it and run `ods pull` to replace it.",
                path.display()
            )
        }
    }
}

#[derive(Debug)]
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    /// Find an existing workspace. Never creates. For read commands.
    pub fn open(explicit: Option<&Path>) -> Result<Workspace> {
        let pwd = std::env::current_dir()?;
        Self::open_from(&pwd, explicit)
    }

    /// Find an existing workspace starting from a specific directory. Never creates.
    pub fn open_from(start: &Path, explicit: Option<&Path>) -> Result<Workspace> {
        let root = if let Some(path) = explicit {
            if !path.exists() {
                anyhow::bail!(
                    "✖ no ods workspace found at '{}'\n  Pass -i <trud.zip> -o <dir>, or run `ods pull` or `ods trud pull` to create a workspace.",
                    path.display()
                );
            }
            if check_releases_json(path)? {
                path.to_path_buf()
            } else if let Some(found) = find_workspace_root_from(path, None)? {
                found
            } else {
                anyhow::bail!(
                    "✖ no ods workspace found at '{}'\n  Pass -i <trud.zip> -o <dir>, or run `ods pull` or `ods trud pull` to create a workspace.",
                    path.display()
                );
            }
        } else {
            find_workspace_root_from(start, None)?.ok_or_else(|| {
                anyhow!(
                    "✖ no ods workspace found here\n  Pass -i <trud.zip> -o <dir>, or run `ods pull` or `ods trud pull` to create a workspace."
                )
            })?
        };
        Ok(Workspace { root })
    }

    /// Find one, or establish it at `explicit` (else the default). For write commands.
    pub fn open_or_create(explicit: Option<&Path>) -> Result<Workspace> {
        let root = if let Some(path) = explicit {
            if check_releases_json(path)? {
                path.to_path_buf()
            } else if path.join("releases").is_dir() || path.file_name().is_some_and(|n| n == DEFAULT_WORKSPACE_DIR) {
                // An explicit workspace path missing the marker: that is the root, seed marker if absent
                path.to_path_buf()
            } else if !path.exists() {
                if path.file_name().is_some_and(|n| n == DEFAULT_WORKSPACE_DIR) {
                    path.to_path_buf()
                } else {
                    anyhow::bail!(
                        "✖ no ods workspace found at '{}'\n  Pass an existing workspace or empty directory to create one.",
                        path.display()
                    );
                }
            } else {
                let is_empty = fs::read_dir(path)
                    .map(|mut entries| entries.next().is_none())
                    .unwrap_or(false);
                if !is_empty {
                    anyhow::bail!(
                        "✖ directory '{}' is not an ods workspace and is not empty",
                        path.display()
                    );
                }
                path.to_path_buf()
            }
        } else {
            find_workspace_root(None)?.unwrap_or_else(|| PathBuf::from(DEFAULT_WORKSPACE_DIR))
        };

        fs::create_dir_all(&root)
            .with_context(|| format!("creating workspace at {}", root.display()))?;
        ensure_workspace_root(&root)?;

        Ok(Workspace { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn active_release(&self) -> Result<(String, PathBuf)> {
        get_active_release(&self.root)
    }

    pub fn set_active(&self, date: &str) -> Result<()> {
        set_active_release(&self.root, date)?;
        let _ = generate_workspace_readme(&self.root, date, None, None);
        ensure_workspace_root(&self.root)?;
        Ok(())
    }

    pub fn prepare_release(&self, date: &str) -> Result<PathBuf> {
        prepare_release_dir(&self.root, date)
    }

    pub fn releases(&self) -> Result<Vec<ReleaseInfo>> {
        list_releases(&self.root)
    }

    pub fn parquet_dir(&self) -> Result<PathBuf> {
        if self.root.join("orgs.parquet").exists() {
            return Ok(self.root.clone());
        }
        if self.root.join("current").join("orgs.parquet").exists() {
            return Ok(self.root.join("current"));
        }
        if self.root.join("parquet").join("orgs.parquet").exists() {
            return Ok(self.root.join("parquet"));
        }
        if self.root.join("current").join("parquet").join("orgs.parquet").exists() {
            return Ok(self.root.join("current").join("parquet"));
        }
        if let Ok((_date, active_dir)) = self.active_release() {
            if active_dir.join("orgs.parquet").exists() {
                return Ok(active_dir);
            }
            if active_dir.join("parquet").join("orgs.parquet").exists() {
                return Ok(active_dir.join("parquet"));
            }
            return Ok(active_dir);
        }

        let rels = self.releases().unwrap_or_default();
        if !rels.is_empty() {
            let n = rels.len();
            let count_str = if n == 1 {
                "1 release".to_string()
            } else {
                format!("{} releases", n)
            };
            let newest_date = &rels[0].date;
            let ws_name = self.root
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(DEFAULT_WORKSPACE_DIR);
            return Err(anyhow!(
                "✖ No active release pinned\n  {} in {}/releases/, none active.\n  Pin one:  ods use {}",
                count_str,
                ws_name,
                newest_date
            ));
        }

        Err(anyhow!(
            "✖ no ods workspace found here\n  Pass -i <trud.zip> -o <dir>, or run `ods pull` or `ods trud pull` to create a workspace."
        ))
    }
}

/// Resolves the root workspace directory from a specific starting path:
/// 1. Explicit wins. A path from --workspace / -i / -o is the root. No further checks.
/// 2. Walk up. For each ancestor a, starting at start and ascending:
///    a/_releases.json validates -> root = a.
///    a/<DEFAULT_WORKSPACE_DIR>/_releases.json validates -> root = a/<DEFAULT_WORKSPACE_DIR>.
///    Stop before ascending past: the first a that contains a .git entry, $HOME, or the filesystem root.
///
/// Returns None if no workspace root was found.
pub fn find_workspace_root_from(start: &Path, explicit: Option<&Path>) -> Result<Option<PathBuf>> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    find_workspace_root_from_with_home(start, explicit, home.as_deref())
}

/// Inner resolver function allowing custom home boundary for unit testing.
pub fn find_workspace_root_from_with_home(
    start: &Path,
    explicit: Option<&Path>,
    home: Option<&Path>,
) -> Result<Option<PathBuf>> {
    // 1. Explicit wins. No further checks.
    if let Some(path) = explicit {
        return Ok(Some(path.to_path_buf()));
    }

    // 2. Walk up. For each ancestor a, starting at start and ascending:
    // a/_releases.json validates -> root = a.
    // a/<DEFAULT_WORKSPACE_DIR>/_releases.json validates -> root = a/<DEFAULT_WORKSPACE_DIR>.
    // Stop before ascending past: the first a that contains a .git entry, $HOME, or the filesystem root.
    let mut current = Some(start);
    while let Some(a) = current {
        if check_releases_json(a)? {
            return Ok(Some(a.to_path_buf()));
        }
        let default_ws = a.join(DEFAULT_WORKSPACE_DIR);
        if check_releases_json(&default_ws)? {
            return Ok(Some(default_ws));
        }

        // Stop before ascending past: the first a that contains a .git entry, $HOME, or the filesystem root.
        if a.join(".git").exists() || home.is_some_and(|h| a == h) {
            break;
        }

        current = a.parent();
    }

    Ok(None)
}

/// Resolves the root workspace directory from current working directory or explicit option.
fn find_workspace_root(explicit: Option<&Path>) -> Result<Option<PathBuf>> {
    if let Some(path) = explicit {
        return Ok(Some(path.to_path_buf()));
    }
    let pwd = std::env::current_dir()?;
    find_workspace_root_from(&pwd, explicit)
}
/// Resolves the parquet directory for read commands (`find`, `cite`, `info`, `role`).
///
/// When an explicit `-i` path is given:
/// 1. If explicit/orgs.parquet exists, return explicit directly (loose parquet dir).
/// 2. If explicit/parquet/orgs.parquet exists, return explicit/parquet.
/// 3. If explicit itself is a release directory (contains `_provenance.json`):
///    - if explicit/parquet is a directory, return explicit/parquet.
///    - otherwise return explicit directly.
/// 4. If explicit does not exist, fail with diagnostic error.
/// 5. Otherwise fall through to `Workspace::open_from(start, Some(explicit))?.parquet_dir()`.
///
/// When no explicit path is given (None):
/// Falls back to `Workspace::open_from(start, None)?.parquet_dir()`.
pub fn resolve_parquet_input(explicit: Option<&Path>) -> Result<PathBuf> {
    let pwd = std::env::current_dir()?;
    resolve_parquet_input_from(&pwd, explicit)
}

/// Resolves the parquet directory starting from a specific directory.
pub fn resolve_parquet_input_from(start: &Path, explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(input) = explicit {
        if input.join("orgs.parquet").exists() {
            return Ok(input.to_path_buf());
        }
        if input.join("parquet").join("orgs.parquet").exists() {
            return Ok(input.join("parquet"));
        }
        if input.join(crate::provenance::PROVENANCE_FILENAME).exists() {
            if input.join("parquet").is_dir() {
                return Ok(input.join("parquet"));
            }
            return Ok(input.to_path_buf());
        }
        if !input.exists() {
            anyhow::bail!(
                "✖ no ods workspace found at '{}'\n  Pass -i <trud.zip> -o <dir>, or run `ods pull` or `ods trud pull` to create a workspace.",
                input.display()
            );
        }
        let ws = Workspace::open_from(start, Some(input))?;
        return ws.parquet_dir();
    }

    let ws = Workspace::open_from(start, None)?;
    ws.parquet_dir()
}

/// Ensures the directory structure for a specific release date inside workspace root:
/// `./<workspace>/releases/<date>/trud/`
fn prepare_release_dir(workspace_root: &Path, release_date: &str) -> Result<PathBuf> {
    let release_dir = workspace_root.join("releases").join(release_date);
    fs::create_dir_all(release_dir.join("trud"))
        .context("Failed to create release trud directory")?;
    ensure_workspace_root(workspace_root)?;
    Ok(release_dir)
}

/// Ensures all workspace root furniture exists (gitignore, readme, and _releases.json marker).
pub fn ensure_workspace_root(workspace_root: &Path) -> Result<()> {
    fs::create_dir_all(workspace_root)
        .with_context(|| format!("creating workspace root directory at {}", workspace_root.display()))?;
    ensure_workspace_gitignore(workspace_root)?;
    ensure_workspace_readme(workspace_root)?;
    ensure_workspace_marker(workspace_root)?;
    Ok(())
}

/// Seeds the workspace marker `_releases.json` from the baked index verbatim if absent.
pub fn ensure_workspace_marker(workspace_root: &Path) -> Result<()> {
    fs::create_dir_all(workspace_root)
        .with_context(|| format!("creating directory at {}", workspace_root.display()))?;
    let marker_path = workspace_root.join(crate::index::RELEASES_JSON_FILENAME);
    if !marker_path.exists() {
        fs::write(&marker_path, crate::index::BAKED_RELEASES_JSON_BYTES)
            .with_context(|| format!("writing workspace marker to {}", marker_path.display()))?;
    }
    Ok(())
}

/// Ensures `.gitignore` ignores heavy raw ZIP/XML files while preserving metadata/parquet artifacts.
fn ensure_workspace_gitignore(workspace_root: &Path) -> Result<()> {
    let gitignore_path = workspace_root.join(".gitignore");
    if !gitignore_path.exists() {
        let content = "# Ignore raw TRUD archive downloads and extracted XML files\nreleases/*/trud/\n*.zip\n*.xml\n";
        fs::write(&gitignore_path, content)
            .context("Failed to write workspace .gitignore file")?;
    }
    Ok(())
}

fn ensure_workspace_readme(workspace_root: &Path) -> Result<()> {
    let readme_path = workspace_root.join("README.md");
    if !readme_path.exists() {
        let active = get_active_release(workspace_root).ok().map(|(d, _)| d);
        let date_str = active.as_deref().unwrap_or("none");
        generate_workspace_readme(workspace_root, date_str, None, None)?;
    }
    Ok(())
}

static NUDGE_EMITTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub const STALENESS_THRESHOLD_DAYS: i64 = 45;

/// Decides whether a release is stale and returns the notice line.
/// Pure function: takes dates, reads no files and no clock.
pub fn staleness_notice(
    release_date: chrono::NaiveDate,
    newest_local_date: chrono::NaiveDate,
    today: chrono::NaiveDate,
) -> Option<String> {
    if release_date < newest_local_date {
        return None;
    }
    let days = (today - release_date).num_days();
    if days > STALENESS_THRESHOLD_DAYS {
        Some(format!(
            "* {} release is {} days old. Run `ods pull` to check for a newer one.",
            release_date, days
        ))
    } else {
        None
    }
}

/// Emits a one-line staleness notice to stderr if the release being read is the newest local release
/// and older than 45 days.
/// At most once per process invocation.
/// Strictly suppressed on machine-readable formats.
pub fn check_and_emit_staleness_nudge(release_dir: &Path, is_human_format: bool) {
    if !is_human_format {
        return;
    }
    if NUDGE_EMITTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let prov_file = release_dir.join(crate::provenance::PROVENANCE_FILENAME);
    let Ok(content) = fs::read_to_string(&prov_file) else {
        return;
    };
    let Ok(prov) = serde_json::from_str::<crate::provenance::OdsProvenance>(&content) else {
        return;
    };
    let Some(ref date_str) = prov.trud_release_date else {
        return;
    };
    let Ok(rel_date) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") else {
        return;
    };

    let Some(ws_root) = find_workspace_root_from(release_dir, None).ok().flatten() else {
        return;
    };
    let Ok(releases) = list_releases(&ws_root) else {
        return;
    };
    let Some(newest_local_date) = releases.iter().filter_map(|r| {
        chrono::NaiveDate::parse_from_str(&r.date, "%Y-%m-%d").ok()
    }).max() else {
        return;
    };

    let today = chrono::Utc::now().date_naive();
    if let Some(notice) = staleness_notice(rel_date, newest_local_date, today) {
        eprintln!("{}", notice);
    }
}

/// Detects if a directory is a release directory.
/// Returns Some("YYYY-MM-DD") if dir is a release directory, or None otherwise.
pub fn detect_release_from_dir(dir: &Path) -> Option<String> {
    // 1. If dir has _provenance.json with trud_release_date:
    if let Some(prov) = crate::provenance::OdsProvenance::load_from_dir(dir).ok() {
        if let Some(d) = prov.trud_release_date {
            return Some(d);
        }
    }
    // 2. If parent directory is named "releases" and folder name matches YYYY-MM-DD:
    // Structural inspection of dir is strictly for the disagreement UX nudge; it never discovers workspaces or selects data.
    if let Some(parent) = dir.parent() {
        if parent.file_name().and_then(|n| n.to_str()) == Some("releases") {
            if let Some(name) = dir.file_name().and_then(|n| n.to_str()) {
                if chrono::NaiveDate::parse_from_str(name, "%Y-%m-%d").is_ok() {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

/// Detects if the current working directory is a release directory.
/// Returns Some("YYYY-MM-DD") if cwd is a release directory, or None otherwise.
pub fn detect_cwd_release() -> Option<String> {
    std::env::current_dir().ok().and_then(|cwd| detect_release_from_dir(&cwd))
}

/// Resolves the active release date of the enclosing workspace (if any).
pub fn workspace_active_release(dir: Option<&Path>) -> Option<String> {
    if let Some(d) = dir {
        if let Some(root) = find_workspace_root_from(d, None).ok().flatten() {
            if let Ok((active, _)) = get_active_release(&root) {
                return Some(active);
            }
        }
    }
    if let Ok(ws) = Workspace::open(None) {
        if let Ok((active, _)) = ws.active_release() {
            return Some(active);
        }
    }
    None
}

/// Resolution state of a command's release against workspace active release and cwd.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleaseResolution {
    /// Resolved release equals workspace's active release, and cwd is not a different release directory.
    Current,
    /// Resolved release equals workspace's active release, but cwd is a different release directory.
    Disagreement { cwd_date: String },
    /// Resolved release does not equal workspace's active release (e.g. explicit -i to an archived release).
    ExplicitNonCurrent,
}

/// Checks the resolution state of `release_date` relative to the workspace's active release and cwd.
pub fn check_release_resolution_with_cwd(
    release_date: &str,
    release_dir: Option<&Path>,
    cwd: Option<&Path>,
) -> ReleaseResolution {
    let is_current = workspace_active_release(release_dir)
        .map(|act| act == release_date)
        .unwrap_or(false);

    if is_current {
        let detected = match cwd {
            Some(dir) => detect_release_from_dir(dir),
            None => detect_cwd_release(),
        };
        if let Some(cwd_date) = detected {
            if cwd_date != release_date {
                return ReleaseResolution::Disagreement { cwd_date };
            }
        }
        ReleaseResolution::Current
    } else {
        ReleaseResolution::ExplicitNonCurrent
    }
}

pub fn check_release_resolution(release_date: &str, release_dir: Option<&Path>) -> ReleaseResolution {
    check_release_resolution_with_cwd(release_date, release_dir, None)
}

/// Formats a single `* Source: {path}` line, optionally muted with ANSI_MUTED.
///
/// This is the canonical definition of the source line format across ods commands.
pub fn format_source_line(path: &str, color: bool) -> String {
    let line = format!("* Source: {}", path);
    if color {
        format!("{}{}{}", crate::ansi::ANSI_MUTED, line, crate::ansi::ANSI_RESET)
    } else {
        line
    }
}

/// Builds the source header lines for a resolved release directory and file name.
///
/// Returns:
/// 1. `* Source: releases/{date}/{file_name}` (or relative workspace path), muted if `color` is true.
/// 2. If running from a release directory different from the active workspace release,
///    an additional disagreement line:
///    `! Run from releases/{cwd_date}. Change source with: ods use {cwd_date}`
pub fn format_source_header(release_dir: &Path, file_name: &str, color: bool) -> Vec<String> {
    let release_date = crate::provenance::OdsProvenance::load_from_dir(release_dir)
        .ok()
        .and_then(|p| p.trud_release_date)
        .or_else(|| {
            find_workspace_root_from(release_dir, None)
                .ok()
                .flatten()
                .and_then(|r| Workspace::open(Some(&r)).ok())
                .and_then(|ws| ws.active_release().ok().map(|(d, _)| d))
        });

    let source_rel = if let Some(ref d) = release_date {
        format!("releases/{}/{}", d, file_name)
    } else {
        let ws_root = find_workspace_root_from(release_dir, None).ok().flatten();
        if let Some(ref root) = ws_root {
            release_dir
                .strip_prefix(root)
                .unwrap_or(release_dir)
                .join(file_name)
                .display()
                .to_string()
        } else {
            release_dir.join(file_name).display().to_string()
        }
    };

    let mut lines = vec![format_source_line(&source_rel, color)];

    if let Some(ref d) = release_date {
        if let ReleaseResolution::Disagreement { ref cwd_date } =
            check_release_resolution(d, Some(release_dir))
        {
            lines.push(format!(
                "! Run from releases/{}. Change source with: ods use {}",
                cwd_date, cwd_date
            ));
        }
    }

    lines
}

/// Builds the source header lines for `ods cite`.
///
/// Returns:
/// 1. `* Source: releases/{date} ({version})`
/// 2. If running from a release directory different from the active workspace release,
///    an additional disagreement line:
///    `! Run from releases/{cwd_date}. Change source with: ods use {cwd_date}`
pub fn format_cite_source_header(
    release_dir: &Path,
    release_date: &str,
    dataset_version: &str,
    cwd: Option<&Path>,
) -> Vec<String> {
    let path_display = format!("releases/{} ({})", release_date, dataset_version);
    let mut lines = vec![format_source_line(&path_display, false)];

    if let ReleaseResolution::Disagreement { cwd_date } =
        check_release_resolution_with_cwd(release_date, Some(release_dir), cwd)
    {
        lines.push(format!(
            "! Run from releases/{}. Change source with: ods use {}",
            cwd_date, cwd_date
        ));
    }

    lines
}

/// Reports an inferred release input for write commands (make, make oci) and names the directory written.
pub fn report_inferred_release_write(release_date: &str, written_dir: &Path) {
    let is_current = workspace_active_release(Some(written_dir))
        .map(|act| act == release_date)
        .unwrap_or(false);
    let current_tag = if is_current { " (current)" } else { "" };
    if let Some(cwd_date) = detect_cwd_release() {
        if cwd_date != release_date {
            eprintln!(
                "  {}{current_tag} → {}, not the {} you're in",
                release_date,
                written_dir.display(),
                cwd_date
            );
            eprintln!("  Switch with: ods use {}", cwd_date);
            return;
        }
    }
    eprintln!("  {}{current_tag} → {}", release_date, written_dir.display());
}

/// Sets or updates the `current` symlink/pointer in the workspace root to target `releases/<release_date>`.
fn set_active_release(workspace_root: &Path, release_date: &str) -> Result<()> {
    let target = Path::new("releases").join(release_date);
    let current_link = workspace_root.join("current");

    if current_link.exists() || fs::symlink_metadata(&current_link).is_ok() {
        let _ = fs::remove_file(&current_link);
        let _ = fs::remove_dir_all(&current_link);
    }

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&target, &current_link)
            .context("Failed to create 'current' symlink")?;
    }

    #[cfg(windows)]
    {
        if std::os::windows::fs::symlink_dir(&target, &current_link).is_err() {
            // Fallback to text file pointer on Windows if symlinks are restricted
            fs::write(&current_link, target.to_string_lossy().as_bytes())
                .context("Failed to create 'current' pointer file")?;
        }
    }

    Ok(())
}

/// Resolves the active release date and path from the workspace.
fn get_active_release(workspace_root: &Path) -> Result<(String, PathBuf)> {
    let current_path = workspace_root.join("current");
    if !current_path.exists() && fs::symlink_metadata(&current_path).is_err() {
        return Err(anyhow!("No active release found in workspace"));
    }

    let real_path = fs::canonicalize(&current_path)
        .or_else(|_| {
            if current_path.is_file() {
                let content = fs::read_to_string(&current_path)?;
                Ok(workspace_root.join(content.trim()))
            } else {
                Err(anyhow!("Could not resolve 'current' release path"))
            }
        })?;

    let release_date = real_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow!("Invalid release path name"))?
        .to_string();

    Ok((release_date, real_path))
}

/// Lists all local releases in `./<workspace>/releases/`.
fn list_releases(workspace_root: &Path) -> Result<Vec<ReleaseInfo>> {
    let releases_dir = workspace_root.join("releases");
    if !releases_dir.exists() {
        return Ok(Vec::new());
    }

    let active_date = get_active_release(workspace_root).map(|(date, _)| date).ok();

    let mut releases = Vec::new();
    for entry in fs::read_dir(releases_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if let Some(date) = path.file_name().and_then(|n| n.to_str()) {
                let is_active = active_date.as_deref() == Some(date);
                let has_parquet = path.join("orgs.parquet").exists() || path.join("parquet").join("orgs.parquet").exists();
                releases.push(ReleaseInfo {
                    date: date.to_string(),
                    path,
                    is_active,
                    has_parquet,
                });
            }
        }
    }

    releases.sort_by(|a, b| b.date.cmp(&a.date));
    Ok(releases)
}

/// Generates the self-documenting `README.md` file in the workspace root.
fn generate_workspace_readme(
    workspace_root: &Path,
    release_date: &str,
    seq_num: Option<&str>,
    entity_count: Option<usize>,
) -> Result<()> {
    let readme_path = workspace_root.join("README.md");
    let seq_info = seq_num.map(|s| format!(" (TRUD Sequence #{s})")).unwrap_or_default();
    let count_info = entity_count.map(|c| format!("{c} entities")).unwrap_or_else(|| "Active dataset".to_string());
    let ws_name = workspace_root.file_name().and_then(|n| n.to_str()).unwrap_or(DEFAULT_WORKSPACE_DIR);

    let content = format!(
        "# `ods` workspace\n\n\
         This directory holds verified releases of NHS Organisation Data as Parquet files, pulled by the `ods` CLI.\n\
         The data is NHS England's Organisation Data Service, under the Open Government Licence.\n\n\
         - **Active Release**: {release_date}{seq_info}\n\
         - **Status**: {count_info} indexed in `current/`\n\n\
         ## Directory Structure\n\n\
         - `_releases.json`: Cached release index\n\
         - `current`: Symlink pointing to active release\n\
         - `releases/`: Dated release directories containing Parquet tables, Frictionless datapackage, and metadata\n\n\
         ## Quick Start: Querying with DuckDB\n\n\
         ```sql\n\
         -- Query active GP practices in Sedbergh\n\
         SELECT name, ods_code, postcode, telephone\n\
         FROM '{ws_name}/current/orgs.parquet'\n\
         WHERE status = 'active' AND town = 'SEDBERGH';\n\
         ```\n\n\
         ## Python Integration\n\n\
         ```python\n\
         import duckdb\n\
         con = duckdb.connect()\n\
         df = con.execute(\"SELECT * FROM '{ws_name}/current/orgs.parquet'\").df()\n\
         print(df.head())\n\
         ```\n\n\
         ## CLI Commands\n\n\
         - `ods find <query>`: Fast terminal lookup\n\
         - `ods cite`: Output APA and BibTeX academic citations\n\
         - `ods diff`: Diff active release against previous release\n"
    );

    fs::write(readme_path, content).context("Failed to write workspace README.md")?;
    Ok(())
}

/// Helper function to count total rows in a Parquet file.
pub fn count_records_in_parquet(path: &Path) -> Result<usize> {
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    let file = fs::File::open(path).with_context(|| format!("Opening parquet file {}", path.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let reader = builder.build()?;
    let mut total = 0;
    for batch in reader {
        total += batch?.num_rows();
    }
    Ok(total)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationOutcome {
    VerifiedPublished {
        date: String,
        version: String,
        digest: String,
    },
    VerifiedUnpublished {
        date: String,
        version: String,
        digest: String,
    },
    Mismatch {
        date: String,
        version: String,
        expected_digest: String,
        reconstructed_digest: String,
    },
    Corrupted(String),
}

impl VerificationOutcome {
    pub fn is_verified(&self) -> bool {
        matches!(
            self,
            VerificationOutcome::VerifiedPublished { .. }
                | VerificationOutcome::VerifiedUnpublished { .. }
        )
    }

    pub fn is_verified_published(&self) -> bool {
        matches!(self, VerificationOutcome::VerifiedPublished { .. })
    }
}

pub fn verify_release_dir(
    release_dir: &Path,
    index: &crate::index::OdsReleaseIndex,
) -> VerificationOutcome {
    let prov = match crate::provenance::OdsProvenance::load_from_dir(release_dir).ok() {
        Some(p) => p,
        None => return VerificationOutcome::Corrupted("Missing or unreadable _provenance.json".to_string()),
    };

    let date = match prov.trud_release_date.as_deref() {
        Some(d) => d.to_string(),
        None => return VerificationOutcome::Corrupted("Provenance missing trud_release_date".to_string()),
    };

    let version = match crate::datapackage::read_dataset_version_from_dir(release_dir) {
        Some(v) => v,
        None => return VerificationOutcome::Corrupted("datapackage.json missing version".to_string()),
    };

    let (manifest, _) = match crate::commands::make_oci::build_manifest_from_dir(release_dir, &prov, &version) {
        Ok(m) => m,
        Err(e) => return VerificationOutcome::Corrupted(format!("Failed to reconstruct manifest: {}", e)),
    };

    let reconstructed_digest = match manifest.digest() {
        Ok(d) => d,
        Err(e) => return VerificationOutcome::Corrupted(format!("Failed to compute manifest digest: {}", e)),
    };

    // Cross-check datapackage.json resources if present
    let dp_path = release_dir.join("datapackage.json");
    if dp_path.exists() {
        if let Ok(dp_bytes) = fs::read(&dp_path) {
            if let Ok(dp) = serde_json::from_slice::<serde_json::Value>(&dp_bytes) {
                if let Some(resources) = dp.get("resources").and_then(|r| r.as_array()) {
                    for res in resources {
                        if let Some(res_hash) = res.get("hash").and_then(|h| h.as_str()) {
                            let clean_hash = res_hash.trim_start_matches("sha256:").to_lowercase();
                            let res_name = res.get("name").and_then(|n| n.as_str()).unwrap_or("");
                            let res_path = res.get("path").and_then(|p| p.as_str()).unwrap_or("");
                            let found_layer = manifest.layers.iter().find(|l| {
                                l.annotations
                                    .as_ref()
                                    .and_then(|a| a.get(crate::oci::ANNOTATION_TITLE))
                                    .map(|t| t == res_name || t == res_path)
                                    .unwrap_or(false)
                            });
                            match found_layer {
                                Some(l) => {
                                    let l_hash = l.digest.trim_start_matches("sha256:").to_lowercase();
                                    if clean_hash != l_hash {
                                        return VerificationOutcome::Corrupted(format!(
                                            "datapackage resource {} hash mismatch",
                                            res_name
                                        ));
                                    }
                                }
                                None => {
                                    return VerificationOutcome::Corrupted(format!(
                                        "datapackage resource {} not found in manifest layers",
                                        res_name
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let dataset = index
        .releases
        .iter()
        .find(|r| r.trud_release_date == date)
        .and_then(|r| r.datasets.iter().find(|d| d.dataset_version == version));

    if let Some(entry) = dataset {
        if entry.manifest_digest == reconstructed_digest {
            return VerificationOutcome::VerifiedPublished {
                date,
                version,
                digest: reconstructed_digest,
            };
        } else {
            return VerificationOutcome::Mismatch {
                date,
                version,
                expected_digest: entry.manifest_digest.clone(),
                reconstructed_digest,
            };
        }
    }

    VerificationOutcome::VerifiedUnpublished {
        date,
        version,
        digest: reconstructed_digest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn test_staleness_notice() {
        struct TestCase {
            name: &'static str,
            release_date: NaiveDate,
            newest_local_date: NaiveDate,
            today: NaiveDate,
            expected: Option<&'static str>,
        }

        let cases = [
            TestCase {
                name: "newest release at 46 days old emits notice",
                release_date: NaiveDate::from_ymd_opt(2026, 8, 28).unwrap(),
                newest_local_date: NaiveDate::from_ymd_opt(2026, 8, 28).unwrap(),
                today: NaiveDate::from_ymd_opt(2026, 10, 13).unwrap(),
                expected: Some("* 2026-08-28 release is 46 days old. Run `ods pull` to check for a newer one."),
            },
            TestCase {
                name: "newest release at 45 days old is quiet",
                release_date: NaiveDate::from_ymd_opt(2026, 8, 28).unwrap(),
                newest_local_date: NaiveDate::from_ymd_opt(2026, 8, 28).unwrap(),
                today: NaiveDate::from_ymd_opt(2026, 10, 12).unwrap(),
                expected: None,
            },
            TestCase {
                name: "older release at 400 days old with newer local release is quiet",
                release_date: NaiveDate::from_ymd_opt(2025, 9, 8).unwrap(),
                newest_local_date: NaiveDate::from_ymd_opt(2026, 8, 28).unwrap(),
                today: NaiveDate::from_ymd_opt(2026, 10, 13).unwrap(),
                expected: None,
            },
        ];

        for case in cases {
            let actual = staleness_notice(case.release_date, case.newest_local_date, case.today);
            assert_eq!(
                actual.as_deref(),
                case.expected,
                "case '{}' failed",
                case.name
            );
        }
    }
}




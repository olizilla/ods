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

static WARNED_WORKSPACES: std::sync::Mutex<Option<std::collections::HashSet<PathBuf>>> =
    std::sync::Mutex::new(None);

pub fn emit_cache_notice_if_needed(workspace_root: &Path) {
    let canonical = std::fs::canonicalize(workspace_root).unwrap_or_else(|_| workspace_root.to_path_buf());
    let mut guard = WARNED_WORKSPACES.lock().unwrap();
    let set = guard.get_or_insert_with(std::collections::HashSet::new);
    if !set.insert(canonical) {
        return;
    }
    let marker_path = workspace_root.join(crate::index::RELEASES_JSON_FILENAME);
    if marker_path.exists() {
        eprintln!(
            "! Ignoring {}: it isn't a release index this ods can read\n  Using the index built into ods. The next ods pull will replace it.",
            marker_path.display()
        );
    } else {
        eprintln!(
            "! Missing {}: using the index built into ods",
            marker_path.display()
        );
    }
}

#[cfg(test)]
pub fn reset_cache_notices() {
    let mut guard = WARNED_WORKSPACES.lock().unwrap();
    if let Some(set) = guard.as_mut() {
        set.clear();
    }
}

/// Returns `path` relativized against the current working directory, canonicalizing
/// both so differences like macOS's `/var/folders` vs `/private/var/folders` resolve cleanly.
pub fn relative_to_cwd(path: &Path) -> PathBuf {
    if let Ok(cwd) = std::env::current_dir() {
        let canon_cwd = cwd.canonicalize().unwrap_or(cwd);
        let canon_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        canon_path
            .strip_prefix(&canon_cwd)
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|_| path.to_path_buf())
    } else {
        path.to_path_buf()
    }
}

/// Checks whether `dir/releases/` contains at least one date-shaped directory
/// (`\d{4}-\d{2}-\d{2}`) holding a readable `_provenance.json`.
///
/// Bounded traversal: `releases/` is only opened if it exists.
/// Stops at the first valid release found without enumerating the rest.
pub fn has_readable_release(dir: &Path) -> bool {
    let releases_dir = dir.join("releases");
    if !releases_dir.is_dir() {
        return false;
    }
    let Ok(entries) = fs::read_dir(&releases_dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if chrono::NaiveDate::parse_from_str(name, "%Y-%m-%d").is_ok()
            && crate::provenance::OdsProvenance::load_from_dir(&path).ok().is_some()
        {
            return true;
        }
    }
    false
}

/// Checks the `_releases.json` marker in `dir`:
/// - Returns Ok(false) if file does not exist or fails validation (allowing candidate to qualify on releases).
/// - Returns Ok(true) if file exists, validates structurally, and does not contradict baked index.
/// - Returns Err with SecurityError if file exists and contradicts baked index.
pub fn check_releases_json(dir: &Path) -> Result<bool> {
    if let Ok(baked) = crate::index::OdsReleaseIndex::baked() {
        check_releases_json_with_baked(dir, &baked)
    } else {
        check_releases_json_with_baked(dir, &crate::index::OdsReleaseIndex::default())
    }
}

pub fn check_releases_json_with_baked(dir: &Path, baked: &crate::index::OdsReleaseIndex) -> Result<bool> {
    let path = dir.join(crate::index::RELEASES_JSON_FILENAME);
    if !path.is_file() {
        return Ok(false);
    }
    let Ok(bytes) = fs::read(&path) else {
        return Ok(false);
    };
    let idx: crate::index::OdsReleaseIndex = match serde_json::from_slice(&bytes) {
        Ok(i) => i,
        Err(_) => return Ok(false),
    };
    if idx.validate().is_err() {
        return Ok(false);
    }

    // A cache that parses but contradicts a baked fact is a security error from merge
    baked.merge(&idx)?;

    Ok(true)
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
            } else if has_readable_release(path) {
                emit_cache_notice_if_needed(path);
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
        ensure_workspace_gitignore(&self.root)?;
        ensure_workspace_readme(&self.root)?;
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
        if has_readable_release(a) {
            emit_cache_notice_if_needed(a);
            return Ok(Some(a.to_path_buf()));
        }

        let default_ws = a.join(DEFAULT_WORKSPACE_DIR);
        if check_releases_json(&default_ws)? {
            return Ok(Some(default_ws));
        }
        if has_readable_release(&default_ws) {
            emit_cache_notice_if_needed(&default_ws);
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

/// Seeds the workspace cache `_releases.json` from the baked index verbatim if absent or unreadable.
pub fn ensure_workspace_marker(workspace_root: &Path) -> Result<()> {
    fs::create_dir_all(workspace_root)
        .with_context(|| format!("creating directory at {}", workspace_root.display()))?;
    let marker_path = workspace_root.join(crate::index::RELEASES_JSON_FILENAME);
    if !marker_path.exists() || !validate_releases_json(workspace_root) {
        fs::write(&marker_path, crate::index::BAKED_RELEASES_JSON_BYTES)
            .with_context(|| format!("writing workspace index cache to {}", marker_path.display()))?;
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
    DateUnknown {
        date: String,
        version: String,
        digest: String,
    },
    DifferentArchive {
        date: String,
        version: String,
        digest: String,
        this_archive_sha256: String,
        published_archive_sha256: String,
        published_digest: Option<String>,
    },
    VersionUnpublished {
        date: String,
        version: String,
        digest: String,
        published_versions: Vec<String>,
    },
    DifferentBytes {
        date: String,
        version: String,
        published_digest: String,
        reconstructed_digest: String,
    },
    ChangedSinceBuilt {
        file: String,
    },
    NoProvenance,
    Corrupted(String),
}

impl VerificationOutcome {
    pub fn is_verified(&self) -> bool {
        match self {
            VerificationOutcome::VerifiedPublished { .. }
            | VerificationOutcome::DateUnknown { .. }
            | VerificationOutcome::VersionUnpublished { .. } => true,
            VerificationOutcome::DifferentArchive { published_digest, .. } => {
                published_digest.is_none()
            }
            VerificationOutcome::DifferentBytes { .. }
            | VerificationOutcome::ChangedSinceBuilt { .. }
            | VerificationOutcome::NoProvenance
            | VerificationOutcome::Corrupted(_) => false,
        }
    }

    pub fn is_verified_published(&self) -> bool {
        matches!(self, VerificationOutcome::VerifiedPublished { .. })
    }
}

pub fn verify_release_dir(
    release_dir: &Path,
    index: &crate::index::OdsReleaseIndex,
) -> VerificationOutcome {
    let prov_file = release_dir.join(crate::provenance::PROVENANCE_FILENAME);
    if !prov_file.exists() {
        return VerificationOutcome::NoProvenance;
    }

    let prov = match crate::provenance::OdsProvenance::load_from_file(&prov_file) {
        crate::provenance::ProvenanceLoad::Read(p, _) => *p,
        crate::provenance::ProvenanceLoad::Absent => {
            return VerificationOutcome::NoProvenance;
        }
        crate::provenance::ProvenanceLoad::Unreadable { path, date } => {
            return VerificationOutcome::Corrupted(
                crate::provenance::format_unreadable_provenance_error(&path, &date),
            );
        }
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
                            let file_id = if !res_name.is_empty() { res_name } else { res_path };
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
                                        return VerificationOutcome::ChangedSinceBuilt {
                                            file: file_id.to_string(),
                                        };
                                    }
                                }
                                None => {
                                    return VerificationOutcome::Corrupted(format!(
                                        "datapackage resource {} not found in manifest layers",
                                        file_id
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let release_entry = index.releases.iter().find(|r| r.trud_release_date == date);
    let release_row = match release_entry {
        Some(r) => r,
        None => {
            return VerificationOutcome::DateUnknown {
                date,
                version,
                digest: reconstructed_digest,
            };
        }
    };

    if let Some(ref this_archive_sha256) = prov.trud_release_sha256 {
        if !this_archive_sha256.is_empty()
            && !this_archive_sha256.eq_ignore_ascii_case(&release_row.trud_release_sha256)
        {
            let published_digest = release_row
                .datasets
                .iter()
                .find(|d| d.dataset_version == version)
                .map(|d| d.manifest_digest.clone());

            return VerificationOutcome::DifferentArchive {
                date,
                version,
                digest: reconstructed_digest,
                this_archive_sha256: this_archive_sha256.clone(),
                published_archive_sha256: release_row.trud_release_sha256.clone(),
                published_digest,
            };
        }
    }

    let dataset_entry = release_row.datasets.iter().find(|d| d.dataset_version == version);
    match dataset_entry {
        Some(entry) => {
            if entry.manifest_digest == reconstructed_digest {
                VerificationOutcome::VerifiedPublished {
                    date,
                    version,
                    digest: reconstructed_digest,
                }
            } else {
                VerificationOutcome::DifferentBytes {
                    date,
                    version,
                    published_digest: entry.manifest_digest.clone(),
                    reconstructed_digest,
                }
            }
        }
        None => {
            let published_versions = release_row.datasets.iter().map(|d| d.dataset_version.clone()).collect();
            VerificationOutcome::VersionUnpublished {
                date,
                version,
                digest: reconstructed_digest,
                published_versions,
            }
        }
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

    #[test]
    fn test_verify_release_dir_cases() {
        let base_tmp = tempfile::tempdir().unwrap();

        let make_prov_json = |date: &str, sha: &str| -> String {
            serde_json::to_string_pretty(&crate::provenance::OdsProvenance::from_trud_statement(
                date, sha, 38064419,
            ))
            .unwrap()
        };

        let setup_fixture_dir = |dir: &Path, prov_content: Option<&str>, dp_version: Option<&str>, corrupt_resource: bool, extra_file: bool| {
            fs::create_dir_all(dir).unwrap();
            fs::write(dir.join("orgs.parquet"), b"dummy orgs content").unwrap();
            if extra_file {
                fs::write(dir.join("roles.parquet"), b"dummy roles content").unwrap();
            }

            let orgs_hash = crate::provenance::compute_file_sha256(&dir.join("orgs.parquet")).unwrap();

            if let Some(prov_str) = prov_content {
                fs::write(dir.join(crate::provenance::PROVENANCE_FILENAME), prov_str).unwrap();
            }

            if let Some(version) = dp_version {
                let res_hash = if corrupt_resource {
                    "0000000000000000000000000000000000000000000000000000000000000000".to_string()
                } else {
                    orgs_hash.to_lowercase()
                };
                let mut resources = vec![
                    serde_json::json!({
                        "name": "orgs.parquet",
                        "path": "orgs.parquet",
                        "hash": format!("sha256:{}", res_hash)
                    })
                ];
                if extra_file {
                    let roles_hash = crate::provenance::compute_file_sha256(&dir.join("roles.parquet")).unwrap();
                    resources.push(serde_json::json!({
                        "name": "roles.parquet",
                        "path": "roles.parquet",
                        "hash": format!("sha256:{}", roles_hash.to_lowercase())
                    }));
                }
                let dp = serde_json::json!({
                    "name": "ods",
                    "version": version,
                    "resources": resources
                });
                fs::write(dir.join("datapackage.json"), serde_json::to_vec(&dp).unwrap()).unwrap();
            }
        };

        // Reference published directory to get published manifest digest
        let ref_dir = base_tmp.path().join("ref_published");
        let published_prov = make_prov_json(
            "2026-08-28",
            "ABDD194B1569D5FF3CDD81D618847F05642BD43C5B15D6CD43D8289B7466D801",
        );
        setup_fixture_dir(&ref_dir, Some(&published_prov), Some("0.1.0"), false, false);
        let ref_prov: crate::provenance::OdsProvenance = serde_json::from_str(&published_prov).unwrap();
        let (manifest, _) = crate::commands::make_oci::build_manifest_from_dir(&ref_dir, &ref_prov, "0.1.0").unwrap();
        let published_digest = manifest.digest().unwrap();

        let index = crate::index::OdsReleaseIndex {
            schema: crate::index::RELEASES_SCHEMA_V1_URL.to_string(),
            trud_signing_key_fingerprints: vec!["71ED5964BAE53E83556320A42BE59DADEE84BEB0".to_string()],
            mirrors: vec![],
            releases: vec![
                crate::index::Release {
                    trud_release_date: "2026-08-28".to_string(),
                    trud_release_sha256: "ABDD194B1569D5FF3CDD81D618847F05642BD43C5B15D6CD43D8289B7466D801".to_string(),
                    trud_release_filesize_bytes: 38064419,
                    datasets: vec![
                        crate::index::Dataset {
                            dataset_version: "0.1.0".to_string(),
                            manifest_digest: published_digest.clone(),
                            dataset_filesize_bytes: 1000,
                            tool_version: "0.2.0".to_string(),
                            tool_git_sha: "0123456789abcdef0123456789abcdef01234567".to_string(),
                            dataset_doi: None,
                            withdrawn: None,
                        }
                    ],
                }
            ],
        };

        struct TableCase {
            #[allow(dead_code)]
            name: &'static str,
            dir_name: &'static str,
            prov: Option<String>,
            version: Option<&'static str>,
            corrupt_resource: bool,
            extra_file: bool,
            check: Box<dyn Fn(&VerificationOutcome)>,
        }

        let cases: Vec<TableCase> = vec![
            TableCase {
                name: "published",
                dir_name: "case_published",
                prov: Some(make_prov_json("2026-08-28", "ABDD194B1569D5FF3CDD81D618847F05642BD43C5B15D6CD43D8289B7466D801")),
                version: Some("0.1.0"),
                corrupt_resource: false,
                extra_file: false,
                check: Box::new(|outcome| {
                    assert!(matches!(outcome, VerificationOutcome::VerifiedPublished { date, version, .. } if date == "2026-08-28" && version == "0.1.0"));
                }),
            },
            TableCase {
                name: "A. date unknown",
                dir_name: "case_date_unknown",
                prov: Some(make_prov_json("2026-07-31", "8151248D1569D5FF3CDD81D618847F05642BD43C5B15D6CD43D8289B7466D801")),
                version: Some("0.1.0"),
                corrupt_resource: false,
                extra_file: false,
                check: Box::new(|outcome| {
                    assert!(matches!(outcome, VerificationOutcome::DateUnknown { date, version, .. } if date == "2026-07-31" && version == "0.1.0"));
                }),
            },
            TableCase {
                name: "B. different archive (version published)",
                dir_name: "case_diff_archive_published",
                prov: Some(make_prov_json("2026-08-28", "1111111111111111111111111111111111111111111111111111111111111111")),
                version: Some("0.1.0"),
                corrupt_resource: false,
                extra_file: false,
                check: Box::new(|outcome| {
                    assert!(matches!(outcome, VerificationOutcome::DifferentArchive { date, version, this_archive_sha256, published_archive_sha256, published_digest, .. }
                        if date == "2026-08-28" && version == "0.1.0"
                        && this_archive_sha256 == "1111111111111111111111111111111111111111111111111111111111111111"
                        && published_archive_sha256 == "ABDD194B1569D5FF3CDD81D618847F05642BD43C5B15D6CD43D8289B7466D801"
                        && published_digest.is_some()));
                    assert!(!outcome.is_verified());
                }),
            },
            TableCase {
                name: "B. different archive (version unpublished)",
                dir_name: "case_diff_archive_unpublished",
                prov: Some(make_prov_json("2026-08-28", "1111111111111111111111111111111111111111111111111111111111111111")),
                version: Some("0.3.0"),
                corrupt_resource: false,
                extra_file: false,
                check: Box::new(|outcome| {
                    assert!(matches!(outcome, VerificationOutcome::DifferentArchive { date, version, this_archive_sha256, published_archive_sha256, published_digest, .. }
                        if date == "2026-08-28" && version == "0.3.0"
                        && this_archive_sha256 == "1111111111111111111111111111111111111111111111111111111111111111"
                        && published_archive_sha256 == "ABDD194B1569D5FF3CDD81D618847F05642BD43C5B15D6CD43D8289B7466D801"
                        && published_digest.is_none()));
                    assert!(outcome.is_verified());
                }),
            },
            TableCase {
                name: "C. version unpublished",
                dir_name: "case_version_unpub",
                prov: Some(make_prov_json("2026-08-28", "ABDD194B1569D5FF3CDD81D618847F05642BD43C5B15D6CD43D8289B7466D801")),
                version: Some("0.3.0"),
                corrupt_resource: false,
                extra_file: false,
                check: Box::new(|outcome| {
                    assert!(matches!(outcome, VerificationOutcome::VersionUnpublished { date, version, published_versions, .. }
                        if date == "2026-08-28" && version == "0.3.0" && published_versions == &["0.1.0"]));
                }),
            },
            TableCase {
                name: "F. different bytes",
                dir_name: "case_diff_bytes",
                prov: Some(make_prov_json("2026-08-28", "ABDD194B1569D5FF3CDD81D618847F05642BD43C5B15D6CD43D8289B7466D801")),
                version: Some("0.1.0"),
                corrupt_resource: false,
                extra_file: true,
                check: Box::new(|outcome| {
                    assert!(matches!(outcome, VerificationOutcome::DifferentBytes { date, version, .. }
                        if date == "2026-08-28" && version == "0.1.0"));
                }),
            },
            TableCase {
                name: "D. changed since built",
                dir_name: "case_changed_since_built",
                prov: Some(make_prov_json("2026-08-28", "ABDD194B1569D5FF3CDD81D618847F05642BD43C5B15D6CD43D8289B7466D801")),
                version: Some("0.1.0"),
                corrupt_resource: true,
                extra_file: false,
                check: Box::new(|outcome| {
                    assert!(matches!(outcome, VerificationOutcome::ChangedSinceBuilt { file } if file == "orgs.parquet"));
                }),
            },
            TableCase {
                name: "no provenance",
                dir_name: "case_no_provenance",
                prov: None,
                version: Some("0.1.0"),
                corrupt_resource: false,
                extra_file: false,
                check: Box::new(|outcome| {
                    assert!(matches!(outcome, VerificationOutcome::NoProvenance));
                }),
            },
            TableCase {
                name: "corrupted (unreadable provenance)",
                dir_name: "case_corrupted_prov",
                prov: Some("not valid json at all".to_string()),
                version: Some("0.1.0"),
                corrupt_resource: false,
                extra_file: false,
                check: Box::new(|outcome| {
                    assert!(matches!(outcome, VerificationOutcome::Corrupted(err) if err.contains("isn't provenance this ods can read")));
                }),
            },
        ];

        for case in cases {
            let dir = base_tmp.path().join(case.dir_name);
            setup_fixture_dir(&dir, case.prov.as_deref(), case.version, case.corrupt_resource, case.extra_file);
            let outcome = verify_release_dir(&dir, &index);
            (case.check)(&outcome);
        }
    }
}




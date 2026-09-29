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
    /// The release directory holds `trud/`: TRUD's archive, pulled.
    pub has_trud: bool,
}

impl ReleaseInfo {
    /// Pulled from TRUD and not built: `trud/` and no Parquet files.
    pub fn is_pulled_not_built(&self) -> bool {
        self.has_trud && !self.has_parquet
    }
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
    if let Ok(idx) = crate::index::OdsReleaseIndex::from_slice(&bytes) {
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

/// Whether `dir` holds a readable, factual record of its release: Parquet files carrying
/// provenance (built or pulled), or a readable TRUD archive package, `trud/datapackage.json` (pulled
/// from TRUD, not yet built).
fn has_release_record(dir: &Path) -> bool {
    matches!(crate::provenance::read_release(dir), Ok(crate::provenance::ReleaseRecord::Provenanced(_)))
        || crate::provenance::TrudArchivePackage::load_from_file(&crate::provenance::trud_archive_package_path(dir)).ok().is_some()
}

/// The release date `dir`'s Parquet files name, when they carry provenance. For labels and
/// nudges only: a release that can't be read says nothing here, and the command that reads it
/// reports why.
pub fn embedded_release_date(dir: &Path) -> Option<String> {
    match crate::provenance::read_release(dir) {
        Ok(crate::provenance::ReleaseRecord::Provenanced(facts)) => Some(facts.release_date),
        _ => None,
    }
}

/// The dataset version `dir`'s Parquet files carry, when they carry provenance. Read from the
/// files alone: it is there whether or not the release verifies against an index.
pub fn embedded_dataset_version(dir: &Path) -> Option<String> {
    match crate::provenance::read_release(dir) {
        Ok(crate::provenance::ReleaseRecord::Provenanced(facts)) => Some(facts.dataset_version),
        _ => None,
    }
}

/// The release date `dir`'s TRUD archive package names, for a release pulled from TRUD.
fn trud_archive_package_date(dir: &Path) -> Option<String> {
    crate::provenance::TrudArchivePackage::load_from_file(&crate::provenance::trud_archive_package_path(dir))
        .ok()
        .map(|r| r.version)
}

/// Checks whether `dir/releases/` contains at least one date-shaped directory
/// (`\d{4}-\d{2}-\d{2}`) holding a readable release record (see `has_release_record`): a stored
/// release's Parquet files carrying provenance, or a TRUD archive package.
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
        if chrono::NaiveDate::parse_from_str(name, "%Y-%m-%d").is_ok() && has_release_record(&path) {
            return true;
        }
    }
    false
}

/// Checks the `_releases.json` marker in `dir`:
/// - Returns Ok(false) if file does not exist or fails validation (allowing candidate to qualify on releases).
/// - Returns Ok(true) if file exists, validates structurally, and does not contradict baked index.
/// - Returns Ok(true) for an index in the old format: it still marks a workspace, and a command
///   that reads it as its index refuses it, naming `ods pull`, which replaces it.
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
    if crate::index::is_old_format(&bytes) {
        return Ok(true);
    }
    let idx = match crate::index::OdsReleaseIndex::from_slice(&bytes) {
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

/// Whether `dir` is a workspace root: it holds a valid `_releases.json`, or a readable release.
/// A readable release without the marker is noted on stderr, once per process.
fn is_workspace_root(dir: &Path) -> Result<bool> {
    if check_releases_json(dir)? {
        return Ok(true);
    }
    if has_readable_release(dir) {
        emit_cache_notice_if_needed(dir);
        return Ok(true);
    }
    Ok(false)
}

/// Whether `rel`, a path relative to a workspace root, is one the workspace defines:
/// `releases`, `releases/<date>`, `releases/<date>/trud`, `releases/<date>/trud/oci` and anything
/// below it, or `current` (the active release's pointer) with the same paths below it.
fn is_workspace_defined_path(rel: &Path) -> bool {
    let Some(parts) = rel.components().map(|c| c.as_os_str().to_str()).collect::<Option<Vec<&str>>>() else {
        return false;
    };
    // What a release directory holds that a command may be run from: its `trud/`, and TRUD's OCI layout in it
    fn in_release(rest: &[&str]) -> bool {
        matches!(rest, [] | ["trud"] | ["trud", "oci", ..])
    }
    match parts.as_slice() {
        ["releases"] => true,
        ["releases", date, rest @ ..] => chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_ok() && in_release(rest),
        ["current", rest @ ..] => in_release(rest),
        _ => false,
    }
}

/// Resolves the root workspace directory from a specific starting path. It walks up only while
/// the path is recognisably inside a workspace:
/// 1. Explicit wins. A path from --workspace / -i / -o is the root. No further checks.
/// 2. `start` is a workspace root, or `start/ods_data` is: that is the root.
/// 3. `start` is inside a workspace root at a path the workspace defines (`releases/`,
///    `releases/<date>/`, `releases/<date>/trud/`, `releases/<date>/trud/oci/…`, `current`):
///    the root the path is relative to.
/// 4. Anything else: no workspace. A `.git` directory, `$HOME` and the directories between
///    are not consulted, so a directory made inside a project that has an `ods_data`
///    doesn't reach it.
///
/// Returns None if no workspace root was found.
pub fn find_workspace_root_from(start: &Path, explicit: Option<&Path>) -> Result<Option<PathBuf>> {
    if let Some(path) = explicit {
        return Ok(Some(path.to_path_buf()));
    }

    if is_workspace_root(start)? {
        return Ok(Some(start.to_path_buf()));
    }
    let default_ws = start.join(DEFAULT_WORKSPACE_DIR);
    if is_workspace_root(&default_ws)? {
        return Ok(Some(default_ws));
    }

    for root in start.ancestors().skip(1) {
        let Ok(rel) = start.strip_prefix(root) else {
            continue;
        };
        if is_workspace_defined_path(rel) && is_workspace_root(root)? {
            return Ok(Some(root.to_path_buf()));
        }
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
/// 3. If explicit itself is a release directory (holds `datapackage.json` or `trud/`):
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
        if input.join(crate::datapackage::DATAPACKAGE_FILENAME).exists() || input.join("trud").is_dir() {
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

/// The README `ods` writes in a workspace root: the same bytes in every workspace, with nothing
/// in it that changes (no dates, counts, names or release facts). Its first line marks it as
/// `ods`'s.
pub const WORKSPACE_README: &str = include_str!("workspace_readme.md");

/// The first line of `WORKSPACE_README`, which marks a README as `ods`'s to rewrite.
const WORKSPACE_README_MARKER: &str = "<!-- Written by ods, which rewrites this file: edits won't last. -->";

/// How every README an earlier `ods` generated began, before it carried the marker.
const OLDER_WORKSPACE_README_OPENING: &str = "# `ods` workspace\n\nThis directory holds verified releases of NHS Organisation Data as Parquet files, pulled by the `ods` CLI.";

/// Whether a workspace README is `ods`'s: it starts with the marker, or with an earlier `ods`'s
/// generated opening. Anything else is the user's.
fn is_ods_workspace_readme(content: &[u8]) -> bool {
    content.starts_with(WORKSPACE_README_MARKER.as_bytes()) || content.starts_with(OLDER_WORKSPACE_README_OPENING.as_bytes())
}

/// Writes the workspace README when it's missing, and rewrites it when it's `ods`'s and differs
/// from `WORKSPACE_README` (after an upgrade, say). A README the user wrote is left alone.
fn ensure_workspace_readme(workspace_root: &Path) -> Result<()> {
    let readme_path = workspace_root.join("README.md");
    let write = || fs::write(&readme_path, WORKSPACE_README).with_context(|| format!("writing {}", readme_path.display()));
    match fs::read(&readme_path) {
        Ok(existing) if existing == WORKSPACE_README.as_bytes() => Ok(()),
        Ok(existing) if is_ods_workspace_readme(&existing) => write(),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => write(),
        Err(e) => Err(e).with_context(|| format!("reading {}", readme_path.display())),
    }
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
    // The release's date is the one its Parquet files carry.
    let Some(date_str) = embedded_release_date(release_dir) else {
        return;
    };
    let Ok(rel_date) = chrono::NaiveDate::parse_from_str(&date_str, "%Y-%m-%d") else {
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
    // 1. The date its Parquet files carry, or its TRUD archive package's:
    if let Some(d) = embedded_release_date(dir).or_else(|| trud_archive_package_date(dir)) {
        return Some(d);
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

/// What `find`, `info` and `role` say about a release's provenance before they read it. Files
/// that disagree, or carry an object this `ods` can't read, are refused with the reader's `✖`
/// block. A release without provenance is warned about and still read. A directory with no
/// Parquet files says nothing here: the command's own not-found error covers it.
pub fn check_release_provenance(parquet_dir: &Path) -> Result<()> {
    match crate::provenance::read_release(parquet_dir)? {
        crate::provenance::ReleaseRecord::NoProvenance => eprintln!(
            "! {}\n  You can explore it, but not cite or verify it.",
            crate::provenance::describe_no_provenance(parquet_dir)
        ),
        crate::provenance::ReleaseRecord::Provenanced(_) | crate::provenance::ReleaseRecord::NoFiles => {}
    }
    Ok(())
}

/// Builds the source header lines for a resolved release directory and file name.
///
/// Returns:
/// 1. `* Source: releases/{date}/{file_name}` (or relative workspace path), muted if `color` is true.
/// 2. If running from a release directory different from the active workspace release,
///    an additional disagreement line:
///    `! Run from releases/{cwd_date}. Change source with: ods use {cwd_date}`
pub fn format_source_header(release_dir: &Path, file_name: &str, color: bool) -> Vec<String> {
    let release_date = embedded_release_date(release_dir)
        .or_else(|| trud_archive_package_date(release_dir))
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
                let has_trud = path.join("trud").is_dir();
                releases.push(ReleaseInfo {
                    date: date.to_string(),
                    path,
                    is_active,
                    has_parquet,
                    has_trud,
                });
            }
        }
    }

    releases.sort_by(|a, b| b.date.cmp(&a.date));
    Ok(releases)
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
    /// Parquet files with no `datapackage` key: built from an archive `ods` couldn't match to a
    /// TRUD release (`ods make --force -o`), or by an older `ods`.
    NoProvenance,
    /// The directory holds no Parquet files.
    NoFiles,
    /// The files can't be read as a release: the `✖` block that says why.
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
            | VerificationOutcome::NoProvenance
            | VerificationOutcome::NoFiles
            | VerificationOutcome::Corrupted(_) => false,
        }
    }

    pub fn is_verified_published(&self) -> bool {
        matches!(self, VerificationOutcome::VerifiedPublished { .. })
    }
}

/// Verifies a release directory against the release index by rebuilding its manifest from its
/// Parquet files (`crate::oci::dataset`) and comparing the digest with the index row's. The
/// facts, the release date, the dataset version and the source's hash, come from the object the
/// files carry. No stored `oci/` is needed or read.
pub fn verify_release_dir(
    release_dir: &Path,
    index: &crate::index::OdsReleaseIndex,
) -> VerificationOutcome {
    let facts = match crate::provenance::read_release(release_dir) {
        Ok(crate::provenance::ReleaseRecord::Provenanced(facts)) => *facts,
        Ok(crate::provenance::ReleaseRecord::NoProvenance) => return VerificationOutcome::NoProvenance,
        Ok(crate::provenance::ReleaseRecord::NoFiles) => return VerificationOutcome::NoFiles,
        Err(e) => return VerificationOutcome::Corrupted(format!("{:#}", e)),
    };
    let date = facts.release_date.clone();
    let version = facts.dataset_version.clone();

    let reconstructed_digest = match crate::oci::dataset::build(release_dir, &facts).and_then(|(m, _)| m.digest()) {
        Ok(d) => d,
        Err(e) => return VerificationOutcome::Corrupted(format!("✖ Can't rebuild the manifest for {}: {:#}", relative_to_cwd(release_dir).display(), e)),
    };

    let release_entry = index.release(&date);
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

    let this_archive_sha256 = facts.source_sha256_upper();
    if release_row.source.hash != facts.source.hash {
        let published_digest = release_row
            .datasets
            .iter()
            .find(|d| d.version == facts.version())
            .map(|d| d.manifest_digest.clone());

        return VerificationOutcome::DifferentArchive {
            date,
            version,
            digest: reconstructed_digest,
            this_archive_sha256,
            published_archive_sha256: release_row.source.sha256_hex().to_uppercase(),
            published_digest,
        };
    }

    let dataset_entry = release_row.datasets.iter().find(|d| d.version == facts.version());
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
            let published_versions = release_row.datasets.iter().map(|d| d.dataset_version().to_string()).collect();
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
        use crate::commands::parquet::write_stub_parquet;
        let base_tmp = tempfile::tempdir().unwrap();
        let published_sha = "ABDD194B1569D5FF3CDD81D618847F05642BD43C5B15D6CD43D8289B7466D801";
        let other_sha = "1111111111111111111111111111111111111111111111111111111111111111";

        // The object `ods make` embeds for a release of `date`, from an archive with `sha`, at
        // dataset `version`.
        let embedded = |date: &str, sha: &str, version: &str| {
            crate::provenance::TrudArchivePackage::for_trud_release(date, "archive.zip", sha, 38064419, &[])
                .unwrap()
                .embedded(version)
                .unwrap()
        };

        // Reference published directory, for the published manifest digest
        let ref_dir = base_tmp.path().join("ref_published");
        fs::create_dir_all(&ref_dir).unwrap();
        write_stub_parquet(&ref_dir.join("orgs.parquet"), Some(&embedded("2026-08-28", published_sha, "0.1.0")), "orgs").unwrap();
        let (manifest, _, _) = crate::oci::dataset::build_from_dir(&ref_dir).unwrap();
        let published_digest = manifest.digest().unwrap();

        let index = crate::index::OdsReleaseIndex {
            mirrors: vec![],
            releases: vec![crate::index::Release {
                source: crate::index::SourceRelease {
                    version: "2026-08-28".to_string(),
                    hash: crate::provenance::prefixed_sha256(published_sha),
                    bytes: 38064419,
                    issues: vec![],
                },
                datasets: vec![crate::index::Dataset {
                    version: "2026-08-28_0.1.0".to_string(),
                    manifest_digest: published_digest.clone(),
                    bytes: 1000,
                    tool_version: "0.2.0".to_string(),
                    tool_git_sha: "0123456789abcdef0123456789abcdef01234567".to_string(),
                    doi: None,
                    withdrawn: None,
                }],
            }],
            ..crate::index::OdsReleaseIndex::default()
        };

        // Each case: a directory name, and the files to write into it as (file, embedded
        // object or none, content). `None` for the object writes a file with no `datapackage`
        // key; a content of "not parquet" writes bytes that aren't a Parquet file at all.
        type Files = Vec<(&'static str, Option<crate::provenance::Embedded>, &'static str)>;
        type Check = Box<dyn Fn(&VerificationOutcome)>;
        let published = || Some(embedded("2026-08-28", published_sha, "0.1.0"));
        let cases: Vec<(&str, Files, Check)> = vec![
            ("published", vec![("orgs.parquet", published(), "orgs")], Box::new(|o| {
                assert!(matches!(o, VerificationOutcome::VerifiedPublished { date, version, .. } if date == "2026-08-28" && version == "0.1.0"), "{o:?}");
            })),
            ("A. date unknown", vec![("orgs.parquet", Some(embedded("2026-07-31", published_sha, "0.1.0")), "orgs")], Box::new(|o| {
                assert!(matches!(o, VerificationOutcome::DateUnknown { date, version, .. } if date == "2026-07-31" && version == "0.1.0"), "{o:?}");
            })),
            ("B. different archive (version published)", vec![("orgs.parquet", Some(embedded("2026-08-28", other_sha, "0.1.0")), "orgs")], Box::new(move |o| {
                assert!(matches!(o, VerificationOutcome::DifferentArchive { this_archive_sha256, published_archive_sha256, published_digest, .. }
                    if this_archive_sha256 == other_sha && published_archive_sha256 == published_sha && published_digest.is_some()), "{o:?}");
                assert!(!o.is_verified());
            })),
            ("B. different archive (version unpublished)", vec![("orgs.parquet", Some(embedded("2026-08-28", other_sha, "0.3.0")), "orgs")], Box::new(|o| {
                assert!(matches!(o, VerificationOutcome::DifferentArchive { version, published_digest, .. } if version == "0.3.0" && published_digest.is_none()), "{o:?}");
                assert!(o.is_verified());
            })),
            ("C. version unpublished", vec![("orgs.parquet", Some(embedded("2026-08-28", published_sha, "0.3.0")), "orgs")], Box::new(|o| {
                assert!(matches!(o, VerificationOutcome::VersionUnpublished { version, published_versions, .. } if version == "0.3.0" && published_versions == &["0.1.0"]), "{o:?}");
            })),
            ("F. different bytes", vec![("orgs.parquet", published(), "orgs"), ("roles.parquet", published(), "roles")], Box::new(|o| {
                assert!(matches!(o, VerificationOutcome::DifferentBytes { date, version, .. } if date == "2026-08-28" && version == "0.1.0"), "{o:?}");
            })),
            ("no provenance", vec![("orgs.parquet", None, "orgs")], Box::new(|o| {
                assert!(matches!(o, VerificationOutcome::NoProvenance), "{o:?}");
            })),
            ("files disagree", vec![("orgs.parquet", published(), "orgs"), ("roles.parquet", Some(embedded("2026-08-28", published_sha, "0.3.0")), "roles")], Box::new(|o| {
                assert!(matches!(o, VerificationOutcome::Corrupted(block) if block.contains("don't carry the same provenance") && block.contains("orgs.parquet") && block.contains("roles.parquet")), "{o:?}");
            })),
            ("not a parquet file", vec![("orgs.parquet", None, "not parquet")], Box::new(|o| {
                assert!(matches!(o, VerificationOutcome::Corrupted(block) if block.contains("can't be read as a Parquet file")), "{o:?}");
            })),
        ];

        for (name, files, check) in cases {
            let dir = base_tmp.path().join(name.replace(['.', ' ', '(', ')'], "_"));
            fs::create_dir_all(&dir).unwrap();
            for (file, object, content) in files {
                if content == "not parquet" {
                    fs::write(dir.join(file), b"dummy orgs content").unwrap();
                } else {
                    write_stub_parquet(&dir.join(file), object.as_ref(), content).unwrap();
                }
            }
            check(&verify_release_dir(&dir, &index));
        }
    }
}




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
///
/// Reduces to one accepted shape: parses as `OdsReleaseIndex`, and its `_type` is `ods_release_index`.
pub fn validate_releases_json(dir: &Path) -> bool {
    let path = dir.join(crate::index::RELEASES_JSON_FILENAME);
    if !path.is_file() {
        return false;
    }
    let Ok(bytes) = fs::read(&path) else {
        return false;
    };
    if let Ok(idx) = serde_json::from_slice::<crate::index::OdsReleaseIndex>(&bytes) {
        return idx.type_tag == "ods_release_index";
    }
    false
}

#[derive(Debug)]
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    /// Find an existing workspace. Never creates. For read commands.
    pub fn open(explicit: Option<&Path>) -> Result<Workspace> {
        let root = if let Some(path) = explicit {
            if !path.exists() {
                anyhow::bail!(
                    "✖ no ods workspace found at '{}'\n  Pass -i <trud.zip> -o <dir>, or run `ods pull` or `ods trud pull` to create a workspace.",
                    path.display()
                );
            }
            if validate_releases_json(path) {
                path.to_path_buf()
            } else if let Some(found) = find_workspace_root_from(path, None) {
                found
            } else {
                anyhow::bail!(
                    "✖ no ods workspace found at '{}'\n  Pass -i <trud.zip> -o <dir>, or run `ods pull` or `ods trud pull` to create a workspace.",
                    path.display()
                );
            }
        } else {
            find_workspace_root(None).ok_or_else(|| {
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
            if validate_releases_json(path) {
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
            find_workspace_root(None).unwrap_or_else(|| PathBuf::from(DEFAULT_WORKSPACE_DIR))
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
/// Returns None if no workspace root was found.
pub fn find_workspace_root_from(start: &Path, explicit: Option<&Path>) -> Option<PathBuf> {
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
) -> Option<PathBuf> {
    // 1. Explicit wins. No further checks.
    if let Some(path) = explicit {
        return Some(path.to_path_buf());
    }

    // 2. Walk up. For each ancestor a, starting at start and ascending:
    // a/_releases.json validates -> root = a.
    // a/<DEFAULT_WORKSPACE_DIR>/_releases.json validates -> root = a/<DEFAULT_WORKSPACE_DIR>.
    // Stop before ascending past: the first a that contains a .git entry, $HOME, or the filesystem root.
    let mut current = Some(start);
    while let Some(a) = current {
        if validate_releases_json(a) {
            return Some(a.to_path_buf());
        }
        let default_ws = a.join(DEFAULT_WORKSPACE_DIR);
        if validate_releases_json(&default_ws) {
            return Some(default_ws);
        }

        // Stop before ascending past: the first a that contains a .git entry, $HOME, or the filesystem root.
        if a.join(".git").exists() || home.is_some_and(|h| a == h) {
            break;
        }

        current = a.parent();
    }

    None
}

/// Resolves the root workspace directory from current working directory or explicit option.
fn find_workspace_root(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return Some(path.to_path_buf());
    }
    let pwd = std::env::current_dir().ok()?;
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
/// 5. Otherwise fall through to `Workspace::open(Some(explicit))?.parquet_dir()`.
///
/// When no explicit path is given (None):
/// Falls back to `Workspace::open(None)?.parquet_dir()`.
pub fn resolve_parquet_input(explicit: Option<&Path>) -> Result<PathBuf> {
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
        let ws = Workspace::open(Some(input))?;
        return ws.parquet_dir();
    }

    let ws = Workspace::open(None)?;
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

/// Emits a one-line staleness nudge to stderr if the newest release in the index is older than 45 days.
/// At most once per process invocation.
/// Strictly suppressed on machine-readable formats (`json`, `csv`, `tsv`, `ndjson`).
pub fn check_and_emit_staleness_nudge(index: &crate::index::OdsReleaseIndex, is_machine_readable: bool) {
    if is_machine_readable {
        return;
    }
    if NUDGE_EMITTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let today = chrono::Utc::now().date_naive();
    if let Some((newest_date, days)) = index.staleness(today) {
        if days > crate::index::STALENESS_THRESHOLD_DAYS {
            eprintln!(
                "! {} is {} days old. TRUD ships roughly every 4 weeks\n  Check with: ods pull",
                newest_date, days
            );
        }
    }
}

/// Detects if the current working directory is a release directory.
/// Returns Some("YYYY-MM-DD") if cwd is a release directory, or None otherwise.
pub fn detect_cwd_release() -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    // 1. If cwd has _provenance.json with trud_release_date:
    if let Some(prov) = crate::provenance::OdsProvenance::load_from_dir(&cwd) {
        if let Some(d) = prov.trud_release_date {
            return Some(d);
        }
    }
    // 2. If parent directory is named "releases" and folder name matches YYYY-MM-DD:
    // Structural inspection of cwd is strictly for the disagreement UX nudge; it never discovers workspaces or selects data.
    if let Some(parent) = cwd.parent() {
        if parent.file_name().and_then(|n| n.to_str()) == Some("releases") {
            if let Some(name) = cwd.file_name().and_then(|n| n.to_str()) {
                if chrono::NaiveDate::parse_from_str(name, "%Y-%m-%d").is_ok() {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

/// Resolves the active release date of the enclosing workspace (if any).
pub fn workspace_active_release(dir: Option<&Path>) -> Option<String> {
    if let Some(d) = dir {
        if let Some(root) = find_workspace_root_from(d, None) {
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
pub fn check_release_resolution(release_date: &str, release_dir: Option<&Path>) -> ReleaseResolution {
    let is_current = workspace_active_release(release_dir)
        .map(|act| act == release_date)
        .unwrap_or(false);

    if is_current {
        if let Some(cwd_date) = detect_cwd_release() {
            if cwd_date != release_date {
                return ReleaseResolution::Disagreement { cwd_date };
            }
        }
        ReleaseResolution::Current
    } else {
        ReleaseResolution::ExplicitNonCurrent
    }
}

/// Reports the release date read from to stderr in the two-space gutter.
/// Appends `(current)` only when the resolved release equals the workspace's active release;
/// otherwise prints the date bare.
/// If cwd is a release directory other than current, reports the disagreement and offers `ods use <cwd_date>`.
/// Suppressed entirely on machine-readable formats.
pub fn report_release_resolution(
    release_date: &str,
    release_dir: Option<&Path>,
    is_machine_readable: bool,
) {
    if is_machine_readable {
        return;
    }
    match check_release_resolution(release_date, release_dir) {
        ReleaseResolution::Current => {
            eprintln!("  {} (current)", release_date);
        }
        ReleaseResolution::Disagreement { cwd_date } => {
            eprintln!("  {} (current), not the {} you're in", release_date, cwd_date);
            eprintln!("  Switch with: ods use {}", cwd_date);
        }
        ReleaseResolution::ExplicitNonCurrent => {
            eprintln!("  {}", release_date);
        }
    }
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
        "# NHS Organisation Data Service (ODS) Workspace\n\n\
         This directory contains versioned NHS Organisation Data managed by the `ods` CLI.\n\n\
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
    custom_index: Option<&crate::index::OdsReleaseIndex>,
) -> VerificationOutcome {
    let prov = match crate::provenance::OdsProvenance::load_from_dir(release_dir) {
        Some(p) => p,
        None => return VerificationOutcome::Corrupted("Missing or unreadable _provenance.json".to_string()),
    };

    let date = match prov.trud_release_date.as_deref() {
        Some(d) => d.to_string(),
        None => return VerificationOutcome::Corrupted("Provenance missing trud_release_date".to_string()),
    };

    let version = match prov.dataset_version.clone() {
        Some(v) => v,
        None => return VerificationOutcome::Corrupted("Provenance missing dataset_version".to_string()),
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

    let baked_index = crate::index::OdsReleaseIndex::baked().ok();
    let index_to_check = custom_index.or(baked_index.as_ref());

    if let Some(index) = index_to_check {
        if let Some(entry) = index
            .releases
            .iter()
            .find(|r| r.trud_release_date == date && r.dataset_version == version)
        {
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
    }

    VerificationOutcome::VerifiedUnpublished {
        date,
        version,
        digest: reconstructed_digest,
    }
}

pub fn is_release_dir_verified(release_dir: &Path) -> bool {
    verify_release_dir(release_dir, None).is_verified()
}



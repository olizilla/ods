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
    let path = dir.join("_releases.json");
    if !path.is_file() {
        return false;
    }
    let Ok(bytes) = fs::read(&path) else {
        return false;
    };
    if serde_json::from_slice::<crate::index::CachedReleaseIndex>(&bytes).is_ok() {
        return true;
    }
    if serde_json::from_slice::<crate::index::OdsReleaseIndex>(&bytes).is_ok() {
        return true;
    }
    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&bytes) {
        if val.get("_type").and_then(|t| t.as_str()) == Some("ods_release_index") {
            return true;
        }
        if val
            .get("index")
            .and_then(|i| i.get("_type"))
            .and_then(|t| t.as_str())
            == Some("ods_release_index")
        {
            return true;
        }
    }
    false
}

/// Backward-compatible alias for workspace root validation.
pub fn is_workspace_root(dir: &Path) -> bool {
    validate_releases_json(dir)
}

/// Resolves the root workspace directory from a specific starting path:
/// 1. Explicit wins. A path from --workspace / -i / -o is the root. No further checks.
/// 2. start is the root. start/_releases.json validates -> root = start.
/// 3. start is a release dir. start/_provenance.json exists and start's parent is named releases and start/../../_releases.json validates -> root = start/../..
/// 4. Walk up. For each ancestor a, starting at start and ascending:
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

    // 2. start is the root. start/_releases.json validates -> root = start.
    if validate_releases_json(start) {
        return Some(start.to_path_buf());
    }

    // 3. start is a release dir. start/_provenance.json exists and start's parent is named releases and start/../../_releases.json validates -> root = start/../..
    if start.join(crate::provenance::PROVENANCE_FILENAME).exists() {
        if let Some(parent) = start.parent() {
            if parent.file_name().is_some_and(|n| n == "releases") {
                if let Some(grandparent) = parent.parent() {
                    if validate_releases_json(grandparent) {
                        return Some(grandparent.to_path_buf());
                    }
                }
            } else if validate_releases_json(parent) {
                return Some(parent.to_path_buf());
            }
        }
    }

    // Also check canonicalized start for symlink pointers (like `current`)
    if let Ok(canon) = start.canonicalize() {
        if canon != start && canon.join(crate::provenance::PROVENANCE_FILENAME).exists() {
            if let Some(parent) = canon.parent() {
                if parent.file_name().is_some_and(|n| n == "releases") {
                    if let Some(grandparent) = parent.parent() {
                        if validate_releases_json(grandparent) {
                            return Some(grandparent.to_path_buf());
                        }
                    }
                }
            }
        }
    }

    // 4. Walk up. For each ancestor a, starting at start and ascending:
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
pub fn find_workspace_root(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return Some(path.to_path_buf());
    }
    let pwd = std::env::current_dir().ok()?;
    find_workspace_root_from(&pwd, explicit)
}

/// Legacy wrapper for discover_dataset_dir.
pub fn discover_parquet_dir(user_input: Option<&Path>) -> Result<PathBuf> {
    discover_dataset_dir(user_input, None)
}

/// Discovers the active dataset/parquet directory:
/// 1. User-supplied `--input` argument (if explicit)
/// 2. Workspace root's active release directory (or unpinned diagnostic error)
pub fn discover_dataset_dir(
    user_input: Option<&Path>,
    workspace_override: Option<&Path>,
) -> Result<PathBuf> {
    if let Some(input) = user_input {
        if input.join("orgs.parquet").exists() {
            return Ok(input.to_path_buf());
        }
        if input.join("current").join("orgs.parquet").exists() {
            return Ok(input.join("current"));
        }
        if input.join("parquet").join("orgs.parquet").exists() {
            return Ok(input.join("parquet"));
        }
        if input.join("current").join("parquet").join("orgs.parquet").exists() {
            return Ok(input.join("current").join("parquet"));
        }
        if input.exists() {
            return Ok(input.to_path_buf());
        }
    }

    if let Some(workspace_root) = find_workspace_root(workspace_override) {
        if let Ok((_date, active_dir)) = get_active_release(&workspace_root) {
            if active_dir.join("orgs.parquet").exists() {
                return Ok(active_dir);
            }
            if active_dir.join("parquet").join("orgs.parquet").exists() {
                return Ok(active_dir.join("parquet"));
            }
            return Ok(active_dir);
        }

        let releases = list_releases(&workspace_root).unwrap_or_default();
        if !releases.is_empty() {
            let n = releases.len();
            let count_str = if n == 1 {
                "1 release".to_string()
            } else {
                format!("{} releases", n)
            };
            let newest_date = &releases[0].date;
            let ws_name = workspace_root
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
    }

    Err(anyhow!(
        "✖ no ods workspace found here\n  Pass -i <trud.zip> -o <dir>, or run `ods pull` to create a workspace."
    ))
}

/// Ensures the directory structure for a specific release date inside workspace root:
/// `./<workspace>/releases/<date>/trud/`
pub fn prepare_release_dir(workspace_root: &Path, release_date: &str) -> Result<PathBuf> {
    let release_dir = workspace_root.join("releases").join(release_date);
    fs::create_dir_all(release_dir.join("trud"))
        .context("Failed to create release trud directory")?;
    ensure_workspace_gitignore(workspace_root)?;
    Ok(release_dir)
}

/// Ensures `.gitignore` ignores heavy raw ZIP/XML files while preserving metadata/parquet artifacts.
pub fn ensure_workspace_gitignore(workspace_root: &Path) -> Result<()> {
    let gitignore_path = workspace_root.join(".gitignore");
    if !gitignore_path.exists() {
        let content = "# Ignore raw TRUD archive downloads and extracted XML files\nreleases/*/trud/\n*.zip\n*.xml\n";
        fs::write(&gitignore_path, content)
            .context("Failed to write workspace .gitignore file")?;
    }
    Ok(())
}

/// Sets or updates the `current` symlink/pointer in the workspace root to target `releases/<release_date>`.
pub fn set_active_release(workspace_root: &Path, release_date: &str) -> Result<()> {
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
pub fn get_active_release(workspace_root: &Path) -> Result<(String, PathBuf)> {
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
pub fn list_releases(workspace_root: &Path) -> Result<Vec<ReleaseInfo>> {
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
pub fn generate_workspace_readme(
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



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
    pub has_ndjson: bool,
    pub has_markdown: bool,
}

/// Discovers the active Parquet directory based on the 4-tier hierarchy:
/// 1. User-supplied `--input` argument (if explicit)
/// 2. Local workspace `./ods_data/current/parquet/`
/// 3. Parent directory traversal `../ods_data/current/parquet/`
/// 4. Local directory `./parquet/` or `./`
pub fn discover_parquet_dir(user_input: Option<&Path>) -> Result<PathBuf> {
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
        if input.join(DEFAULT_WORKSPACE_DIR).join("current").join("orgs.parquet").exists() {
            return Ok(input.join(DEFAULT_WORKSPACE_DIR).join("current"));
        }
        if input.exists() {
            return Ok(input.to_path_buf());
        }
    }

    let cwd = std::env::current_dir().context("Failed to get current working directory")?;

    // Check local `./ods_data/current/`
    let local_current = cwd.join(DEFAULT_WORKSPACE_DIR).join("current");
    if local_current.join("orgs.parquet").exists() {
        return Ok(local_current);
    }
    if local_current.join("parquet").join("orgs.parquet").exists() {
        return Ok(local_current.join("parquet"));
    }

    // Check parent directory traversal `../ods_data/current/`
    if let Some(parent) = cwd.parent() {
        let parent_current = parent.join(DEFAULT_WORKSPACE_DIR).join("current");
        if parent_current.join("orgs.parquet").exists() {
            return Ok(parent_current);
        }
        if parent_current.join("parquet").join("orgs.parquet").exists() {
            return Ok(parent_current.join("parquet"));
        }
    }

    // Check fallback `./parquet/`
    let local_parquet = cwd.join("parquet");
    if local_parquet.join("orgs.parquet").exists() {
        return Ok(local_parquet);
    }

    // Check current directory directly `./orgs.parquet`
    if cwd.join("orgs.parquet").exists() {
        return Ok(cwd);
    }

    Err(anyhow!(
        "No ODS Parquet dataset found.\n\
         Please provide a dataset directory via `--input <path>`, run `ods parquet <xml>` to compile a dataset, or populate `./ods_data/`."
    ))
}

/// Resolves the root workspace directory (`./ods_data/` or parent).
pub fn find_workspace_root() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    if cwd.join(DEFAULT_WORKSPACE_DIR).is_dir() {
        return Some(cwd.join(DEFAULT_WORKSPACE_DIR));
    }
    if let Some(parent) = cwd.parent() {
        if parent.join(DEFAULT_WORKSPACE_DIR).is_dir() {
            return Some(parent.join(DEFAULT_WORKSPACE_DIR));
        }
    }
    None
}

/// Ensures the directory structure for a specific release date inside workspace root:
/// `./ods_data/releases/<date>/parquet/`
/// `./ods_data/releases/<date>/ndjson/`
/// `./ods_data/releases/<date>/markdown/`
pub fn prepare_release_dir(workspace_root: &Path, release_date: &str) -> Result<PathBuf> {
    let release_dir = workspace_root.join("releases").join(release_date);
    fs::create_dir_all(release_dir.join("trud"))
        .context("Failed to create release trud directory")?;
    fs::create_dir_all(release_dir.join("markdown"))
        .context("Failed to create release markdown directory")?;
    ensure_workspace_gitignore(workspace_root)?;
    Ok(release_dir)
}

/// Ensures `./ods_data/.gitignore` ignores heavy raw ZIP/XML files while preserving metadata/parquet artifacts.
pub fn ensure_workspace_gitignore(workspace_root: &Path) -> Result<()> {
    let gitignore_path = workspace_root.join(".gitignore");
    if !gitignore_path.exists() {
        let content = "# Ignore raw TRUD archive downloads and extracted XML files\nreleases/*/trud/\n*.zip\n*.xml\n!releases/*/markdown/wiki.zip\n";
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

/// Lists all local releases in `./ods_data/releases/`.
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
                let has_ndjson = path.join("ods.ndjson").exists() || path.join("ndjson").join("ods.ndjson").exists();
                let has_markdown = path.join("markdown").join("wiki.zip").exists();
                releases.push(ReleaseInfo {
                    date: date.to_string(),
                    path,
                    is_active,
                    has_parquet,
                    has_ndjson,
                    has_markdown,
                });
            }
        }
    }

    releases.sort_by(|a, b| b.date.cmp(&a.date));
    Ok(releases)
}

/// Generates the self-documenting `./ods_data/README.md` file.
pub fn generate_workspace_readme(
    workspace_root: &Path,
    release_date: &str,
    seq_num: Option<&str>,
    entity_count: Option<usize>,
) -> Result<()> {
    let readme_path = workspace_root.join("README.md");
    let seq_info = seq_num.map(|s| format!(" (TRUD Sequence #{s})")).unwrap_or_default();
    let count_info = entity_count.map(|c| format!("{c} entities")).unwrap_or_else(|| "Active dataset".to_string());

    let content = format!(
        "# NHS Organisation Data Service (ODS) Workspace\n\n\
         This directory contains versioned NHS Organisation Data managed by the `ods` CLI.\n\n\
         - **Active Release**: {release_date}{seq_info}\n\
         - **Status**: {count_info} indexed in `current/parquet/`\n\n\
         ## Directory Structure\n\n\
         - `current/`: Symlink pointing to active release\n\
         - `releases/`: Dated release snapshots containing `parquet/`, `ndjson/`, and `markdown/`\n\n\
         ## Quick Start: Querying with DuckDB\n\n\
         ```sql\n\
         -- Query active GP practices in Sedbergh\n\
         SELECT name, ods_code, postcode, telephone\n\
         FROM 'ods_data/current/parquet/orgs.parquet'\n\
         WHERE status = 'active' AND town = 'SEDBERGH';\n\
         ```\n\n\
         ## Python Integration\n\n\
         ```python\n\
         import duckdb\n\
         con = duckdb.connect()\n\
         df = con.execute(\"SELECT * FROM 'ods_data/current/parquet/orgs.parquet' WHERE status = 'active'\").df()\n\
         ```\n\n\
         ## CLI Commands\n\n\
         - `ods find <query>`: Fast terminal lookup\n\
         - `ods status`: Inspect local workspace release versions\n\
         - `ods switch <date>`: Switch active release pin\n\
         - `ods diff`: Diffs active release against previous release\n\
         - `ods cite`: Output APA and BibTeX academic citations\n"
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


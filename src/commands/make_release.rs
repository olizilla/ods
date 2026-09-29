use anyhow::{bail, Context, Result};
use clap::Parser;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::index::{parse_semver, sort_datasets, Dataset, OdsReleaseIndex, Release, SourceRelease, RELEASES_SCHEMA_V1_URL};
use crate::oci::*;
use crate::provenance::{ReleaseFacts, ReleaseRecord};

#[derive(Parser, Debug, Clone)]
pub struct Args {
    /// Path to compiled release directory (defaults to active release)
    #[arg(long, short)]
    pub input: Option<PathBuf>,

    /// Optional Zenodo DOI for the dataset release
    #[arg(long)]
    pub doi: Option<String>,

    /// Path to git repository containing ods tool (defaults to auto-detection)
    #[arg(long)]
    pub tool_repo: Option<PathBuf>,

    /// Path to release index file (defaults to data/releases.json)
    #[arg(long)]
    pub index: Option<PathBuf>,
}

/// The `ods` that is recording a release: the build that runs `ods make release`. The index
/// row names it, so the record says which commit built the dataset. It is an argument to the
/// checks, not read from the environment, so a test can name the build it wants and no
/// environment variable can claim a commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildIdentity {
    /// The `version` in `Cargo.toml`, which is the tag without its `v`.
    pub tool_version: String,
    pub git_sha: Option<String>,
    pub dirty: bool,
}

impl BuildIdentity {
    /// The identity compiled into this binary by `build.rs`.
    pub fn compiled() -> Self {
        Self {
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
            git_sha: option_env!("ODS_GIT_SHA").map(|s| s.to_string()),
            dirty: option_env!("ODS_GIT_DIRTY").is_some(),
        }
    }
}

fn is_tool_repo_dir(dir: &Path) -> bool {
    if !dir.join(".git").exists() || !dir.join("Cargo.toml").exists() {
        return false;
    }
    let content = match fs::read_to_string(dir.join("Cargo.toml")) {
        Ok(c) => c,
        Err(_) => return false,
    };

    let mut in_package_section = false;
    let mut is_ods_package = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_package_section = trimmed == "[package]";
        } else if in_package_section {
            let stripped: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
            if stripped == "name=\"ods\"" || stripped == "name='ods'" {
                is_ods_package = true;
                break;
            }
        }
    }

    is_ods_package && (dir.join("src").join("main.rs").exists() || dir.join("src").join("lib.rs").exists())
}

pub fn find_tool_repo(release_dir: &Path) -> Option<PathBuf> {
    for ancestor in release_dir.ancestors() {
        if is_tool_repo_dir(ancestor) {
            return Some(ancestor.to_path_buf());
        }
    }
    if let Ok(mut cur) = std::env::current_dir() {
        loop {
            if is_tool_repo_dir(&cur) {
                return Some(cur);
            }
            if !cur.pop() {
                break;
            }
        }
    }
    if let Ok(dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let p = PathBuf::from(dir);
        if is_tool_repo_dir(&p) {
            return Some(p);
        }
    }
    None
}

pub fn run(args: Args) -> Result<()> {
    run_as(args, &BuildIdentity::compiled())
}

pub fn run_as(args: Args, build: &BuildIdentity) -> Result<()> {
    let release_dir = match args.input {
        Some(ref p) => p.clone(),
        None => {
            bail!("✖ --input is required for 'ods make release'\n  Pass the release directory to publish, e.g. ods make release --input <release-dir>");
        }
    };

    if !release_dir.exists() {
        bail!("Release directory does not exist: {}", release_dir.display());
    }

    // What the release is comes from its Parquet files: the object they all carry.
    let facts = match crate::provenance::read_release(&release_dir)? {
        ReleaseRecord::Provenanced(facts) => *facts,
        other => bail!("{}", crate::provenance::format_record_refusal(&release_dir, &other).unwrap_or_default()),
    };
    let version = facts.dataset_version.clone();

    let tool_repo = match args.tool_repo {
        Some(ref p) => {
            if is_tool_repo_dir(p) {
                Some(p.clone())
            } else {
                None
            }
        }
        None => find_tool_repo(&release_dir),
    };

    // 1. Read index
    let (custom_index, target_index_path) = if let Some(ref p) = args.index {
        (Some(read_index_file(p)?), p.clone())
    } else if let Some(ref tr) = tool_repo {
        let p = tr.join("data").join("releases.json");
        if p.exists() {
            (Some(read_index_file(&p)?), p)
        } else {
            (None, p)
        }
    } else {
        bail!("cannot locate the ods repository — data/releases.json is where a release row is reviewed\n  Pass --tool-repo, or run from inside the repo.");
    };

    // 2. Run Checks 10-14 before any writes
    let failures = perform_all_release_checks(
        &release_dir,
        &version,
        tool_repo.as_deref(),
        custom_index.as_ref(),
        build,
    )?;

    if !failures.is_empty() {
        for failure in &failures {
            eprintln!("✖ {}", failure);
        }
        let count = failures.len();
        eprintln!("{} check{} failed", count, if count == 1 { "" } else { "s" });
        bail!("Release checks failed:\n{}", failures.join("\n"));
    }

    // 3. All checks passed: run ods make oci to generate oci/ wholesale
    crate::commands::make_oci::run(crate::commands::make_oci::Args {
        input: Some(release_dir.clone()),
        ..Default::default()
    })?;

    let date = facts.release_date.as_str();

    // 4. Read the manifest digest from oci/index.json: the entry whose ref name is
    // `<date>_<version>` names the manifest `ods make oci` just wrote, so there's no need to
    // pick out the one blob in oci/blobs/sha256/ that isn't a symlink.
    let oci_dir = release_dir.join("oci");
    let blobs_dir = oci_dir.join("blobs").join("sha256");
    let index_bytes = fs::read(oci_dir.join("index.json")).context("reading oci/index.json")?;
    let oci_index: OciIndex =
        serde_json::from_slice(&index_bytes).context("parsing oci/index.json")?;
    let ref_versioned = format!("{}_{}", date, version);
    let manifest_digest = oci_index
        .manifests
        .iter()
        .find(|m| {
            m.annotations
                .as_ref()
                .and_then(|a| a.get(ANNOTATION_REF_NAME))
                .map(|r| r == &ref_versioned)
                .unwrap_or(false)
        })
        .map(|m| m.digest.clone())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "oci/index.json missing manifest entry with ref name '{}'",
                ref_versioned
            )
        })?;
    let manifest_bytes = fs::read(blobs_dir.join(manifest_digest.trim_start_matches("sha256:")))
        .with_context(|| format!("reading manifest blob {}", manifest_digest))?;
    let manifest: OciManifest = serde_json::from_slice(&manifest_bytes)?;

    // Summary lines on progress
    let total_bytes: u64 = manifest.layers.iter().map(|l| l.size).sum();
    let mb = (total_bytes as f64) / (1024.0 * 1024.0);
    eprintln!("* {} layers, {:.1} MB, manifest {}", manifest.layers.len(), mb, manifest_digest);

    // 5. Update data/releases.json (strictly after the checks and the oci build succeed).
    // Publishing the bytes and laying out ods.fyi's bucket keys is scripts/mirror-to-ods-fyi.sh's
    // job, from the pushed OCI image, not this command's.
    let mut index: OdsReleaseIndex = if target_index_path.exists() {
        OdsReleaseIndex::from_slice(&fs::read(&target_index_path)?)?
    } else {
        OdsReleaseIndex::baked()?
    };

    // Guarantee the built release's row, from the source the files name
    if index.release(date).is_none() {
        index.releases.push(release_row_for(&facts));
    }

    // Add the dataset (or preserve the existing row on an identical re-publish)
    let dataset_version = facts.version().to_string();
    let new_dataset = Dataset {
        version: dataset_version.clone(),
        manifest_digest: manifest_digest.clone(),
        bytes: total_bytes,
        tool_version: build.tool_version.clone(),
        tool_git_sha: build.git_sha.clone().unwrap_or_default().to_lowercase(),
        doi: args.doi.clone(),
        withdrawn: None,
    };
    let rel = index
        .releases
        .iter_mut()
        .find(|r| r.source.version == date)
        .expect("Release row for date must exist");
    if let Some(existing_ds) = rel.datasets.iter_mut().find(|d| d.version == dataset_version) {
        if existing_ds.manifest_digest == manifest_digest {
            if args.doi.is_some() {
                existing_ds.doi = args.doi.clone();
            }
        } else {
            existing_ds.manifest_digest = manifest_digest.clone();
            if args.doi.is_some() {
                existing_ds.doi = args.doi.clone();
            }
        }
    } else {
        rel.datasets.push(new_dataset.clone());
    }
    sort_datasets(&mut rel.datasets);

    index.releases.sort_by(|a, b| b.source.version.cmp(&a.source.version));

    index.validate()?;

    // An identical re-publish leaves the row as it was, so the row names the ods that first
    // recorded it, which may not be this one.
    let stored_dataset = index
        .dataset(&dataset_version)
        .map(|(_, d)| d.clone())
        .expect("the dataset row was just written");
    let short_sha = stored_dataset.tool_git_sha.get(..7).unwrap_or(&stored_dataset.tool_git_sha);
    eprintln!("* recorded as built by ods {} ({})", stored_dataset.tool_version, short_sha);

    if let Some(parent) = target_index_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let updated = index.to_json_pretty()?;
    fs::write(&target_index_path, &updated)?;
    eprintln!("✓ data/releases.json updated — review with `git diff data/releases.json`");

    // 7. Print the recorded dataset to stdout (compact 1-line JSON)
    println!("{}", serde_json::to_string(&stored_dataset)?);

    Ok(())
}

/// Reads the index file a release row will be written to, refusing, with a `✖` block, one this
/// ods can't read: unreadable, not an index, in the old format, or invalid.
fn read_index_file(p: &Path) -> Result<OdsReleaseIndex> {
    let bytes = match fs::read(p) {
        Ok(bytes) => bytes,
        Err(e) => {
            let err_str = e.to_string();
            let clean_err = err_str.split(" (os error").next().unwrap_or(&err_str);
            eprintln!("✖ Cannot read release index '{}': {}", p.display(), clean_err);
            return Err(crate::commands::pull::AlreadyReported.into());
        }
    };
    let idx = match OdsReleaseIndex::from_slice(&bytes) {
        Ok(i) => i,
        Err(e) if e.downcast_ref::<crate::index::OldFormatIndex>().is_some() => {
            eprintln!("{}", crate::commands::pull::format_old_format_index_flag(&p.display().to_string()));
            return Err(crate::commands::pull::AlreadyReported.into());
        }
        Err(_) => {
            eprintln!("✖ Cannot parse release index '{}' as an ODS release index", p.display());
            eprintln!("  Expected $schema {}", RELEASES_SCHEMA_V1_URL);
            return Err(crate::commands::pull::AlreadyReported.into());
        }
    };
    if let Err(e) = idx.validate() {
        eprintln!("✖ Cannot parse release index '{}' as an ODS release index", p.display());
        eprintln!("  {}", e);
        return Err(crate::commands::pull::AlreadyReported.into());
    }
    Ok(idx)
}

/// The release row for the source release `facts` name, before any dataset is recorded on it.
fn release_row_for(facts: &ReleaseFacts) -> Release {
    Release {
        source: SourceRelease {
            version: facts.release_date.clone(),
            hash: facts.source.hash.clone(),
            bytes: facts.source.bytes,
            issues: Vec::new(),
        },
        datasets: Vec::new(),
    }
}

pub fn perform_all_release_checks(
    release_dir: &Path,
    expected_version: &str,
    tool_repo: Option<&Path>,
    custom_index: Option<&OdsReleaseIndex>,
    build: &BuildIdentity,
) -> Result<Vec<String>> {
    let mut failures = Vec::new();

    // The facts these checks validate (the archive's size, the dataset version, the date and
    // SHA-256 the release row gets minted from) come from the object the Parquet files carry.
    let facts: ReleaseFacts = match crate::provenance::read_release(release_dir) {
        Ok(ReleaseRecord::Provenanced(facts)) => *facts,
        Ok(other) => {
            return Ok(vec![crate::provenance::format_record_refusal(release_dir, &other)
                .unwrap_or_default()
                .trim_start_matches("✖ ")
                .to_string()])
        }
        Err(e) => return Ok(vec![format!("{:#}", e).trim_start_matches("✖ ").to_string()]),
    };

    let tag_name = format!("v{}", build.tool_version);

    // Check 10: the recording ods was built from a clean tree
    if build.dirty {
        failures.push(format!(
            "this ods was built from a dirty working tree\n  A release records the ods that built it, so `cargo install --git … --tag {}` must reproduce it.",
            tag_name
        ));
    }

    if tool_repo.is_none() {
        failures.push("cannot locate the ods repository — data/releases.json is where a release row is reviewed\n  Pass --tool-repo, or run from inside the repo.".to_string());
    }

    let repo_dir = tool_repo.unwrap_or(release_dir);
    let repo_dir_str = repo_dir.to_string_lossy();

    // Check 11: it has a commit, and that commit is the one v<tool_version> points at
    match build.git_sha {
        None => {
            failures.push(format!(
                "this ods was built without a git commit, so it can't say which commit built the dataset\n  Data must be recorded by a tagged tool version, so `cargo install --git … --tag {}` reproduces it.",
                tag_name
            ));
        }
        Some(ref tool_sha) => {
            let output = Command::new("git")
                .args([
                    "-C",
                    &repo_dir_str,
                    "rev-parse",
                    "-q",
                    "--verify",
                    &format!("refs/tags/{}^{{commit}}", tag_name),
                ])
                .output();
            if let Ok(out) = output {
                if out.status.success() {
                    let tag_sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if !tag_sha.starts_with(tool_sha.as_str()) && !tool_sha.starts_with(tag_sha.as_str()) {
                        failures.push(format!(
                            "this ods is commit {}, not the commit {} points at ({})\n  Data must be recorded by a tagged tool version, so `cargo install --git … --tag {}` reproduces it.",
                            tool_sha, tag_name, tag_sha, tag_name
                        ));
                    }
                } else {
                    failures.push(format!(
                        "this ods is commit {}, not the commit {} points at (tag missing)\n  Data must be recorded by a tagged tool version, so `cargo install --git … --tag {}` reproduces it.",
                        tool_sha, tag_name, tag_name
                    ));
                }
            }
        }
    }

    // Check 12: the archive's size is plausible, so a synthetic or fixture zip is never
    // recorded as a real release. Real archives are about 38 MB. This reads a source fact from
    // the files; it asks nobody whether TRUD published the archive.
    if facts.source.bytes < 1_000_000 {
        failures.push(format!(
            "the source archive's size is implausibly small ({} bytes, must be >= 1 MB)",
            facts.source.bytes
        ));
    }

    // Check 13: the files' dataset version matches this build's constant and parses as semver.
    let ver = &facts.dataset_version;
    let tool_dataset_ver = crate::datapackage::DATASET_VERSION;
    if ver != tool_dataset_ver {
        failures.push(format!(
            "the Parquet files' dataset version ({}) does not match this build of ods ({})\n  The release was compiled by an older tool. Re-run `ods make`, or check out the\n  tool version that built it.",
            ver, tool_dataset_ver
        ));
    }
    if let Err(e) = parse_semver(ver) {
        failures.push(format!("Dataset version {} is not valid SemVer: {}", ver, e));
    }

    // Index checks (Steps 1, 2, 3)
    let previous_index = if let Some(idx) = custom_index {
        idx.clone()
    } else if let Some(tr) = tool_repo {
        let p = tr.join("data").join("releases.json");
        if p.exists() {
            match fs::read(&p) {
                Ok(bytes) => match OdsReleaseIndex::from_slice(&bytes) {
                    Ok(parsed) => parsed,
                    Err(e) => {
                        failures.push(format!(
                            "Cannot parse release index '{}' as an ODS release index\n  Expected $schema {}\n  {}",
                            p.display(),
                            crate::index::RELEASES_SCHEMA_V1_URL,
                            e
                        ));
                        return Ok(failures);
                    }
                },
                Err(e) => {
                    failures.push(format!("Cannot read release index '{}': {}", p.display(), e));
                    return Ok(failures);
                }
            }
        } else {
            crate::index::OdsReleaseIndex::baked()?
        }
    } else {
        crate::index::OdsReleaseIndex::baked()?
    };

    let mut candidate_index = previous_index.clone();

    let date = facts.release_date.as_str();
    // The dataset row's version: `<source version>_<dataset version>`.
    let dataset_version = format!("{}_{}", date, expected_version);

    // Guarantee the built release's row from the files' source before the checks
    if !date.is_empty() && candidate_index.release(date).is_none() {
        candidate_index.releases.push(release_row_for(&facts));
    }

    // Check this release's source: the row this release will land on holds the archive's own hash
    if let Some(rel_row) = candidate_index.release(date) {
        if rel_row.source.hash != facts.source.hash {
            failures.push(format!(
                "The release row's source.hash ({}) does not match the Parquet files' source hash ({})",
                rel_row.source.hash, facts.source.hash
            ));
        }
    }

    // Check 14: the append-only rule, via previous.merge(&candidate)
    let (candidate_manifest_digest, candidate_filesize_bytes) =
        match crate::oci::dataset::build(release_dir, &facts) {
            Ok((m, _)) => match m.digest() {
                Ok(d) => (d, m.layers.iter().map(|l| l.size).sum::<u64>()),
                Err(e) => {
                    failures.push(format!("Cannot compute candidate manifest digest: {}", e));
                    return Ok(failures);
                }
            },
            Err(e) => {
                failures.push(format!("Cannot build candidate manifest: {}", e));
                return Ok(failures);
            }
        };

    if let Some(rel_row) = candidate_index.releases.iter_mut().find(|r| r.source.version == date) {
        if let Some(existing_ds) = rel_row.datasets.iter_mut().find(|d| d.version == dataset_version) {
            existing_ds.manifest_digest = candidate_manifest_digest;
            existing_ds.bytes = candidate_filesize_bytes;
        } else {
            rel_row.datasets.push(Dataset {
                version: dataset_version.clone(),
                manifest_digest: candidate_manifest_digest,
                bytes: candidate_filesize_bytes,
                tool_version: build.tool_version.clone(),
                tool_git_sha: build.git_sha.clone().unwrap_or_default().to_lowercase(),
                doi: None,
                withdrawn: None,
            });
            sort_datasets(&mut rel_row.datasets);
        }
    }

    candidate_index.releases.sort_by(|a, b| b.source.version.cmp(&a.source.version));

    // Validate the candidate index inside perform_all_release_checks
    if failures.is_empty() {
        if let Err(e) = candidate_index.validate() {
            failures.push(format!("Candidate release index fails validation: {}", e));
        }
    }

    // Append-only check: previous.merge(&candidate) ensures no contradictions to existing data
    if failures.is_empty() {
        if let Err(e) = previous_index.merge(&candidate_index) {
            failures.push(e.to_string());
        }
    }

    Ok(failures)
}

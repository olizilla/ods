use anyhow::{bail, Context, Result};
use clap::Parser;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::index::{parse_semver, Dataset, OdsReleaseIndex, Release, RELEASES_SCHEMA_V1_URL};
use crate::oci::*;
use crate::provenance::OdsProvenance;

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

    let prov = match OdsProvenance::load_from_dir(&release_dir) {
        crate::provenance::ProvenanceLoad::Read(p, _) => *p,
        crate::provenance::ProvenanceLoad::Unreadable { path, date } => {
            bail!(
                "{}",
                crate::provenance::format_unreadable_provenance_error(&path, &date)
            );
        }
        crate::provenance::ProvenanceLoad::Absent => {
            bail!(
                "{}",
                crate::provenance::format_no_provenance_error(&release_dir)
            );
        }
    };

    let version = crate::datapackage::read_dataset_version_from_dir(&release_dir).ok_or_else(|| {
        anyhow::anyhow!("Missing version in datapackage.json\n  Run `ods make` to build the release directory.")
    })?;

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
        match fs::read(p) {
            Ok(bytes) => {
                let idx: OdsReleaseIndex = match serde_json::from_slice(&bytes) {
                    Ok(i) => i,
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
                (Some(idx), p.clone())
            }
            Err(e) => {
                let err_str = e.to_string();
                let clean_err = err_str.split(" (os error").next().unwrap_or(&err_str);
                eprintln!("✖ Cannot read release index '{}': {}", p.display(), clean_err);
                return Err(crate::commands::pull::AlreadyReported.into());
            }
        }
    } else if let Some(ref tr) = tool_repo {
        let p = tr.join("data").join("releases.json");
        if p.exists() {
            match fs::read(&p) {
                Ok(bytes) => {
                    let idx: OdsReleaseIndex = match serde_json::from_slice(&bytes) {
                        Ok(i) => i,
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
                    (Some(idx), p)
                }
                Err(e) => {
                    let err_str = e.to_string();
                    let clean_err = err_str.split(" (os error").next().unwrap_or(&err_str);
                    eprintln!("✖ Cannot read release index '{}': {}", p.display(), clean_err);
                    return Err(crate::commands::pull::AlreadyReported.into());
                }
            }
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

    let date = prov.trud_release_date.as_deref().unwrap_or("unknown");

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
        let content = fs::read_to_string(&target_index_path)?;
        serde_json::from_str(&content)?
    } else {
        OdsReleaseIndex::baked()?
    };

    // Guarantee the built date's row from _provenance.json before reading TRUD response
    if !index.releases.iter().any(|r| r.trud_release_date == date) {
        index.releases.push(Release {
            trud_release_date: date.to_string(),
            trud_release_sha256: prov.trud_release_sha256.clone().unwrap_or_default().to_uppercase(),
            trud_release_filesize_bytes: prov.trud_release_filesize_bytes.unwrap_or_default(),
            datasets: Vec::new(),
        });
    }

    // Add the dataset (or preserve the existing row on an identical re-publish)
    let new_dataset = Dataset {
        dataset_version: version.clone(),
        manifest_digest: manifest_digest.clone(),
        dataset_filesize_bytes: total_bytes,
        tool_version: build.tool_version.clone(),
        tool_git_sha: build.git_sha.clone().unwrap_or_default().to_lowercase(),
        dataset_doi: args.doi.clone(),
        withdrawn: None,
    };
    let rel = index
        .releases
        .iter_mut()
        .find(|r| r.trud_release_date == date)
        .expect("Release row for date must exist");
    if let Some(existing_ds) = rel.datasets.iter_mut().find(|d| d.dataset_version == version) {
        if existing_ds.manifest_digest == manifest_digest {
            if args.doi.is_some() {
                existing_ds.dataset_doi = args.doi.clone();
            }
        } else {
            existing_ds.manifest_digest = manifest_digest.clone();
            if args.doi.is_some() {
                existing_ds.dataset_doi = args.doi.clone();
            }
        }
    } else {
        rel.datasets.push(new_dataset.clone());
    }
    rel.datasets.sort_by(|a, b| {
        let va = parse_semver(&a.dataset_version).unwrap_or((0, 0, 0));
        let vb = parse_semver(&b.dataset_version).unwrap_or((0, 0, 0));
        va.cmp(&vb)
    });

    index.releases.sort_by(|a, b| b.trud_release_date.cmp(&a.trud_release_date));

    index.validate()?;

    // An identical re-publish leaves the row as it was, so the row names the ods that first
    // recorded it, which may not be this one.
    let stored_dataset = index
        .releases
        .iter()
        .find(|r| r.trud_release_date == date)
        .and_then(|r| r.datasets.iter().find(|d| d.dataset_version == version))
        .cloned()
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

pub fn perform_all_release_checks(
    release_dir: &Path,
    expected_version: &str,
    tool_repo: Option<&Path>,
    custom_index: Option<&OdsReleaseIndex>,
    build: &BuildIdentity,
) -> Result<Vec<String>> {
    let mut failures = Vec::new();

    let prov = match OdsProvenance::load_from_dir(release_dir).error_building()? {
        Some(p) => p,
        None => return Ok(vec!["Missing _provenance.json in release directory".to_string()]),
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

    // Check 12: the archive's size is present and plausible, so a synthetic or fixture zip is
    // never recorded as a real release. Real archives are about 38 MB. This reads a source
    // fact from provenance; it asks nobody whether TRUD published the archive.
    match prov.trud_release_filesize_bytes {
        Some(sz) if sz < 1_000_000 => {
            failures.push(format!(
                "trud_release_filesize_bytes is implausibly small ({} bytes, must be >= 1 MB)",
                sz
            ));
        }
        None => {
            failures.push("Missing trud_release_filesize_bytes in _provenance.json".to_string());
        }
        _ => {}
    }

    // Check 13: datapackage.json version matches built-in constant and parses as semver
    let dp_ver = crate::datapackage::read_dataset_version_from_dir(release_dir);
    if let Some(ref ver) = dp_ver {
        let tool_dataset_ver = crate::datapackage::DATASET_VERSION;
        if ver != tool_dataset_ver {
            failures.push(format!(
                "datapackage.json version ({}) does not match this build of ods ({})\n  The release was compiled by an older tool. Re-run `ods make`, or check out the\n  tool version that built it.",
                ver, tool_dataset_ver
            ));
        }
        if let Err(e) = parse_semver(ver) {
            failures.push(format!("Dataset version {} is not valid SemVer: {}", ver, e));
        }
    } else {
        failures.push("datapackage.json missing version".to_string());
    }

    // Index checks (Steps 1, 2, 3)
    let previous_index = if let Some(idx) = custom_index {
        idx.clone()
    } else if let Some(tr) = tool_repo {
        let p = tr.join("data").join("releases.json");
        if p.exists() {
            match fs::read(&p) {
                Ok(bytes) => match serde_json::from_slice::<OdsReleaseIndex>(&bytes) {
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

    let date = prov.trud_release_date.as_deref().unwrap_or("");
    let prov_sha = prov.trud_release_sha256.as_deref().unwrap_or("").to_uppercase();
    let prov_size = prov.trud_release_filesize_bytes.unwrap_or(0);

    // Guarantee the built date's row from _provenance.json before the checks
    if !date.is_empty() && !candidate_index.releases.iter().any(|r| r.trud_release_date == date) {
        candidate_index.releases.push(Release {
            trud_release_date: date.to_string(),
            trud_release_sha256: prov_sha.clone(),
            trud_release_filesize_bytes: prov_size,
            datasets: Vec::new(),
        });
    }

    // Check this release's source: the row this release will land on holds the archive's own hash
    if let Some(rel_row) = candidate_index.releases.iter().find(|r| r.trud_release_date == date) {
        if !prov_sha.is_empty() && rel_row.trud_release_sha256.to_uppercase() != prov_sha {
            failures.push(format!(
                "The release row's trud_release_sha256 ({}) does not match _provenance.json ({})",
                rel_row.trud_release_sha256, prov_sha
            ));
        }
    }

    // Check 14: the append-only rule, via previous.merge(&candidate)
    let (candidate_manifest_digest, candidate_filesize_bytes) =
        match crate::commands::make_oci::build_manifest_from_dir(release_dir, &prov, expected_version) {
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

    if let Some(rel_row) = candidate_index.releases.iter_mut().find(|r| r.trud_release_date == date) {
        if let Some(existing_ds) = rel_row.datasets.iter_mut().find(|d| d.dataset_version == expected_version) {
            existing_ds.manifest_digest = candidate_manifest_digest;
            existing_ds.dataset_filesize_bytes = candidate_filesize_bytes;
        } else {
            rel_row.datasets.push(Dataset {
                dataset_version: expected_version.to_string(),
                manifest_digest: candidate_manifest_digest,
                dataset_filesize_bytes: candidate_filesize_bytes,
                tool_version: build.tool_version.clone(),
                tool_git_sha: build.git_sha.clone().unwrap_or_default().to_lowercase(),
                dataset_doi: None,
                withdrawn: None,
            });
            rel_row.datasets.sort_by(|a, b| {
                let va = parse_semver(&a.dataset_version).unwrap_or((0, 0, 0));
                let vb = parse_semver(&b.dataset_version).unwrap_or((0, 0, 0));
                va.cmp(&vb)
            });
        }
    }

    candidate_index.releases.sort_by(|a, b| b.trud_release_date.cmp(&a.trud_release_date));

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

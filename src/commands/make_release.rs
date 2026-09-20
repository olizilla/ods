use anyhow::{bail, Result};
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

    /// OCI repository name for staging keys (defaults to ods-data)
    #[arg(long, default_value = "ods-data")]
    pub repository: String,

    /// Output path for the staging directory (defaults to dist/)
    #[arg(long, short)]
    pub output: Option<PathBuf>,

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

fn copy_blob(src: &Path, dst: &Path) -> Result<()> {
    if let Some(parent) = dst.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            let err_str = e.to_string();
            let clean_err = err_str.split(" (os error").next().unwrap_or(&err_str);
            eprintln!("✖ Cannot stage blob {} → {}: {}", src.display(), dst.display(), clean_err);
            bail!("Cannot stage blob {} → {}: {}", src.display(), dst.display(), clean_err);
        }
    }
    if dst.exists() {
        let _ = fs::remove_file(dst);
    }
    if let Err(e) = fs::copy(src, dst) {
        let err_str = e.to_string();
        let clean_err = err_str.split(" (os error").next().unwrap_or(&err_str);
        eprintln!("✖ Cannot stage blob {} → {}: {}", src.display(), dst.display(), clean_err);
        bail!("Cannot stage blob {} → {}: {}", src.display(), dst.display(), clean_err);
    }
    Ok(())
}

fn count_files_in_dir(dir: &Path) -> usize {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .count()
}

pub fn run(args: Args) -> Result<()> {
    let release_dir = match args.input {
        Some(ref p) => p.clone(),
        None => {
            bail!("✖ --input is required for 'ods make release'\n  Pass the release directory to publish, e.g. ods make release --input <release-dir>");
        }
    };

    if !release_dir.exists() {
        bail!("Release directory does not exist: {}", release_dir.display());
    }

    let prov = OdsProvenance::load_from_dir(&release_dir)
        .error_building()?
        .ok_or_else(|| anyhow::anyhow!("Missing _provenance.json in {}", release_dir.display()))?;

    let version = prov.dataset_version.clone().ok_or_else(|| {
        anyhow::anyhow!("Missing dataset_version in _provenance.json\n  Run `ods make` to build the release directory.")
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
        check: false,
    })?;

    // 4. Read back manifest from oci/
    let oci_dir = release_dir.join("oci");
    let blobs_dir = oci_dir.join("blobs").join("sha256");
    let manifest_path = fs::read_dir(&blobs_dir)?
        .flatten()
        .find(|e| e.path().is_file() && !e.path().is_symlink())
        .ok_or_else(|| anyhow::anyhow!("No manifest blob found in oci/blobs/sha256/"))?
        .path();
    let manifest_bytes = fs::read(&manifest_path)?;
    let manifest: OciManifest = serde_json::from_slice(&manifest_bytes)?;
    let manifest_digest = manifest.digest()?;

    let date = prov.trud_release_date.as_deref().unwrap_or("unknown");
    let versioned_tag = format!("{}_{}", date, version);

    // Summary lines on progress
    let total_bytes: u64 = manifest.layers.iter().map(|l| l.size).sum();
    let mb = (total_bytes as f64) / (1024.0 * 1024.0);
    eprintln!("* {} layers, {:.1} MB, manifest {}", manifest.layers.len(), mb, manifest_digest);

    let tool_ver = prov.tool_version.as_deref().unwrap_or(env!("CARGO_PKG_VERSION"));
    if let Some(ref tool_sha) = prov.tool_git_sha {
        let short_sha = if tool_sha.len() >= 7 { &tool_sha[..7] } else { tool_sha };
        eprintln!("* tool_git_sha {} matches v{}", short_sha, tool_ver);
    }

    let ver_source = match prov.trud_release_sha256_verified {
        Some(crate::provenance::TrudVerificationSource::TrudApi) => "trud_api",
        Some(crate::provenance::TrudVerificationSource::PublishedRelease) => "published_release",
        _ => "unverified",
    };
    eprintln!("* trud_release_sha256 verified via {}", ver_source);

    // 5. Generate dist/ staging directory (holds only release objects, no releases.json)
    let dist_dir = args.output.unwrap_or_else(|| {
        tool_repo
            .as_ref()
            .map(|tr| tr.join("dist"))
            .unwrap_or_else(|| PathBuf::from("dist"))
    });

    if dist_dir.exists() {
        fs::remove_dir_all(&dist_dir)?;
    }
    fs::create_dir_all(&dist_dir)?;

    // 5a. dist/v2/{repository}/blobs/sha256/{hex} (for every blob in oci/blobs/sha256/)
    let repo_name = &args.repository;
    for entry in fs::read_dir(&blobs_dir)? {
        let entry = entry?;
        let p = entry.path();
        let file_name = entry.file_name();
        let dst_blob = dist_dir
            .join("v2")
            .join(repo_name)
            .join("blobs")
            .join("sha256")
            .join(&file_name);

        let canonical_source = if p.is_symlink() {
            let target = fs::read_link(&p)?;
            if target.is_relative() {
                p.parent().unwrap().join(target)
            } else {
                target
            }
        } else {
            p.clone()
        };
        copy_blob(&canonical_source, &dst_blob)?;
    }

    // 5c. Manifest tags: versioned, bare date, latest
    let manifests_dir = dist_dir.join("v2").join(repo_name).join("manifests");
    fs::create_dir_all(&manifests_dir)?;
    fs::write(manifests_dir.join(&versioned_tag), &manifest_bytes)?;
    fs::write(manifests_dir.join(date), &manifest_bytes)?;
    fs::write(manifests_dir.join("latest"), &manifest_bytes)?;

    let object_count = count_files_in_dir(&dist_dir);
    eprintln!("✓ dist/ written, {} objects", object_count);

    // 6. Update data/releases.json (strictly after staging succeeds)
    let mut index: OdsReleaseIndex = if target_index_path.exists() {
        let content = fs::read_to_string(&target_index_path)?;
        serde_json::from_str(&content)?
    } else {
        OdsReleaseIndex::default()
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

    // Step 1: Record TRUD's releases
    let trud_resp_path = release_dir.join("trud").join(format!("trud-releases-{}.json", date));
    if trud_resp_path.exists() {
        let bytes = fs::read(&trud_resp_path)?;
        let resp: crate::commands::fetch::TrudApiResponse = serde_json::from_slice(&bytes)?;
        for item in resp.releases {
            if !index.releases.iter().any(|r| r.trud_release_date == item.release_date) {
                index.releases.push(Release {
                    trud_release_date: item.release_date,
                    trud_release_sha256: item.archive_file_sha256.to_uppercase(),
                    trud_release_filesize_bytes: item.archive_file_size,
                    datasets: Vec::new(),
                });
            }
        }
    }

    // Step 3: Add dataset
    let new_dataset = Dataset {
        dataset_version: version.clone(),
        manifest_digest: manifest_digest.clone(),
        dataset_doi: args.doi.clone(),
        withdrawn: None,
    };
    let rel = index
        .releases
        .iter_mut()
        .find(|r| r.trud_release_date == date)
        .expect("Release row for date must exist");
    rel.datasets.push(new_dataset.clone());
    rel.datasets.sort_by(|a, b| {
        let va = parse_semver(&a.dataset_version).unwrap_or((0, 0, 0));
        let vb = parse_semver(&b.dataset_version).unwrap_or((0, 0, 0));
        va.cmp(&vb)
    });

    index.releases.sort_by(|a, b| b.trud_release_date.cmp(&a.trud_release_date));

    index.validate()?;

    if let Some(parent) = target_index_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let updated = index.to_json_pretty()?;
    fs::write(&target_index_path, &updated)?;
    eprintln!("✓ data/releases.json updated — review with `git diff data/releases.json`");

    // 7. Print new dataset to stdout (compact 1-line JSON)
    println!("{}", serde_json::to_string(&new_dataset)?);

    Ok(())
}

pub fn perform_all_release_checks(
    release_dir: &Path,
    expected_version: &str,
    tool_repo: Option<&Path>,
    custom_index: Option<&OdsReleaseIndex>,
) -> Result<Vec<String>> {
    let mut failures = Vec::new();

    let prov = match OdsProvenance::load_from_dir(release_dir).error_building()? {
        Some(p) => p,
        None => return Ok(vec!["Missing _provenance.json in release directory".to_string()]),
    };

    // Check 10: tool_git_dirty == false
    if prov.tool_git_dirty == Some(true) {
        failures.push("tool_git_dirty is true: dataset built from a dirty working tree".to_string());
    }

    if tool_repo.is_none() {
        failures.push("cannot locate the ods repository — data/releases.json is where a release row is reviewed\n  Pass --tool-repo, or run from inside the repo.".to_string());
    }

    let repo_dir = tool_repo.unwrap_or(release_dir);
    let repo_dir_str = repo_dir.to_string_lossy();

    // Check 11: tool_git_sha == commit v<tool_version> points at
    let tool_ver = prov.tool_version.as_deref().unwrap_or(env!("CARGO_PKG_VERSION"));
    if let Some(ref tool_sha) = prov.tool_git_sha {
        let tag_name = format!("v{}", tool_ver);
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
                        "tool_git_sha {} is not the commit {} points at ({})\n  Data must be built by a tagged tool version, so `cargo install --git … --tag {}` reproduces it.",
                        tool_sha, tag_name, tag_sha, tag_name
                    ));
                }
            } else {
                failures.push(format!(
                    "tool_git_sha {} is not the commit {} points at (tag missing)\n  Data must be built by a tagged tool version, so `cargo install --git … --tag {}` reproduces it.",
                    tool_sha, tag_name, tag_name
                ));
            }
        }
    }

    // Check 12: trud_release_sha256_verified == "trud_api" or "published_release" and plausible filesize (>= 1 MB)
    match prov.trud_release_sha256_verified {
        Some(crate::provenance::TrudVerificationSource::TrudApi)
        | Some(crate::provenance::TrudVerificationSource::PublishedRelease) => {}
        _ => {
            failures.push(
                "trud_release_sha256_verified is not trud_api: source was never verified against TRUD".to_string(),
            );
        }
    }

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

    // Check 13: dataset_version matches built-in constant and parses as semver
    if let Some(ref prov_ver) = prov.dataset_version {
        let tool_dataset_ver = crate::datapackage::dataset_version();
        if prov_ver != tool_dataset_ver {
            failures.push(format!(
                "Provenance dataset_version ({}) does not match this build of ods ({})\n  The release was compiled by an older tool. Re-run `ods make`, or check out the\n  tool version that built it.",
                prov_ver, tool_dataset_ver
            ));
        }
        if let Err(e) = parse_semver(expected_version) {
            failures.push(format!("Dataset version {} is not valid SemVer: {}", expected_version, e));
        }
    } else {
        failures.push("Provenance missing dataset_version".to_string());
    }

    // Index checks (Steps 1, 2, 3)
    let mut candidate_index = if let Some(idx) = custom_index {
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
            crate::index::OdsReleaseIndex::default()
        }
    } else {
        crate::index::OdsReleaseIndex::default()
    };

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

    let trud_resp_path = release_dir.join("trud").join(format!("trud-releases-{}.json", date));

    // Step 1: Record TRUD's releases
    if trud_resp_path.exists() {
        if let Ok(bytes) = fs::read(&trud_resp_path) {
            if let Ok(resp) = serde_json::from_slice::<crate::commands::fetch::TrudApiResponse>(&bytes) {
                for item in &resp.releases {
                    let item_sha = item.archive_file_sha256.to_uppercase();
                    if let Some(existing) = candidate_index.releases.iter().find(|r| r.trud_release_date == item.release_date) {
                        let existing_sha = existing.trud_release_sha256.to_uppercase();
                        if existing_sha != item_sha {
                            let ex_short = if existing_sha.len() >= 8 { &existing_sha[..8] } else { &existing_sha };
                            let item_short = if item_sha.len() >= 8 { &item_sha[..8] } else { &item_sha };
                            failures.push(format!(
                                "The index records TRUD release {} with SHA-256 {}…, but trud/trud-releases-{}.json says {}…\n  TRUD may have reissued it. Nothing was written.",
                                item.release_date, ex_short, date, item_short
                            ));
                        } else if existing.trud_release_filesize_bytes != item.archive_file_size {
                            failures.push(format!(
                                "The index records TRUD release {} with size {} bytes, but trud/trud-releases-{}.json says {} bytes\n  TRUD may have reissued it. Nothing was written.",
                                item.release_date, existing.trud_release_filesize_bytes, date, item.archive_file_size
                            ));
                        }
                    } else {
                        candidate_index.releases.push(Release {
                            trud_release_date: item.release_date.clone(),
                            trud_release_sha256: item_sha,
                            trud_release_filesize_bytes: item.archive_file_size,
                            datasets: Vec::new(),
                        });
                    }
                }
            }
        }
    }

    // Step 2: Check this release's source
    if let Some(rel_row) = candidate_index.releases.iter().find(|r| r.trud_release_date == date) {
        if !prov_sha.is_empty() && rel_row.trud_release_sha256.to_uppercase() != prov_sha {
            failures.push(format!(
                "The release row's trud_release_sha256 ({}) does not match _provenance.json ({})",
                rel_row.trud_release_sha256, prov_sha
            ));
        }
    }

    // Step 3 (Check 14): no dataset with this dataset_version on this date
    if let Some(rel_row) = candidate_index.releases.iter_mut().find(|r| r.trud_release_date == date) {
        if rel_row.datasets.iter().any(|d| d.dataset_version == expected_version) {
            failures.push(format!(
                "data/releases.json already has a dataset for {} {}\n  A published (date, version) pair names one set of bytes forever.\n  Bump dataset_version and re-pack.",
                date, expected_version
            ));
        } else {
            rel_row.datasets.push(Dataset {
                dataset_version: expected_version.to_string(),
                manifest_digest: "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_string(),
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

    Ok(failures)
}

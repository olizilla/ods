use anyhow::{bail, Result};
use clap::Parser;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::index::{parse_semver, MirrorEntry, OdsReleaseIndex, ReleaseIndexEntry};
use crate::oci::*;
use crate::provenance::OdsProvenance;

#[derive(Parser, Debug, Clone)]
pub struct Args {
    /// Path to compiled release directory (defaults to active release)
    #[arg(long, short)]
    pub input: Option<PathBuf>,

    /// Dataset semver for this release cut (e.g. 1.0.1; defaults to _provenance.json)
    #[arg(long, short)]
    pub version: Option<String>,

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

    /// Skip remote git network checks
    #[arg(long)]
    pub offline: bool,
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

fn link_or_copy(src: &Path, dst: &Path) -> Result<()> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    if dst.exists() {
        fs::remove_file(dst)?;
    }
    if fs::hard_link(src, dst).is_err() {
        fs::copy(src, dst)?;
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
            let root = crate::workspace::find_workspace_root(None)
                .ok_or_else(|| anyhow::anyhow!("No workspace found. Specify --input <release_dir>"))?;
            let (_, active_path) = crate::workspace::get_active_release(&root)?;
            active_path
        }
    };

    if !release_dir.exists() {
        bail!("Release directory does not exist: {}", release_dir.display());
    }

    let prov = OdsProvenance::load_from_dir(&release_dir)
        .ok_or_else(|| anyhow::anyhow!("Missing _provenance.json in {}", release_dir.display()))?;

    let version = match args.version {
        Some(v) => v,
        None => prov.dataset_version.clone().ok_or_else(|| {
            anyhow::anyhow!("Missing dataset_version in _provenance.json. Pass --version <semver>")
        })?,
    };

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

    // 1. Run ods make oci first to regenerate oci/ wholesale
    crate::commands::make_oci::run(crate::commands::make_oci::Args {
        input: Some(release_dir.clone()),
        version: Some(version.clone()),
        check: false,
    })?;

    // 2. Read back manifest from oci/
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

    let custom_index = if let Some(ref p) = args.index {
        let content = fs::read_to_string(p)?;
        Some(serde_json::from_str::<OdsReleaseIndex>(&content)?)
    } else if let Some(ref tr) = tool_repo {
        let p = tr.join("data").join("releases.json");
        if p.exists() {
            let content = fs::read_to_string(&p)?;
            Some(serde_json::from_str::<OdsReleaseIndex>(&content)?)
        } else {
            None
        }
    } else {
        None
    };

    // 3. Run Checks 10-15
    let failures = perform_all_release_checks(
        &release_dir,
        &version,
        tool_repo.as_deref(),
        custom_index.as_ref(),
        args.offline,
    )?;

    if !failures.is_empty() {
        for failure in &failures {
            eprintln!("✖ {}", failure);
        }
        let count = failures.len();
        eprintln!("{} check{} failed", count, if count == 1 { "" } else { "s" });
        bail!("Release checks failed:\n{}", failures.join("\n"));
    }

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
    eprintln!("* data/{} free locally and on origin", versioned_tag);

    let release_row = ReleaseIndexEntry {
        trud_release_date: date.to_string(),
        dataset_version: version.clone(),
        tag: versioned_tag.clone(),
        manifest_digest: manifest_digest.clone(),
        trud_release_sha256: prov.trud_release_sha256.clone().unwrap_or_default().to_uppercase(),
        tool_version: prov.tool_version.clone().unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string()),
        dataset_doi: args.doi.clone(),
        withdrawn: None,
    };

    // 4. Append to data/releases.json
    let target_index_path = args.index.clone().or_else(|| {
        tool_repo.as_ref().map(|tr| tr.join("data").join("releases.json"))
    }).ok_or_else(|| anyhow::anyhow!("cannot locate the ods repository — data/releases.json is where a release row is reviewed\n  Pass --tool-repo, or run from inside the repo."))?;

    if target_index_path.exists() {
        let content = fs::read_to_string(&target_index_path)?;
        let mut index: OdsReleaseIndex = serde_json::from_str(&content)?;
        index.releases.push(release_row.clone());
        let updated = serde_json::to_string_pretty(&index)? + "\n";
        fs::write(&target_index_path, &updated)?;
        eprintln!("✓ data/releases.json updated — review with `git diff data/releases.json`");
    } else {
        let default_mirrors = OdsReleaseIndex::baked()
            .map(|b| b.mirrors)
            .unwrap_or_else(|_| vec![
                MirrorEntry {
                    url: "https://ods.fyi/v2/ods-data".to_string(),
                },
                MirrorEntry {
                    url: "https://ghcr.io/v2/olizilla/ods-data".to_string(),
                },
            ]);
        let index = OdsReleaseIndex {
            type_tag: "ods_release_index".to_string(),
            index_version: 2,
            concept_doi: None,
            mirrors: default_mirrors,
            releases: vec![release_row.clone()],
        };
        if let Some(parent) = target_index_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let updated = serde_json::to_string_pretty(&index)? + "\n";
        fs::write(&target_index_path, &updated)?;
        eprintln!("✓ data/releases.json updated — review with `git diff data/releases.json`");
    }

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

        let real_source = if p.is_symlink() {
            fs::read_link(&p)?
        } else {
            p.clone()
        };
        let canonical_source = if real_source.is_relative() {
            p.parent().unwrap().join(real_source)
        } else {
            real_source
        };
        link_or_copy(&canonical_source, &dst_blob)?;
    }

    // 5c. Manifest tags: versioned, bare date, latest
    let manifests_dir = dist_dir.join("v2").join(repo_name).join("manifests");
    fs::create_dir_all(&manifests_dir)?;
    fs::write(manifests_dir.join(&versioned_tag), &manifest_bytes)?;
    fs::write(manifests_dir.join(date), &manifest_bytes)?;
    fs::write(manifests_dir.join("latest"), &manifest_bytes)?;

    // 5d. Layer files by title: {date}/{version}/{title} and latest/{title}
    for layer in &manifest.layers {
        if let Some(ref ann) = layer.annotations {
            if let Some(title) = ann.get(ANNOTATION_TITLE) {
                let src_layer = release_dir.join(title);
                if src_layer.exists() {
                    let dst_versioned = dist_dir.join(date).join(&version).join(title);
                    let dst_latest = dist_dir.join("latest").join(title);
                    link_or_copy(&src_layer, &dst_versioned)?;
                    link_or_copy(&src_layer, &dst_latest)?;
                }
            }
        }
    }

    let object_count = count_files_in_dir(&dist_dir);
    eprintln!("✓ dist/ written, {} objects", object_count);

    // 6. Print new row to stdout (compact 1-line JSON)
    println!("{}", serde_json::to_string(&release_row)?);

    Ok(())
}

pub fn perform_all_release_checks(
    release_dir: &Path,
    expected_version: &str,
    tool_repo: Option<&Path>,
    custom_index: Option<&OdsReleaseIndex>,
    offline: bool,
) -> Result<Vec<String>> {
    let mut failures = Vec::new();

    let prov = match OdsProvenance::load_from_dir(release_dir) {
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

    // Check 13: dataset_version matches expected_version and parses as semver
    if let Some(ref prov_ver) = prov.dataset_version {
        if prov_ver != expected_version {
            failures.push(format!(
                "Provenance dataset_version ({}) != expected version ({})",
                prov_ver, expected_version
            ));
        }
        if let Err(e) = parse_semver(expected_version) {
            failures.push(format!("Dataset version {} is not valid SemVer: {}", expected_version, e));
        }
    } else {
        failures.push("Provenance missing dataset_version".to_string());
    }

    // Check 14: data/releases.json has no row for this (date, version)
    let index_opt = if let Some(idx) = custom_index {
        Some(idx.clone())
    } else {
        crate::index::OdsReleaseIndex::baked().ok()
    };

    if let Some(ref index) = index_opt {
        let date = prov.trud_release_date.as_deref().unwrap_or("");
        if index
            .releases
            .iter()
            .any(|r| r.trud_release_date == date && r.dataset_version == expected_version)
        {
            failures.push(format!(
                "data/releases.json already has a row for {} {}\n  A published (date, version) pair names one set of bytes forever.\n  Bump dataset_version and re-pack.",
                date, expected_version
            ));
        }
    }

    // Check 15: git tag data/<date>_<version> free locally and on origin
    let date = prov.trud_release_date.as_deref().unwrap_or("");
    let git_tag = format!("data/{}_{}", date, expected_version);
    let local_tag = Command::new("git")
        .args([
            "-C",
            &repo_dir_str,
            "rev-parse",
            "-q",
            "--verify",
            &format!("refs/tags/{}", git_tag),
        ])
        .output();
    if let Ok(out) = local_tag {
        if out.status.success() {
            failures.push(format!("git tag {} already exists locally", git_tag));
        }
    }

    let is_offline = offline || std::env::var("ODS_OFFLINE").is_ok();
    if !is_offline {
        let has_origin = Command::new("git")
            .args(["-C", &repo_dir_str, "remote", "get-url", "origin"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if has_origin {
            let origin_tag = Command::new("git")
                .args([
                    "-C",
                    &repo_dir_str,
                    "-c",
                    "http.lowSpeedLimit=1000",
                    "-c",
                    "http.lowSpeedTime=2",
                    "ls-remote",
                    "--exit-code",
                    "--tags",
                    "origin",
                    &git_tag,
                ])
                .output();
            if let Ok(out) = origin_tag {
                if out.status.success() {
                    failures.push(format!("git tag {} exists on origin", git_tag));
                }
            }
        }
    }

    Ok(failures)
}

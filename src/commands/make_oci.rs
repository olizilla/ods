use anyhow::{bail, Context, Result};
use clap::Parser;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::index::{parse_semver, OdsReleaseIndex, ReleaseIndexEntry};
use crate::oci::*;
use crate::provenance::{compute_file_sha256, OdsProvenance, PROVENANCE_FILENAME};

#[derive(Parser, Debug, Clone)]
pub struct Args {
    /// Path to compiled release directory
    #[arg(long, short)]
    pub input: PathBuf,

    /// Dataset semver for this release cut (e.g. 1.0.1)
    #[arg(long, short)]
    pub version: String,

    /// Verify existing OCI layout without rebuilding
    #[arg(long)]
    pub check: bool,

    /// Skip remote git network checks
    #[arg(long)]
    pub offline: bool,

    /// Path to git repository containing ods tool (defaults to auto-detection)
    #[arg(long)]
    pub tool_repo: Option<PathBuf>,

    /// Path to release index file (defaults to data/releases.json or baked index)
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
    if let Ok(dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let p = PathBuf::from(dir);
        if is_tool_repo_dir(&p) {
            return Some(p);
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
    let mut cur = release_dir.to_path_buf();
    loop {
        if is_tool_repo_dir(&cur) {
            return Some(cur);
        }
        if !cur.pop() {
            break;
        }
    }
    None
}

/// Builds an OCI manifest directly by scanning the release directory files and computing digests.
pub fn build_manifest_from_dir(
    release_dir: &Path,
    prov: &OdsProvenance,
    version: &str,
) -> Result<(OciManifest, Vec<u8>)> {
    let mut publishable_files = Vec::new();
    let mut file_hashes = HashMap::new();
    let mut file_sizes = HashMap::new();

    let entries = fs::read_dir(release_dir)?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with('.') || name == "_release.json" {
                    continue;
                }
                publishable_files.push(name.to_string());
                let meta = fs::metadata(&path)?;
                let hash = compute_file_sha256(&path)?;
                file_sizes.insert(name.to_string(), meta.len());
                file_hashes.insert(name.to_string(), hash);
            }
        }
    }
    publishable_files.sort();

    let prov_hash = file_hashes
        .get(PROVENANCE_FILENAME)
        .ok_or_else(|| anyhow::anyhow!("Missing _provenance.json"))?
        .to_lowercase();
    let prov_size = file_sizes[PROVENANCE_FILENAME];
    let config = OciDescriptor::new(
        MEDIA_TYPE_PROVENANCE,
        &format!("sha256:{}", prov_hash),
        prov_size,
    );

    let mut layers = Vec::new();
    for fname in &publishable_files {
        let fhash = file_hashes[fname].to_lowercase();
        let fsize = file_sizes[fname];
        let media_type = media_type_for_file(fname);
        layers.push(OciDescriptor::new(media_type, &format!("sha256:{}", fhash), fsize).with_title(fname));
    }

    let mut annotations = BTreeMap::new();
    if let Some(ref pub_date) = prov.publication_date {
        annotations.insert(
            ANNOTATION_CREATED.to_string(),
            derive_created_timestamp(pub_date),
        );
    }
    annotations.insert(
        ANNOTATION_LICENSES.to_string(),
        "OGL-UK-3.0".to_string(),
    );
    annotations.insert(
        ANNOTATION_SOURCE.to_string(),
        "https://github.com/olizilla/ods".to_string(),
    );
    annotations.insert(ANNOTATION_VERSION.to_string(), version.to_string());
    annotations.insert(ANNOTATION_FYI_DATASET_VERSION.to_string(), version.to_string());
    if let Some(ref d) = prov.trud_release_date {
        annotations.insert(ANNOTATION_FYI_TRUD_RELEASE_DATE.to_string(), d.clone());
    }
    if let Some(ref trud_sha) = prov.trud_release_sha256 {
        annotations.insert(ANNOTATION_FYI_TRUD_RELEASE_SHA256.to_string(), trud_sha.to_uppercase());
    }
    if let Some(ref tool_ver) = prov.tool_version {
        annotations.insert(ANNOTATION_FYI_TOOL_VERSION.to_string(), tool_ver.clone());
    }
    if let Some(ref tool_sha) = prov.tool_git_sha {
        annotations.insert(ANNOTATION_FYI_TOOL_GIT_SHA.to_string(), tool_sha.clone());
    }

    let mut manifest = OciManifest {
        schema_version: 2,
        media_type: MEDIA_TYPE_MANIFEST.to_string(),
        artifact_type: ARTIFACT_TYPE_DATASET.to_string(),
        config,
        layers,
        annotations: Some(annotations),
    };
    manifest.sort_layers_by_title();

    let bytes = manifest.to_canonical_bytes()?;
    Ok((manifest, bytes))
}

pub fn run(args: Args) -> Result<()> {
    let release_dir = &args.input;
    if !release_dir.is_dir() {
        bail!("Input release directory '{}' does not exist", release_dir.display());
    }

    // Parse and validate version format early
    parse_semver(&args.version)?;

    let mut prov = OdsProvenance::load_from_dir(release_dir)
        .context("Failed to load _provenance.json from release directory")?;

    let date = prov
        .trud_release_date
        .clone()
        .context("Missing trud_release_date in _provenance.json")?;

    let oci_dir = release_dir.join("oci");
    let blobs_dir = oci_dir.join("blobs").join("sha256");

    let tool_repo = args.tool_repo.clone().or_else(|| find_tool_repo(release_dir));

    if !args.check {
        // Step 0: Validate baseline provenance preconditions BEFORE touching disk!
        prov.validate_baseline()
            .context("✖ Precondition failure: invalid provenance baseline")?;

        // Regenerate oci/ wholesale so stale blobs or symlinks cannot survive
        if oci_dir.exists() {
            fs::remove_dir_all(&oci_dir)?;
        }

        // Step 1: Update dataset_version in _provenance.json on disk
        prov.dataset_version = Some(args.version.clone());
        let prov_path = release_dir.join(PROVENANCE_FILENAME);
        let updated_prov_json = serde_json::to_string_pretty(&prov)?;
        fs::write(&prov_path, updated_prov_json)?;

        // Step 1b: Regenerate datapackage.json with the full dataset SemVer
        let pkg = crate::datapackage::generate_release_datapackage(
            release_dir,
            Some(&prov),
            None,
            Some(&args.version),
        );
        fs::write(
            release_dir.join("datapackage.json"),
            serde_json::to_string_pretty(&pkg)? + "\n",
        )?;

        // Step 2: Write SHA256SUMS from the dynamically discovered files (excluding SHA256SUMS and _release.json)
        let mut publishable_names = Vec::new();
        let entries = fs::read_dir(release_dir)?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with('.') || name == "SHA256SUMS" || name == "_release.json" {
                        continue;
                    }
                    publishable_names.push(name.to_string());
                }
            }
        }
        publishable_names.sort();

        let mut sha_lines = Vec::new();
        for fname in &publishable_names {
            let hash = compute_file_sha256(&release_dir.join(fname))?;
            sha_lines.push(format!("{}  {}", hash.to_uppercase(), fname));
        }
        let sums_path = release_dir.join("SHA256SUMS");
        let sums_content = sha_lines.join("\n") + "\n";
        fs::write(&sums_path, &sums_content)?;

        // Step 3: Build OciManifest from disk
        let (manifest, manifest_bytes) = build_manifest_from_dir(release_dir, &prov, &args.version)?;
        let manifest_digest = manifest.digest()?;

        // Step 4: Write oci/ layout
        fs::create_dir_all(&blobs_dir)?;

        let layout = OciLayout::default();
        let layout_bytes = serde_json::to_vec_pretty(&layout)?;
        fs::write(oci_dir.join("oci-layout"), layout_bytes)?;

        let manifest_hex = manifest_digest.trim_start_matches("sha256:");
        let manifest_blob_path = blobs_dir.join(manifest_hex);
        fs::write(&manifest_blob_path, &manifest_bytes)?;

        let mut ann_date = BTreeMap::new();
        ann_date.insert(ANNOTATION_REF_NAME.to_string(), date.clone());

        let mut ann_versioned = BTreeMap::new();
        ann_versioned.insert(ANNOTATION_REF_NAME.to_string(), format!("{}_{}", date, args.version));

        let entry_date = OciIndexManifestEntry {
            media_type: MEDIA_TYPE_MANIFEST.to_string(),
            artifact_type: ARTIFACT_TYPE_DATASET.to_string(),
            digest: manifest_digest.clone(),
            size: manifest_bytes.len() as u64,
            annotations: Some(ann_date),
        };

        let entry_versioned = OciIndexManifestEntry {
            media_type: MEDIA_TYPE_MANIFEST.to_string(),
            artifact_type: ARTIFACT_TYPE_DATASET.to_string(),
            digest: manifest_digest.clone(),
            size: manifest_bytes.len() as u64,
            annotations: Some(ann_versioned),
        };

        let oci_index = OciIndex {
            schema_version: 2,
            media_type: MEDIA_TYPE_INDEX.to_string(),
            manifests: vec![entry_date, entry_versioned],
        };
        let index_bytes = oci_index.to_canonical_bytes()?;
        fs::write(oci_dir.join("index.json"), index_bytes)?;

        // Step 5: Write relative symlinks in oci/blobs/sha256/
        for layer in &manifest.layers {
            let fname = layer
                .annotations
                .as_ref()
                .and_then(|ann| ann.get(ANNOTATION_TITLE))
                .unwrap();
            let fhash = layer.digest.trim_start_matches("sha256:");
            let link_target = Path::new("../../../").join(fname);
            let link_path = blobs_dir.join(fhash);
            if !link_path.exists() {
                symlink(&link_target, &link_path)
                    .with_context(|| format!("creating symlink at {}", link_path.display()))?;
            }
        }

        // Step 6: Write _release.json
        let versioned_tag = format!("{}_{}", date, args.version);
        let release_row = ReleaseIndexEntry {
            trud_release_date: date.clone(),
            dataset_version: args.version.clone(),
            tag: versioned_tag,
            manifest_digest: manifest_digest.clone(),
            trud_release_sha256: prov.trud_release_sha256.clone().unwrap_or_default().to_uppercase(),
            tool_version: prov.tool_version.clone().unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string()),
            dataset_doi: None,
            withdrawn: None,
        };
        let row_json = serde_json::to_string_pretty(&release_row)?;
        fs::write(release_dir.join("_release.json"), row_json + "\n")?;
    }

    // Step 7: Perform all 11 structural and publishability checks
    // ALWAYS PERFORM ALL CHECKS, EVEN IF SOME FAIL
    let custom_index = if let Some(ref p) = args.index {
        let content = fs::read_to_string(p)?;
        Some(serde_json::from_str::<OdsReleaseIndex>(&content)?)
    } else {
        None
    };

    let failures = perform_all_checks(
        release_dir,
        &args.version,
        tool_repo.as_deref(),
        custom_index.as_ref(),
        args.offline,
    )?;

    if !failures.is_empty() {
        for f in &failures {
            eprintln!("✖ {}", f);
        }
        let count = failures.len();
        let word = if count == 1 { "check" } else { "checks" };
        bail!("{} {} failed", count, word);
    }

    // All checks passed!
    let manifest_path = find_manifest_in_blobs(&blobs_dir)?;
    let manifest_bytes = fs::read(&manifest_path)?;
    let manifest: OciManifest = serde_json::from_slice(&manifest_bytes)?;
    let total_bytes: u64 = manifest.layers.iter().map(|l| l.size).sum();
    let mb = (total_bytes as f64) / 1_048_576.0;

    println!("* {} layers, {:.1} MB", manifest.layers.len(), mb);
    if args.check {
        println!("✓ oci/ verified, manifest {}", manifest.digest()?);
        println!("✓ tags {}, {}_{} verified", date, date, args.version);
        println!("✓ _release.json verified");
    } else {
        println!("✓ oci/ written, manifest {}", manifest.digest()?);
        println!("✓ tags {}, {}_{}", date, date, args.version);
        println!("✓ _release.json written");
    }

    Ok(())
}

fn find_manifest_in_blobs(blobs_dir: &Path) -> Result<PathBuf> {
    for entry in fs::read_dir(blobs_dir)?.flatten() {
        let path = entry.path();
        if path.is_file() && !path.is_symlink() {
            return Ok(path);
        }
    }
    bail!("No real manifest file found in {}", blobs_dir.display());
}

pub fn perform_all_checks_default(release_dir: &Path, expected_version: &str) -> Result<Vec<String>> {
    perform_all_checks(release_dir, expected_version, None, None, true)
}

pub fn perform_all_checks(
    release_dir: &Path,
    expected_version: &str,
    tool_repo: Option<&Path>,
    custom_index: Option<&OdsReleaseIndex>,
    offline: bool,
) -> Result<Vec<String>> {
    let mut failures = Vec::new();

    let prov = match OdsProvenance::load_from_dir(release_dir) {
        Some(p) => p,
        None => {
            failures.push("Failed to load _provenance.json".to_string());
            return Ok(failures);
        }
    };

    // Check 1: Provenance baseline validation
    if let Err(e) = prov.validate_baseline() {
        failures.push(format!("Provenance baseline failure: {}", e));
    }

    // Check 10: tool_git_dirty == false
    if prov.tool_git_dirty == Some(true) {
        failures.push("tool_git_dirty is true: dataset built from a dirty working tree".to_string());
    }

    let oci_dir = release_dir.join("oci");
    let blobs_dir = oci_dir.join("blobs").join("sha256");

    // Check 2: oci-layout exists and has imageLayoutVersion == "1.0.0"
    let layout_path = oci_dir.join("oci-layout");
    if !layout_path.exists() {
        failures.push("Missing oci/oci-layout".to_string());
    } else if let Ok(layout_bytes) = fs::read(&layout_path) {
        if let Ok(layout) = serde_json::from_slice::<OciLayout>(&layout_bytes) {
            if layout.image_layout_version != "1.0.0" {
                failures.push(format!(
                    "oci-layout imageLayoutVersion is '{}', expected '1.0.0'",
                    layout.image_layout_version
                ));
            }
        } else {
            failures.push("Invalid oci/oci-layout JSON".to_string());
        }
    }

    let manifest_path = find_manifest_in_blobs(&blobs_dir).ok();
    let manifest: Option<(Vec<u8>, OciManifest)> = if let Some(ref m_path) = manifest_path {
        match fs::read(m_path) {
            Ok(bytes) => match serde_json::from_slice::<OciManifest>(&bytes) {
                Ok(m) => Some((bytes, m)),
                Err(e) => {
                    failures.push(format!("Invalid OCI manifest: {}", e));
                    None
                }
            },
            Err(e) => {
                failures.push(format!("Failed to read manifest file: {}", e));
                None
            }
        }
    } else {
        failures.push("Missing manifest blob in oci/blobs/sha256/".to_string());
        None
    };

    // Check 3: index.json exists and references the manifest digest for both refs
    let index_path = oci_dir.join("index.json");
    if !index_path.exists() {
        failures.push("Missing oci/index.json".to_string());
    } else if let Ok(index_bytes) = fs::read(&index_path) {
        if let Ok(index) = serde_json::from_slice::<OciIndex>(&index_bytes) {
            let date = prov.trud_release_date.as_deref().unwrap_or("");
            let ref_date = date.to_string();
            let ref_versioned = format!("{}_{}", date, expected_version);

            if let Some((_, ref m)) = manifest {
                let actual_digest = m.digest().unwrap_or_default();

                let has_ref_date = index.manifests.iter().any(|m_entry| {
                    m_entry.digest == actual_digest
                        && m_entry
                            .annotations
                            .as_ref()
                            .and_then(|a| a.get(ANNOTATION_REF_NAME))
                            .map(|r| r == &ref_date)
                            .unwrap_or(false)
                });
                if !has_ref_date {
                    failures.push(format!(
                        "index.json missing manifest entry with ref name '{}' and digest {}",
                        ref_date, actual_digest
                    ));
                }

                let has_ref_versioned = index.manifests.iter().any(|m_entry| {
                    m_entry.digest == actual_digest
                        && m_entry
                            .annotations
                            .as_ref()
                            .and_then(|a| a.get(ANNOTATION_REF_NAME))
                            .map(|r| r == &ref_versioned)
                            .unwrap_or(false)
                });
                if !has_ref_versioned {
                    failures.push(format!(
                        "index.json missing manifest entry with ref name '{}' and digest {}",
                        ref_versioned, actual_digest
                    ));
                }

                for m_entry in &index.manifests {
                    if m_entry.digest != actual_digest {
                        failures.push(format!(
                            "index.json names manifest digest {} but manifest hashes to {}",
                            m_entry.digest, actual_digest
                        ));
                    }
                }
            }
        } else {
            failures.push("Invalid oci/index.json".to_string());
        }
    }

    if let Some((manifest_bytes, ref m)) = manifest {
        // Check 4: blob symlinks resolve and targets hash to link name
        if blobs_dir.exists() {
            if let Ok(entries) = fs::read_dir(&blobs_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_symlink() {
                        let link_name = path.file_name().unwrap().to_string_lossy().to_string();
                        match fs::canonicalize(&path) {
                            Ok(target) => match compute_file_sha256(&target) {
                                Ok(actual_hash) => {
                                    if !actual_hash.eq_ignore_ascii_case(&link_name) {
                                        failures.push(format!(
                                            "blob symlink stale: blobs/sha256/{} -> {}\n  target hashes to sha256:{}",
                                            link_name,
                                            target.display(),
                                            actual_hash.to_lowercase()
                                        ));
                                    }
                                }
                                Err(e) => failures.push(format!(
                                    "Failed to hash symlink target {}: {}",
                                    target.display(),
                                    e
                                )),
                            },
                            Err(_) => {
                                failures.push(format!("blob symlink broken: blobs/sha256/{}", link_name));
                            }
                        }
                    }
                }
            }
        }

        // Check 5: config and every layer digest resolve to a present blob
        let config_hex = m.config.digest.trim_start_matches("sha256:");
        if !blobs_dir.join(config_hex).exists() {
            failures.push(format!("config blob missing: blobs/sha256/{}", config_hex));
        }

        for layer in &m.layers {
            let layer_hex = layer.digest.trim_start_matches("sha256:");
            if !blobs_dir.join(layer_hex).exists() {
                failures.push(format!("layer blob missing: blobs/sha256/{}", layer_hex));
            }
        }

        // Check 6: config.digest equals the digest of the layer titled _provenance.json
        let prov_layer = m.layers.iter().find(|l| {
            l.annotations
                .as_ref()
                .and_then(|ann| ann.get(ANNOTATION_TITLE))
                .map(|t| t == PROVENANCE_FILENAME)
                .unwrap_or(false)
        });
        match prov_layer {
            Some(l) => {
                if l.digest != m.config.digest {
                    failures.push(format!(
                        "config.digest ({}) != _provenance.json layer digest ({})",
                        m.config.digest, l.digest
                    ));
                }
            }
            None => {
                failures.push("Manifest layers missing _provenance.json layer".to_string());
            }
        }

        // Check 7: every digest is lowercase hex
        let check_digest = |d: &str, failures: &mut Vec<String>| {
            if !d.starts_with("sha256:") {
                failures.push(format!("digest does not start with sha256:: {}", d));
                return;
            }
            let hex = &d["sha256:".len()..];
            if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()) {
                failures.push(format!("digest is not lowercase hex: {}", d));
            }
        };

        check_digest(&m.config.digest, &mut failures);
        for layer in &m.layers {
            check_digest(&layer.digest, &mut failures);
        }

        // Check 8: re-packing from source files on disk yields a byte-identical manifest
        match build_manifest_from_dir(release_dir, &prov, expected_version) {
            Ok((_, repacked_bytes)) => {
                if manifest_bytes != repacked_bytes {
                    failures.push(
                        "Re-packing from source files yields non-identical manifest bytes (non-determinism)".to_string(),
                    );
                }
            }
            Err(e) => {
                failures.push(format!("Re-packing from source files failed: {}", e));
            }
        }

        // Check 9: SHA256SUMS agrees with layers[] in both directions
        let sums_path = release_dir.join("SHA256SUMS");
        if !sums_path.exists() {
            failures.push("Missing SHA256SUMS".to_string());
        } else if let Ok(sums_content) = fs::read_to_string(&sums_path) {
            let mut recorded_sums = HashMap::new();
            for line in sums_content.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() == 2 {
                    recorded_sums.insert(parts[1].to_string(), parts[0].to_lowercase());
                }
            }

            for layer in &m.layers {
                let title = layer
                    .annotations
                    .as_ref()
                    .and_then(|a| a.get(ANNOTATION_TITLE))
                    .map(|s| s.as_str())
                    .unwrap_or("");
                if title == "SHA256SUMS" {
                    continue;
                }
                let layer_hex = layer.digest.trim_start_matches("sha256:").to_lowercase();
                if let Some(recorded_hex) = recorded_sums.get(title) {
                    if layer_hex != *recorded_hex {
                        failures.push(format!("SHA256SUMS lists {} with a digest no layer carries", title));
                    }
                } else {
                    failures.push(format!("Layer {} missing from SHA256SUMS", title));
                }
            }

            for fname in recorded_sums.keys() {
                let found = m.layers.iter().any(|l| {
                    l.annotations
                        .as_ref()
                        .and_then(|a| a.get(ANNOTATION_TITLE))
                        .map(|t| t == fname)
                        .unwrap_or(false)
                });
                if !found {
                    failures.push(format!("File {} listed in SHA256SUMS not in manifest layers", fname));
                }
            }
        }
    }

    // Git checks (Checks 11 and 15) run against the tool's repo directory
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

    // Check 12: trud_release_sha256_verified == "trud_api" or "published_release"
    match prov.trud_release_sha256_verified {
        Some(crate::provenance::TrudVerificationSource::TrudApi)
        | Some(crate::provenance::TrudVerificationSource::PublishedRelease) => {}
        _ => {
            failures.push(
                "trud_release_sha256_verified is not trud_api: source was never verified against TRUD".to_string(),
            );
        }
    }

    // Check 13: dataset_version matches expected_version and parses as semver
    if let Some(ref prov_ver) = prov.dataset_version {
        if prov_ver != expected_version {
            failures.push(format!(
                "Provenance dataset_version ({}) != expected version ({})",
                prov_ver, expected_version
            ));
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
                "data/releases.json already has a row for {} {}\n  Published releases are immutable. Bump the patch version.",
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

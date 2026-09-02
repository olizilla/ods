use anyhow::{bail, Context, Result};
use clap::Parser;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use crate::oci::*;
use crate::provenance::{compute_file_sha256, OdsProvenance, PROVENANCE_FILENAME};

#[derive(Parser, Debug, Clone)]
pub struct Args {
    /// Path to compiled release directory (defaults to active release)
    #[arg(long, short)]
    pub input: Option<PathBuf>,

    /// Verify existing OCI layout without rebuilding
    #[arg(long)]
    pub check: bool,
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
                if name.starts_with('.') || name == "oci" {
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

    let manifest = OciManifest {
        schema_version: 2,
        media_type: MEDIA_TYPE_MANIFEST.to_string(),
        artifact_type: ARTIFACT_TYPE_DATASET.to_string(),
        config,
        layers,
        annotations: Some(annotations),
    };

    let manifest_bytes = manifest.to_canonical_bytes()?;
    Ok((manifest, manifest_bytes))
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

    let mut prov = match OdsProvenance::load_from_dir(&release_dir) {
        Some(p) => p,
        None => bail!(
            "Missing or unreadable _provenance.json in {}",
            release_dir.display()
        ),
    };

    let version = prov.dataset_version.clone().ok_or_else(|| {
        anyhow::anyhow!("Missing dataset_version in _provenance.json\n  Run `ods make` to build the release directory.")
    })?;

    let date = prov
        .trud_release_date
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Provenance missing trud_release_date"))?;

    let oci_dir = release_dir.join("oci");
    let blobs_dir = oci_dir.join("blobs").join("sha256");

    if !args.check {
        // Step 0: Validate baseline provenance preconditions BEFORE touching disk!
        prov.validate_baseline()
            .context("✖ Precondition failure: invalid provenance baseline")?;

        // Regenerate oci/ wholesale so stale blobs or symlinks cannot survive
        if oci_dir.exists() {
            fs::remove_dir_all(&oci_dir)?;
        }

        // Step 1: Update dataset_version in _provenance.json on disk
        prov.dataset_version = Some(version.clone());
        let prov_path = release_dir.join(PROVENANCE_FILENAME);
        let updated_prov_json = serde_json::to_string_pretty(&prov)?;
        fs::write(&prov_path, updated_prov_json)?;

        // Step 1b: Regenerate datapackage.json with the full dataset SemVer
        let pkg = crate::datapackage::generate_release_datapackage(
            &release_dir,
            Some(&prov),
            None,
            Some(&version),
        );
        fs::write(
            release_dir.join("datapackage.json"),
            serde_json::to_string_pretty(&pkg)? + "\n",
        )?;

        // Step 2: Build OciManifest from disk
        let (manifest, manifest_bytes) = build_manifest_from_dir(&release_dir, &prov, &version)?;
        let manifest_digest = manifest.digest()?;

        // Step 3: Write oci/ layout
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
        ann_versioned.insert(ANNOTATION_REF_NAME.to_string(), format!("{}_{}", date, version));

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

        // Step 4: Write relative symlinks in oci/blobs/sha256/
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
    }

    // Step 5: Perform structural checks 1-8
    let failures = perform_structural_checks(&release_dir, &version)?;

    if !failures.is_empty() {
        for failure in &failures {
            eprintln!("✖ {}", failure);
        }
        let count = failures.len();
        eprintln!("{} check{} failed", count, if count == 1 { "" } else { "s" });
        bail!("Structural checks failed");
    }

    let manifest_path = find_manifest_in_blobs(&blobs_dir)?;
    let manifest_bytes = fs::read(&manifest_path)?;
    let manifest: OciManifest = serde_json::from_slice(&manifest_bytes)?;
    let manifest_digest = manifest.digest()?;

    let total_size: u64 = manifest.layers.iter().map(|l| l.size).sum();
    let size_mb = (total_size as f64) / (1024.0 * 1024.0);

    let pub_warnings = prov.validate_publishable();

    if args.check {
        println!("* {} layers, {:.1} MB", manifest.layers.len(), size_mb);
        println!("✓ oci/ verified");
        if !pub_warnings.is_empty() {
            for warn in &pub_warnings {
                println!("  {}", warn);
            }
            println!("  this layout is structurally valid; `ods make release` will refuse it");
        }
    } else {
        println!("* {} layers, {:.1} MB", manifest.layers.len(), size_mb);
        println!("✓ oci/ written, manifest {}", manifest_digest);
        if !pub_warnings.is_empty() {
            for warn in &pub_warnings {
                println!("  {}", warn);
            }
            println!("  this layout is structurally valid; `ods make release` will refuse it");
        }
        println!("✓ tags {}, {}_{}", date, date, version);
    }

    Ok(())
}

fn find_manifest_in_blobs(blobs_dir: &Path) -> Result<PathBuf> {
    if !blobs_dir.exists() {
        bail!("blobs directory does not exist: {}", blobs_dir.display());
    }
    for entry in fs::read_dir(blobs_dir)?.flatten() {
        let path = entry.path();
        if path.is_file() && !path.is_symlink() {
            return Ok(path);
        }
    }
    bail!("No real manifest file found in {}", blobs_dir.display());
}

pub fn perform_structural_checks(release_dir: &Path, expected_version: &str) -> Result<Vec<String>> {
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

        // Check 5b: datapackage.json resource hashes agree with layer digests
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
                                let found_layer = m.layers.iter().find(|l| {
                                    l.annotations
                                        .as_ref()
                                        .and_then(|a| a.get(ANNOTATION_TITLE))
                                        .map(|t| t == res_name || t == res_path)
                                        .unwrap_or(false)
                                });
                                match found_layer {
                                    Some(l) => {
                                        let l_hash = l.digest.trim_start_matches("sha256:").to_lowercase();
                                        if clean_hash != l_hash {
                                            failures.push(format!(
                                                "datapackage.json resource {} hash {} disagrees with layer digest {}",
                                                res_name, res_hash, l.digest
                                            ));
                                        }
                                    }
                                    None => {
                                        failures.push(format!(
                                            "datapackage.json resource {} not found in manifest layers",
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
    }

    Ok(failures)
}

pub fn perform_all_checks(
    release_dir: &Path,
    expected_version: &str,
    _tool_repo: Option<&Path>,
    _custom_index: Option<&crate::index::OdsReleaseIndex>,
    _offline: bool,
) -> Result<Vec<String>> {
    perform_structural_checks(release_dir, expected_version)
}

pub fn perform_all_checks_default(release_dir: &Path, expected_version: &str) -> Result<Vec<String>> {
    perform_structural_checks(release_dir, expected_version)
}

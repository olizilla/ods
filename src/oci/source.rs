//! A TRUD archive, packed once as an OCI bundle of exactly what NHS published: the zip, the
//! signed checksum, its signature and NHS's public key, plus NHS's licence and attribution.
//!
//! A bundle is published and forgotten, so its digest is a pure function of those four files
//! and the frozen terms: no clock value, nothing about the tool, layers sorted by title, compact
//! JSON. Anyone holding the same four files packs the same digest with any version of `ods`.
//! The packing rules here freeze with the first push, and `tests/oci_source_manifest_golden_test.rs`
//! pins them.
//!
//! The bundle's date, its tag and the `fyi.ods.trud-release-date` annotation are TRUD's release
//! date from `_provenance.json`, not the date in the zip's filename, which NHS stamps differently
//! for some releases.
//!
//! The `fyi.ods.trud-release-sha256` annotation is a convenience copy of the zip layer's digest,
//! in upper case, in the same form as the dataset manifests' annotation, so a dataset links to its
//! source bundle by one value. It adds no trust: the layer's own digest is the same hash.

use anyhow::{bail, Context, Result};
use base64::Engine;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use super::*;
use crate::provenance::{compute_file_sha256, OdsProvenance, ProvenanceLoad, PROVENANCE_FILENAME};

pub const ARTIFACT_TYPE_SOURCE: &str = "application/vnd.fyi.ods.source.v1";
pub const MEDIA_TYPE_EMPTY: &str = "application/vnd.oci.empty.v1+json";
pub const MEDIA_TYPE_ZIP: &str = "application/zip";
pub const MEDIA_TYPE_XML: &str = "application/xml";
pub const MEDIA_TYPE_PGP_SIGNATURE: &str = "application/pgp-signature";
pub const MEDIA_TYPE_PGP_KEYS: &str = "application/pgp-keys";
pub const ANNOTATION_FYI_ATTRIBUTION: &str = "fyi.ods.attribution";

/// OCI's empty descriptor: the two bytes `{}`.
const EMPTY_CONFIG: &[u8] = b"{}";

/// The directory a release's bundle is packed into, under its `trud/`.
pub const BUNDLE_DIR: &str = "oci";

/// A check that failed, with what to show: which release, which check, and both values.
#[derive(Debug)]
pub struct Refusal {
    pub release: String,
    pub what: String,
    pub details: Vec<String>,
}

impl Refusal {
    fn new(release: &str, what: &str, details: Vec<String>) -> Self {
        Self { release: release.to_string(), what: what.to_string(), details }
    }

    /// The `✖` block, as printed.
    pub fn block(&self) -> String {
        let mut out = format!("✖ {}  {}", self.release, self.what);
        for line in &self.details {
            out.push_str(&format!("\n  {}", line));
        }
        out
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.block())
    }
}

impl std::error::Error for Refusal {}

/// One of NHS's four files, as it goes into the bundle.
#[derive(Debug, Clone)]
struct SourceFile {
    name: String,
    media_type: &'static str,
    sha256_hex: String,
    size: u64,
}

/// A bundle, ready to write or to compare with what's on disk.
#[derive(Debug, Clone)]
pub struct SourceBundle {
    pub date: String,
    pub manifest: OciManifest,
    pub manifest_bytes: Vec<u8>,
    pub manifest_digest: String,
    /// The zip's size, the number a listing shows.
    pub zip_size: u64,
    files: Vec<SourceFile>,
}

struct Found {
    zip: PathBuf,
    checksum: PathBuf,
    signature: PathBuf,
    key: PathBuf,
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

/// Finds NHS's four files in `trud_dir`, by their names. Nothing else in the directory is read.
fn find_files(release: &str, trud_dir: &Path) -> Result<Found, Refusal> {
    let mut zips = Vec::new();
    let mut checksums = Vec::new();
    let mut signatures = Vec::new();
    let mut keys = Vec::new();
    if let Ok(entries) = fs::read_dir(trud_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = file_name(&path);
            if name.ends_with(".zip") {
                zips.push(path);
            } else if name.starts_with("trud_") && name.ends_with(".xml") {
                checksums.push(path);
            } else if name.starts_with("trud_") && name.ends_with(".xml.asc") {
                signatures.push(path);
            } else if name.starts_with("trud-public-key-") && name.ends_with(".pgp") {
                keys.push(path);
            }
        }
    }

    let refuse_missing = |missing: &[&str]| {
        Refusal::new(
            release,
            "a bundle needs all four of NHS's files",
            vec![
                format!("missing: {}", missing.join(", ")),
                format!("in {}", trud_dir.display()),
                format!("Fetch them with: ods trud pull {} (--force refreshes them)", release),
            ],
        )
    };
    let mut missing = Vec::new();
    if zips.is_empty() {
        missing.push("the zip");
    }
    if checksums.is_empty() {
        missing.push("the checksum (trud_<name>.xml)");
    }
    if signatures.is_empty() {
        missing.push("its signature (trud_<name>.xml.asc)");
    }
    if keys.is_empty() {
        missing.push("NHS's public key (trud-public-key-*.pgp)");
    }
    if !missing.is_empty() {
        return Err(refuse_missing(&missing));
    }

    for (kind, found) in [("zip", &zips), ("checksum", &checksums), ("signature", &signatures), ("key", &keys)] {
        if found.len() > 1 {
            let mut names: Vec<String> = found.iter().map(|p| file_name(p)).collect();
            names.sort();
            return Err(Refusal::new(
                release,
                &format!("a bundle holds one {}, and {} has {}", kind, trud_dir.display(), found.len()),
                names,
            ));
        }
    }
    Ok(Found {
        zip: zips.remove(0),
        checksum: checksums.remove(0),
        signature: signatures.remove(0),
        key: keys.remove(0),
    })
}

/// NHS's checksum file is Microsoft FCIV XML: `<name>` is the zip's filename and `<SHA1>` its
/// SHA-1, base64. Returns both.
fn read_checksum_xml(path: &Path) -> Result<(String, String)> {
    use quick_xml::events::Event;
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut reader = quick_xml::Reader::from_str(&text);
    reader.trim_text(true);
    let (mut name, mut sha1, mut current) = (None, None, String::new());
    loop {
        match reader.read_event()? {
            Event::Start(e) => current = String::from_utf8_lossy(e.name().as_ref()).to_string(),
            Event::Text(t) => {
                let value = t.unescape()?.to_string();
                match current.as_str() {
                    "name" => name = Some(value),
                    "SHA1" => sha1 = Some(value),
                    _ => {}
                }
            }
            Event::End(_) => current.clear(),
            Event::Eof => break,
            _ => {}
        }
    }
    match (name, sha1) {
        (Some(n), Some(s)) => Ok((n, s)),
        _ => bail!("{} isn't an FCIV checksum file: no <name> and <SHA1>", path.display()),
    }
}

fn sha1_base64(path: &Path) -> Result<String> {
    let mut hasher = Sha1::new();
    let mut file = fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    std::io::copy(&mut file, &mut hasher)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(hasher.finalize()))
}

fn file_entry(path: &Path, media_type: &'static str) -> Result<SourceFile> {
    Ok(SourceFile {
        name: file_name(path),
        media_type,
        sha256_hex: compute_file_sha256(path)?.to_lowercase(),
        size: fs::metadata(path)?.len(),
    })
}

/// Runs the three checks the bundle needs before anything is written, and builds it.
///
/// 1. all four files are present;
/// 2. the checksum file names this zip, and its SHA-1 is the zip's;
/// 3. the zip's SHA-256 and size are the ones `_provenance.json` records, which `ods trud pull`
///    wrote after checking TRUD's listing.
///
/// `ods` doesn't verify NHS's PGP signature: `scripts/verify-trud-bundle.sh` does, against the
/// key fingerprint pinned in `data/releases.json`.
pub fn gather(release_dir: &Path) -> Result<SourceBundle, Refusal> {
    let trud_dir = release_dir.join("trud");
    let dir_name = file_name(release_dir);
    let found = find_files(&dir_name, &trud_dir)?;

    let zip_name = file_name(&found.zip);

    // Until `_provenance.json` is read, a refusal is labelled with the release directory's name.
    // The zip's filename isn't a source of the date: NHS's stamp and TRUD's release date differ
    // for some releases (2022-05-30's zip is stamped 20220527).
    let mut release = dir_name.clone();
    let unreadable = |release: &str, what: &str, e: anyhow::Error| Refusal::new(release, what, vec![format!("{:#}", e)]);

    // 2. NHS's checksum names this zip, and holds its SHA-1
    let (named, expected_sha1) = read_checksum_xml(&found.checksum)
        .map_err(|e| unreadable(&release, "the checksum file can't be read", e))?;
    if named != zip_name {
        return Err(Refusal::new(
            &release,
            "NHS's checksum file names a different zip",
            vec![format!("the checksum's <name>  {}", named), format!("the zip's filename     {}", zip_name)],
        ));
    }
    let actual_sha1 = sha1_base64(&found.zip).map_err(|e| unreadable(&release, "the zip can't be read", e))?;
    if actual_sha1 != expected_sha1 {
        return Err(Refusal::new(
            &release,
            "the zip isn't the one NHS's checksum file describes",
            vec![
                format!("NHS's SHA-1     {}", expected_sha1),
                format!("the zip's SHA-1  {}", actual_sha1),
            ],
        ));
    }

    // 3. `_provenance.json` records this zip's SHA-256 and size
    let prov_path = release_dir.join(PROVENANCE_FILENAME);
    let prov = match OdsProvenance::load_from_file(&prov_path) {
        ProvenanceLoad::Read(p, _) => *p,
        _ => {
            return Err(Refusal::new(
                &release,
                &format!("there is no readable {} to check the zip against", PROVENANCE_FILENAME),
                vec![format!("Write it from TRUD's listing with: ods trud pull {}", release)],
            ))
        }
    };
    // From here on the release is TRUD's date, which `_provenance.json` records.
    let date = match prov.trud_release_date.clone() {
        Some(date) if !date.is_empty() => date,
        _ => {
            return Err(Refusal::new(
                &release,
                &format!("{} records no release date", PROVENANCE_FILENAME),
                vec![format!("Write it from TRUD's listing with: ods trud pull {}", release)],
            ))
        }
    };
    release = date.clone();
    let zip_file =
        file_entry(&found.zip, MEDIA_TYPE_ZIP).map_err(|e| unreadable(&release, "the zip can't be read", e))?;
    let zip_sha256 = zip_file.sha256_hex.to_uppercase();
    let recorded_sha256 = prov.trud_release_sha256.clone().unwrap_or_default();
    if !recorded_sha256.eq_ignore_ascii_case(&zip_sha256) {
        return Err(Refusal::new(
            &release,
            &format!("{} names a different archive", PROVENANCE_FILENAME),
            vec![
                format!("the zip's SHA-256        {}", zip_sha256),
                format!("{} records  {}", PROVENANCE_FILENAME, recorded_sha256),
            ],
        ));
    }
    if prov.trud_release_filesize_bytes != Some(zip_file.size) {
        return Err(Refusal::new(
            &release,
            &format!("{} records a different size", PROVENANCE_FILENAME),
            vec![
                format!("the zip's size        {} bytes", zip_file.size),
                format!("{} records  {}", PROVENANCE_FILENAME, prov.trud_release_filesize_bytes.map_or("nothing".to_string(), |s| format!("{} bytes", s))),
            ],
        ));
    }

    let mut files = vec![
        zip_file,
        file_entry(&found.checksum, MEDIA_TYPE_XML)
            .map_err(|e| unreadable(&release, "the checksum file can't be read", e))?,
        file_entry(&found.signature, MEDIA_TYPE_PGP_SIGNATURE)
            .map_err(|e| unreadable(&release, "the signature can't be read", e))?,
        file_entry(&found.key, MEDIA_TYPE_PGP_KEYS).map_err(|e| unreadable(&release, "the key can't be read", e))?,
    ];
    files.sort_by(|a, b| a.name.cmp(&b.name));
    let zip_size = files.iter().find(|f| f.media_type == MEDIA_TYPE_ZIP).map(|f| f.size).unwrap_or(0);

    let config_digest = format!("sha256:{:x}", Sha256::digest(EMPTY_CONFIG));
    let layers = files
        .iter()
        .map(|f| OciDescriptor::new(f.media_type, &format!("sha256:{}", f.sha256_hex), f.size).with_title(&f.name))
        .collect();
    let mut annotations = BTreeMap::new();
    annotations.insert(ANNOTATION_FYI_TRUD_RELEASE_DATE.to_string(), date.clone());
    annotations.insert(ANNOTATION_FYI_TRUD_RELEASE_SHA256.to_string(), zip_sha256);
    annotations.insert(ANNOTATION_LICENSES.to_string(), crate::terms::LICENSE.to_string());
    annotations.insert(ANNOTATION_FYI_ATTRIBUTION.to_string(), crate::terms::ATTRIBUTION.to_string());
    let manifest = OciManifest {
        schema_version: 2,
        media_type: MEDIA_TYPE_MANIFEST.to_string(),
        artifact_type: ARTIFACT_TYPE_SOURCE.to_string(),
        config: OciDescriptor::new(MEDIA_TYPE_EMPTY, &config_digest, EMPTY_CONFIG.len() as u64),
        layers,
        annotations: Some(annotations),
    };
    let manifest_bytes = manifest
        .to_canonical_bytes()
        .map_err(|e| unreadable(&release, "the manifest can't be written", e))?;
    let manifest_digest =
        manifest.digest().map_err(|e| unreadable(&release, "the manifest can't be written", e))?;
    Ok(SourceBundle { date, manifest, manifest_bytes, manifest_digest, zip_size, files })
}

fn index_bytes(bundle: &SourceBundle) -> Result<Vec<u8>> {
    let mut annotations = BTreeMap::new();
    annotations.insert(ANNOTATION_REF_NAME.to_string(), bundle.date.clone());
    OciIndex {
        schema_version: 2,
        media_type: MEDIA_TYPE_INDEX.to_string(),
        manifests: vec![OciIndexManifestEntry {
            media_type: MEDIA_TYPE_MANIFEST.to_string(),
            artifact_type: ARTIFACT_TYPE_SOURCE.to_string(),
            digest: bundle.manifest_digest.clone(),
            size: bundle.manifest_bytes.len() as u64,
            annotations: Some(annotations),
        }],
    }
    .to_canonical_bytes()
}

/// Packs `release_dir/trud/` into `release_dir/trud/oci/`, an OCI image layout whose layer
/// blobs are relative symlinks to the files. Nothing is written unless every check passes.
pub fn pack(release_dir: &Path) -> Result<SourceBundle, Refusal> {
    let bundle = gather(release_dir)?;
    let oci_dir = release_dir.join("trud").join(BUNDLE_DIR);
    let write = || -> Result<()> {
        if oci_dir.exists() {
            fs::remove_dir_all(&oci_dir)?;
        }
        let blobs = oci_dir.join("blobs").join("sha256");
        fs::create_dir_all(&blobs)?;
        fs::write(oci_dir.join("oci-layout"), OciLayout::default().to_canonical_bytes()?)?;
        fs::write(oci_dir.join("index.json"), index_bytes(&bundle)?)?;
        fs::write(blobs.join(bundle.manifest_digest.trim_start_matches("sha256:")), &bundle.manifest_bytes)?;
        fs::write(blobs.join(bundle.manifest.config.digest.trim_start_matches("sha256:")), EMPTY_CONFIG)?;
        for file in &bundle.files {
            symlink(Path::new("../../../").join(&file.name), blobs.join(&file.sha256_hex))
                .with_context(|| format!("linking {}", file.name))?;
        }
        Ok(())
    };
    write().map_err(|e| Refusal::new(&bundle.date, "the bundle can't be written", vec![format!("{:#}", e)]))?;
    Ok(bundle)
}

/// Whether `release_dir/trud/oci/` is the bundle its four files make today, byte for byte:
/// the manifest, the index, the empty config, and every layer's link.
pub fn verify(release_dir: &Path) -> Result<SourceBundle, Refusal> {
    let bundle = gather(release_dir)?;
    let oci_dir = release_dir.join("trud").join(BUNDLE_DIR);
    let blobs = oci_dir.join("blobs").join("sha256");
    let stale = |what: String| Refusal::new(&bundle.date, "trud/oci/isn't the bundle these files make", vec![what]);

    let manifest_hex = bundle.manifest_digest.trim_start_matches("sha256:");
    let on_disk = fs::read(blobs.join(manifest_hex)).map_err(|_| stale(format!("no manifest blob {}", manifest_hex)))?;
    if on_disk != bundle.manifest_bytes {
        return Err(stale("the manifest on disk differs".to_string()));
    }
    let index = index_bytes(&bundle).map_err(|e| stale(format!("{:#}", e)))?;
    if fs::read(oci_dir.join("index.json")).ok().as_deref() != Some(index.as_slice()) {
        return Err(stale("index.json differs".to_string()));
    }
    if fs::read(oci_dir.join("oci-layout")).is_err() {
        return Err(stale("oci-layout is missing".to_string()));
    }
    if fs::read(blobs.join(bundle.manifest.config.digest.trim_start_matches("sha256:"))).ok().as_deref()
        != Some(EMPTY_CONFIG)
    {
        return Err(stale("the empty config blob is missing".to_string()));
    }
    for file in &bundle.files {
        let link = blobs.join(&file.sha256_hex);
        let holds = link.is_symlink()
            && compute_file_sha256(&link).map(|h| h.to_lowercase() == file.sha256_hex).unwrap_or(false);
        if !holds {
            return Err(stale(format!("{} isn't linked to its blob", file.name)));
        }
    }
    Ok(bundle)
}

impl SourceBundle {
    /// The four layers' filenames, in the bundle's order.
    pub fn layer_names(&self) -> Vec<String> {
        self.files.iter().map(|f| f.name.clone()).collect()
    }
}

//! Provenance: what a release is, and where it came from.
//!
//! Every Parquet file a build writes carries one key-value metadata key, `datapackage`, holding
//! the embedded object (`Embedded`): the dataset's name and version, its licence and attribution,
//! who holds the rights and distributes, and the source release it was built from, with that
//! release's hash and size. The keys are Data Package property names. Every command that says
//! what a release is reads it from the files (`read_release`), so a copied file, or a directory
//! holding only the Parquet files, says the same thing a workspace release does.
//!
//! `ods trud pull` records the TRUD release it downloaded as `trud/datapackage.json` (`TrudArchivePackage`):
//! the upstream zip can't carry our metadata, so the release is described beside it. `ods make`
//! reads it and derives the embedded object's `sources[0]` from it.

use anyhow::{Context, Result};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

/// The Parquet key-value metadata key the embedded object lives under.
pub const DATAPACKAGE_KEY: &str = "datapackage";

/// The TRUD archive package's file name, under a release's `trud/`.
pub const TRUD_ARCHIVE_PACKAGE_FILENAME: &str = "datapackage.json";

/// The TRUD archive package names the TRUD release as this Data Package `name`. `ods make` checks it to
/// tell a source directory's record from a release's `datapackage.json` view.
pub const SOURCE_NAME: &str = "nhs-ods-xml";

/// The TRUD release's own title, as the TRUD archive package states it.
pub const SOURCE_TITLE: &str = "NHS Organisation Data Service XML Data";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrudVerificationSource {
    TrudApi,
    // Reserved for 07-pull-index-and-trust-chain; local archive yields Unverified until 07
    PublishedRelease,
    Unverified,
}

/// A Data Package licence, with the attribution the licensor asks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct License {
    pub name: String,
    pub path: String,
    pub title: String,
    pub attribution: String,
}

/// A Data Package contributor: who, in which DataCite roles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contributor {
    pub title: String,
    pub roles: Vec<String>,
}

/// The source release a dataset was built from: a Data Package Source, with the release's hash
/// and size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub title: String,
    pub version: String,
    pub path: String,
    pub hash: String,
    pub bytes: u64,
}

/// The object every Parquet file carries under `datapackage`. Field order is the serialised
/// order. A build without provenance (`ods make --force -o`) carries no `datapackage` key at
/// all: it can't back a name, a licence or a source for an archive nobody could match, and the
/// key's absence is how every reader recognises it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Embedded {
    pub name: String,
    pub version: String,
    pub licenses: Vec<License>,
    pub contributors: Vec<Contributor>,
    pub sources: Vec<Source>,
}

impl Embedded {
    /// Compact JSON, as written into the Parquet metadata.
    pub fn to_compact_json(&self) -> Result<String> {
        serde_json::to_string(self).context("serialising the embedded datapackage object")
    }
}

/// One file of the TRUD release, as the TRUD archive package lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordResource {
    pub name: String,
    pub path: String,
    pub mediatype: String,
    pub bytes: u64,
    pub hash: String,
}

/// `trud/datapackage.json`: the TRUD release `ods trud pull` downloaded, as a Data Package. The
/// archive's hash and size are TRUD's word; the other files' are hashed from disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrudArchivePackage {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub name: String,
    pub version: String,
    pub title: String,
    pub homepage: String,
    pub licenses: Vec<License>,
    pub contributors: Vec<Contributor>,
    pub resources: Vec<RecordResource>,
}

/// The `sha256:<lower-case hex>` form every hash in the Data Package vocabulary uses.
pub fn prefixed_sha256(hex: &str) -> String {
    format!("sha256:{}", hex.trim_start_matches("sha256:").to_lowercase())
}

impl TrudArchivePackage {
    /// The record for a TRUD release: the archive from TRUD's word, then each of NHS's other
    /// files (`(name, mediatype, path)`) hashed from disk.
    pub fn for_trud_release(
        date: &str,
        archive_file_name: &str,
        archive_sha256: &str,
        archive_bytes: u64,
        other_files: &[(&str, &str, PathBuf)],
    ) -> Result<Self> {
        let mut resources = vec![RecordResource {
            name: "archive".to_string(),
            path: archive_file_name.to_string(),
            mediatype: crate::oci::source::MEDIA_TYPE_ZIP.to_string(),
            bytes: archive_bytes,
            hash: prefixed_sha256(archive_sha256),
        }];
        for (name, mediatype, path) in other_files {
            let bytes = std::fs::metadata(path).with_context(|| format!("reading {}", path.display()))?.len();
            resources.push(RecordResource {
                name: name.to_string(),
                path: path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                mediatype: mediatype.to_string(),
                bytes,
                hash: prefixed_sha256(&compute_file_sha256(path)?),
            });
        }
        Ok(Self {
            schema: crate::datapackage::ODS_DATAPACKAGE_SCHEMA_URL.to_string(),
            name: SOURCE_NAME.to_string(),
            version: date.to_string(),
            title: SOURCE_TITLE.to_string(),
            homepage: crate::terms::LANDING_PAGE.to_string(),
            licenses: vec![crate::terms::license()],
            contributors: crate::terms::contributors(),
            resources,
        })
    }

    /// The archive resource: the zip TRUD vouches for.
    pub fn archive(&self) -> Option<&RecordResource> {
        self.resources.iter().find(|r| r.name == "archive")
    }

    /// Upper-case hex of the archive's SHA-256, as TRUD writes it.
    pub fn archive_sha256_upper(&self) -> Option<String> {
        self.archive().map(|a| a.hash.trim_start_matches("sha256:").to_uppercase())
    }

    pub fn to_json_string(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)? + "\n")
    }

    /// Whether the record's licences are the ones this `ods` states.
    pub fn has_current_terms(&self) -> bool {
        self.licenses == vec![crate::terms::license()]
    }

    /// What `ods make` needs before it builds: a release date, and the archive's hash and size.
    pub fn validate_baseline(&self) -> Result<()> {
        if self.version.is_empty() {
            anyhow::bail!("{} records no version (the TRUD release date)", TRUD_ARCHIVE_PACKAGE_FILENAME);
        }
        let Some(archive) = self.archive() else {
            anyhow::bail!("{} lists no archive resource", TRUD_ARCHIVE_PACKAGE_FILENAME);
        };
        if !is_prefixed_sha256(&archive.hash) {
            anyhow::bail!("{} records the archive's hash as '{}', not sha256:<hex>", TRUD_ARCHIVE_PACKAGE_FILENAME, archive.hash);
        }
        // A file name beside the record, as `ods trud pull` writes it: never a path that leaves `trud/`.
        let mut components = Path::new(&archive.path).components();
        if !matches!((components.next(), components.next()), (Some(std::path::Component::Normal(_)), None)) {
            anyhow::bail!("{} names the archive as '{}', not a file name beside it", TRUD_ARCHIVE_PACKAGE_FILENAME, archive.path);
        }
        Ok(())
    }

    /// The archive this record describes: the file its `archive` resource names, beside the
    /// record. Refuses, with the repair, when it isn't there.
    pub fn archive_path(&self, record_path: &Path) -> Result<PathBuf> {
        let archive = self.archive().context("the TRUD archive package lists no archive resource")?;
        let dir = record_path.parent().unwrap_or(Path::new("."));
        let path = dir.join(&archive.path);
        if !path.is_file() {
            anyhow::bail!(
                "✖ {} names the archive {}, and it isn't in {}\n  Download it again: ods trud pull {} --force",
                format_provenance_display_path(record_path),
                archive.path,
                crate::workspace::relative_to_cwd(dir).display(),
                self.version
            );
        }
        Ok(path)
    }

    /// The embedded object a build from this release carries: `sources[0]` is derived from the
    /// record (`title`, `version`, `path` ← `homepage`, `hash`/`bytes` ← the archive), each
    /// verbatim. Who holds the rights is in `contributors`.
    pub fn embedded(&self, dataset_version: &str) -> Result<Embedded> {
        self.validate_baseline()?;
        let archive = self.archive().expect("validate_baseline checked the archive");
        Ok(Embedded {
            name: crate::datapackage::NAME.to_string(),
            version: format!("{}_{}", self.version, dataset_version),
            licenses: self.licenses.clone(),
            contributors: self.contributors.clone(),
            sources: vec![Source {
                title: self.title.clone(),
                version: self.version.clone(),
                path: self.homepage.clone(),
                hash: archive.hash.clone(),
                bytes: archive.bytes,
            }],
        })
    }

    /// Writes `release_dir/trud/datapackage.json`, and removes the `trud/_provenance.json` an
    /// older `ods trud pull` wrote there, which this record replaces.
    pub fn write_to_dir(&self, release_dir: &Path) -> Result<()> {
        let path = trud_archive_package_path(release_dir);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&path, self.to_json_string()?).with_context(|| format!("writing {}", path.display()))?;
        let legacy = trud_archive_package_dir(release_dir).join(LEGACY_PROVENANCE_FILENAME);
        if legacy.is_file() {
            let _ = std::fs::remove_file(legacy);
        }
        Ok(())
    }

    /// Reads a TRUD archive package from `path`.
    pub fn load_from_file(path: &Path) -> RecordLoad {
        if !path.exists() {
            return RecordLoad::Absent;
        }
        let unreadable = || RecordLoad::Unreadable {
            path: path.to_path_buf(),
            date: date_for_unreadable_record(path),
        };
        let Ok(content) = std::fs::read_to_string(path) else {
            return unreadable();
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
            return unreadable();
        };
        // A `datapackage.json` that doesn't name the TRUD release is some other Data Package —
        // a release's view — and not a TRUD archive package.
        if value.get("name").and_then(|n| n.as_str()) != Some(SOURCE_NAME) {
            return RecordLoad::NotARecord;
        }
        match serde_json::from_value::<Self>(value) {
            Ok(record) if record.schema == crate::datapackage::ODS_DATAPACKAGE_SCHEMA_URL => {
                RecordLoad::Read(Box::new(record), path.to_path_buf())
            }
            _ => unreadable(),
        }
    }

    /// Finds the TRUD archive package from `dir` or one of up to 4 ancestors: at each level
    /// `<level>/trud/datapackage.json`, then `<level>/datapackage.json` (an explicit `-i <dir>`
    /// holding the zip and the record directly). A `datapackage.json` that names anything but
    /// the TRUD release is a release's view, not a record, and is passed over.
    pub fn load_from_dir(dir: &Path) -> RecordLoad {
        let mut curr = if dir.is_file() { dir.parent().map(|p| p.to_path_buf()) } else { Some(dir.to_path_buf()) };
        for _ in 0..4 {
            let Some(ref path) = curr else { break };
            for candidate in [trud_archive_package_path(path), path.join(TRUD_ARCHIVE_PACKAGE_FILENAME)] {
                match Self::load_from_file(&candidate) {
                    RecordLoad::Absent | RecordLoad::NotARecord => {}
                    found => return found,
                }
            }
            curr = path.parent().map(|p| p.to_path_buf());
        }
        RecordLoad::Absent
    }
}

fn is_prefixed_sha256(s: &str) -> bool {
    s.strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)))
}

/// What an older `ods trud pull` wrote beside the archive, before `trud/datapackage.json`.
/// Nothing reads it; a repair removes it.
pub const LEGACY_PROVENANCE_FILENAME: &str = "_provenance.json";

/// The directory the TRUD archive package lives in: beside the archive it describes.
pub fn trud_archive_package_dir(release_dir: &Path) -> PathBuf {
    release_dir.join("trud")
}

/// Where `ods trud pull` writes its TRUD archive package.
pub fn trud_archive_package_path(release_dir: &Path) -> PathBuf {
    trud_archive_package_dir(release_dir).join(TRUD_ARCHIVE_PACKAGE_FILENAME)
}

#[derive(Debug, Clone)]
pub enum RecordLoad {
    Read(Box<TrudArchivePackage>, PathBuf),
    Unreadable { path: PathBuf, date: String },
    /// A `datapackage.json` that is some other Data Package (a release's view).
    NotARecord,
    Absent,
}

impl RecordLoad {
    pub fn ok(self) -> Option<TrudArchivePackage> {
        match self {
            Self::Read(record, _) => Some(*record),
            _ => None,
        }
    }

    pub fn error_building_with_path(self) -> Result<Option<(TrudArchivePackage, PathBuf)>> {
        match self {
            Self::Read(record, path) => Ok(Some((*record, path))),
            Self::Unreadable { path, date } => anyhow::bail!("{}", format_unreadable_record_error(&path, &date)),
            Self::NotARecord | Self::Absent => Ok(None),
        }
    }
}

fn date_for_unreadable_record(path: &Path) -> String {
    if let Some(d) = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("version").and_then(|d| d.as_str()).map(|d| d.to_string()))
        .filter(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").is_ok())
    {
        return d;
    }
    // `<release>/trud/datapackage.json`: the release directory is named by its date.
    for ancestor in path.ancestors().skip(1).take(2) {
        if let Some(name) = ancestor.file_name().and_then(|n| n.to_str()) {
            if chrono::NaiveDate::parse_from_str(name, "%Y-%m-%d").is_ok() {
                return name.to_string();
            }
        }
    }
    "<date>".to_string()
}

pub fn format_unreadable_record_error(path: &Path, date: &str) -> String {
    format!(
        "✖ {} isn't a TRUD archive package this ods can read\n  Expected $schema {} and name {}\n  Write it again with `ods trud pull {}`, then run `ods make`.",
        format_provenance_display_path(path),
        crate::datapackage::ODS_DATAPACKAGE_SCHEMA_URL,
        SOURCE_NAME,
        date
    )
}

// ---------------------------------------------------------------------------------------------
// Reading a release's provenance from its Parquet files
// ---------------------------------------------------------------------------------------------

/// The facts a release with provenance carries, read from its files' embedded object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseFacts {
    pub embedded: Embedded,
    /// The source release's version: TRUD's release date.
    pub release_date: String,
    /// The dataset version, parsed from after the `_` in `version`.
    pub dataset_version: String,
    /// `sources[0]`.
    pub source: Source,
}

impl ReleaseFacts {
    /// `version`: the OCI tag, `<source release>_<dataset version>`.
    pub fn version(&self) -> &str {
        &self.embedded.version
    }

    /// The source archive's SHA-256 as upper-case hex, as TRUD writes it. The release index
    /// holds the same hash as `source.hash`: `sha256:` and lower case.
    pub fn source_sha256_upper(&self) -> String {
        self.source.hash.trim_start_matches("sha256:").to_uppercase()
    }

    pub fn license(&self) -> &License {
        &self.embedded.licenses[0]
    }

    fn from_embedded(embedded: Embedded) -> std::result::Result<Self, String> {
        let version = embedded.version.clone();
        let Some(source) = embedded.sources.first().cloned() else {
            return Err(format!("version {} names no source", version));
        };
        if embedded.licenses.is_empty() {
            return Err("it names no licence".to_string());
        }
        let Some((release, dataset_version)) = version.split_once('_') else {
            return Err(format!("version {} isn't <source release>_<dataset version>", version));
        };
        if release != source.version {
            return Err(format!("version {} doesn't start with its source's version {}", version, source.version));
        }
        if dataset_version.is_empty() {
            return Err(format!("version {} has no dataset version after the _", version));
        }
        if !is_prefixed_sha256(&source.hash) {
            return Err(format!("its source's hash '{}' isn't sha256:<hex>", source.hash));
        }
        Ok(Self {
            release_date: source.version.clone(),
            dataset_version: dataset_version.to_string(),
            source,
            embedded,
        })
    }
}

/// What a release's Parquet files say it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleaseRecord {
    /// Every file carries the same embedded object, with a version and a source.
    Provenanced(Box<ReleaseFacts>),
    /// No file carries a `datapackage` key: built from an archive `ods` couldn't match to a
    /// source release (`ods make --force`), or by an `ods` older than the key. The files can't
    /// say which.
    NoProvenance,
    /// The directory holds no Parquet files.
    NoFiles,
}

impl ReleaseRecord {
    pub fn facts(&self) -> Option<&ReleaseFacts> {
        match self {
            Self::Provenanced(f) => Some(f),
            _ => None,
        }
    }
}

/// The top-level `*.parquet` files in `dir`, sorted by name: the files a release is.
pub fn release_parquet_files(dir: &Path) -> Result<Vec<String>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?.to_string();
            (path.is_file() && name.ends_with(".parquet")).then_some(name)
        })
        .collect();
    names.sort();
    Ok(names)
}

/// The raw `datapackage` value from one Parquet file's footer, or `None` when it has none.
/// Reads the footer only, never a data page.
pub fn read_embedded_value(path: &Path) -> Result<Option<String>> {
    use parquet::file::reader::{FileReader, SerializedFileReader};
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let reader = SerializedFileReader::new(file).with_context(|| format!("reading {}'s Parquet footer", path.display()))?;
    let kv = reader.metadata().file_metadata().key_value_metadata();
    Ok(kv
        .and_then(|kv| kv.iter().find(|item| item.key == DATAPACKAGE_KEY))
        .map(|item| item.value.clone().unwrap_or_default()))
}

/// Reads the embedded object from every Parquet file in `dir` and says what the release is.
/// Refuses, with a `✖` block naming the files, when a footer can't be read, when an object
/// isn't one this `ods` can read, or when the files don't all carry the same object.
pub fn read_release(dir: &Path) -> Result<ReleaseRecord> {
    let names = release_parquet_files(dir)?;
    if names.is_empty() {
        return Ok(ReleaseRecord::NoFiles);
    }
    let dir_display = crate::workspace::relative_to_cwd(dir).display().to_string();

    let mut values: Vec<(String, Option<String>)> = Vec::with_capacity(names.len());
    for name in &names {
        let value = read_embedded_value(&dir.join(name)).map_err(|e| {
            anyhow::anyhow!(
                "✖ {}/{} can't be read as a Parquet file\n  {:#}\n  Pull the release again with `ods pull --force`, or rebuild it with `ods make`.",
                dir_display,
                name,
                e
            )
        })?;
        values.push((name.clone(), value));
    }

    let first = &values[0].1;
    if values.iter().any(|(_, v)| v != first) {
        let width = names.iter().map(|n| n.len()).max().unwrap_or(0);
        let mut msg = format!("✖ The Parquet files in {} don't carry the same provenance", dir_display);
        for (name, value) in &values {
            msg.push_str(&format!("\n  {:<width$}  {}", name, describe_embedded_value(value.as_deref()), width = width));
        }
        msg.push_str("\n  A release's files all come from one build. Pull it again with `ods pull --force`, or rebuild it with `ods make`.");
        anyhow::bail!(msg);
    }

    let Some(raw) = first else {
        return Ok(ReleaseRecord::NoProvenance);
    };
    let unreadable = |why: String| {
        anyhow::anyhow!(
            "✖ The Parquet files in {} carry provenance this ods can't read\n  {}: {}\n  Pull the release again with `ods pull --force`, or rebuild it with `ods make`.",
            dir_display,
            DATAPACKAGE_KEY,
            why
        )
    };
    let embedded: Embedded = serde_json::from_str(raw).map_err(|e| unreadable(e.to_string()))?;
    ReleaseFacts::from_embedded(embedded)
        .map(|facts| ReleaseRecord::Provenanced(Box::new(facts)))
        .map_err(unreadable)
}

/// One file's line in the disagreement block: its version, and a short fingerprint of the whole
/// value so two objects with one version still tell apart.
fn describe_embedded_value(value: Option<&str>) -> String {
    let Some(value) = value else {
        return "no datapackage metadata".to_string();
    };
    let fingerprint = format!("{:x}", Sha256::digest(value.as_bytes()));
    let version = serde_json::from_str::<serde_json::Value>(value)
        .ok()
        .map(|v| v.get("version").and_then(|v| v.as_str()).unwrap_or("no version").to_string())
        .unwrap_or_else(|| "unreadable".to_string());
    format!("{}  (datapackage sha256:{}…)", version, &fingerprint[..12])
}

// ---------------------------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------------------------

/// What a release without provenance is, as far as its files can say: they carry no
/// `datapackage` key, which a `--force` build and an older `ods`'s build share.
pub fn describe_no_provenance(dir: &Path) -> String {
    format!(
        "{}'s Parquet files carry no provenance: it was built from an archive ods couldn't match to a TRUD release, or by an older ods",
        crate::workspace::relative_to_cwd(dir).display()
    )
}

pub fn format_no_provenance_error(dir: &Path) -> String {
    format!(
        "✖ {}\n  To cite or publish it, get the archive through ods trud pull and build it with ods make, or pull the release with ods pull.",
        describe_no_provenance(dir)
    )
}

/// The `✖` block for a release that can't be cited, published or verified because its files
/// carry no provenance, whichever way.
pub fn format_record_refusal(dir: &Path, record: &ReleaseRecord) -> Option<String> {
    match record {
        ReleaseRecord::Provenanced(_) => None,
        ReleaseRecord::NoProvenance => Some(format_no_provenance_error(dir)),
        ReleaseRecord::NoFiles => Some(format!(
            "✖ No Parquet files found in {}",
            crate::workspace::relative_to_cwd(dir).display()
        )),
    }
}

pub fn format_older_terms_error(record_path: &Path, date: &str) -> String {
    let disp = format_provenance_display_path(record_path);
    format!(
        "✖ {} has older licence terms than this ods\n  Refresh it without downloading the archive: ods trud pull {} --force",
        disp, date
    )
}

pub fn truncate_sha256_for_display(sha: &str) -> String {
    let sha_upper = sha.to_uppercase();
    if sha_upper.len() >= 12 {
        format!("{}…{}", &sha_upper[..8], &sha_upper[sha_upper.len() - 4..])
    } else {
        sha_upper
    }
}

pub fn format_unmatched_archive_warning(filename: &str, sha: &str) -> String {
    let trunc_sha = truncate_sha256_for_display(sha);
    format!(
        "! {} isn't a TRUD release ods knows (SHA-256 {})\n  Built without provenance. You can explore it with find, info and role, but not cite or publish it.",
        filename, trunc_sha
    )
}

pub fn format_unmatched_archive_refusal(filename: &str, sha: &str) -> String {
    let trunc_sha = truncate_sha256_for_display(sha);
    format!(
        "✖ {} isn't a TRUD release ods knows (SHA-256 {})\n  Build it outside the workspace with -o <dir>, or run ods pull for a newer release index.",
        filename, trunc_sha
    )
}

/// `ods make` on an unmatched archive, without `--force`: refused whether or not `-o <dir>` was
/// given, since building it without provenance is now a deliberate act.
pub fn format_make_unmatched_refusal(filename: &str, sha: &str) -> String {
    let trunc_sha = truncate_sha256_for_display(sha);
    format!(
        "✖ {} isn't a TRUD release ods knows (SHA-256 {})\n  Get it through ods trud pull, or run ods pull for a newer release index.\n  To build it anyway, without provenance: ods make -i {} -o <dir> --force",
        filename, trunc_sha, filename
    )
}

/// `ods make --force` without `-o <dir>`: `--force` alone doesn't put an unprovenanced release
/// into a workspace.
pub fn format_make_force_needs_output() -> String {
    "✖ --force builds outside the workspace only\n  Add -o <dir>: a workspace holds only releases with provenance.".to_string()
}

// ---------------------------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TrudZipFacts {
    pub zip_path: PathBuf,
    pub sha256: String,
    pub filesize_bytes: u64,
}

pub fn try_extract_trud_zip_facts(input_path: &Path) -> Option<TrudZipFacts> {
    let find_zip = |dir: &Path| -> Option<std::path::PathBuf> {
        if dir.is_file() && dir.extension().is_some_and(|ext| ext == "zip") {
            return Some(dir.to_path_buf());
        }
        if dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_file() && p.extension().is_some_and(|ext| ext == "zip") {
                        return Some(p);
                    }
                }
            }
            let trud_sub = dir.join("trud");
            if trud_sub.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&trud_sub) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if p.is_file() && p.extension().is_some_and(|ext| ext == "zip") {
                            return Some(p);
                        }
                    }
                }
            }
        }
        None
    };

    let zip_path = find_zip(input_path)?;
    let meta = std::fs::metadata(&zip_path).ok()?;
    let sha256 = compute_file_sha256(&zip_path).ok()?;
    Some(TrudZipFacts {
        zip_path,
        sha256,
        filesize_bytes: meta.len(),
    })
}

pub fn compute_file_sha256(path: &Path) -> Result<String> {
    let file = File::open(path).with_context(|| format!("opening file for sha256: {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    std::io::copy(&mut reader, &mut hasher)?;
    Ok(format!("{:X}", hasher.finalize()))
}

/// Scans `trud_dir` for `.zip` files and finds the one matching `expected_sha` (case-insensitive SHA-256).
/// Returns `(matching_zip_path, all_zip_paths)`.
pub fn find_archive_by_sha(trud_dir: &Path, expected_sha: &str) -> (Option<PathBuf>, Vec<PathBuf>) {
    let mut zip_paths = Vec::new();
    let mut matched = None;
    if trud_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(trud_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.extension().is_some_and(|e| e == "zip") {
                    if matched.is_none() {
                        if let Ok(actual_sha) = compute_file_sha256(&path) {
                            if actual_sha.eq_ignore_ascii_case(expected_sha) {
                                matched = Some(path.clone());
                            }
                        }
                    }
                    zip_paths.push(path);
                }
            }
        }
    }
    zip_paths.sort();
    (matched, zip_paths)
}

pub fn sanitize_trud_url(url: &str, api_key: Option<&str>) -> String {
    if let Some(key) = api_key {
        if !key.is_empty() {
            return url.replace(key, "<REDACTED_API_KEY>");
        }
    }
    url.to_string()
}

/// Formats a provenance path relative to the workspace root if one is found,
/// or as given otherwise.
pub fn format_provenance_display_path(prov_path: &Path) -> String {
    let ws_root = prov_path
        .parent()
        .and_then(|p| crate::workspace::find_workspace_root_from(p, None).ok().flatten())
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .and_then(|pwd| crate::workspace::find_workspace_root_from(&pwd, None).ok().flatten())
        });

    if let Some(ref root) = ws_root {
        if let Ok(rel) = prov_path.strip_prefix(root) {
            return rel.display().to_string();
        }
        if let (Ok(can_prov), Ok(can_root)) = (prov_path.canonicalize(), root.canonicalize()) {
            if let Ok(rel) = can_prov.strip_prefix(&can_root) {
                return rel.display().to_string();
            }
        }
    }
    prov_path.display().to_string()
}

/// Writes the TRUD archive package for a TRUD release into `release_dir/trud/`: the archive from TRUD's
/// word, plus NHS's other files already on disk (`(name, mediatype, path)`).
pub fn write_trud_archive_package(
    release_dir: &Path,
    date: &str,
    archive_file_name: &str,
    archive_sha256: &str,
    archive_bytes: u64,
    other_files: &[(&str, &str, PathBuf)],
) -> Result<TrudArchivePackage> {
    let record = TrudArchivePackage::for_trud_release(date, archive_file_name, archive_sha256, archive_bytes, other_files)?;
    record.write_to_dir(release_dir)?;
    Ok(record)
}

/// The embedded object for a fixture release of `date` built from an archive with the given
/// SHA-256 and size, at this build's dataset version: what `ods make` would embed after `ods
/// trud pull`. For tests.
#[doc(hidden)]
pub fn fixture_embedded_for(date: &str, archive_sha256: &str, archive_bytes: u64) -> Embedded {
    TrudArchivePackage::for_trud_release(
        date,
        &format!("hscorgrefdataxml_data_7.0.0_{}000001.zip", date.replace('-', "")),
        archive_sha256,
        archive_bytes,
        &[],
    )
    .and_then(|record| record.embedded(crate::datapackage::DATASET_VERSION))
    .expect("a fixture record always derives an embedded object")
}

/// `fixture_embedded_for` with a fixed archive hash and size. For tests.
#[doc(hidden)]
pub fn fixture_embedded(date: &str) -> Embedded {
    fixture_embedded_for(date, "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933", 38_064_419)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> TrudArchivePackage {
        TrudArchivePackage::for_trud_release(
            "2026-09-25",
            "hscorgrefdataxml_data_8.0.0_20260925000001.zip",
            "CA0FEE7512F593ADA1FA9B95BF1372B41911167DA463A98FECF33ADFD86697E5",
            38138574,
            &[],
        )
        .unwrap()
    }

    #[test]
    fn test_embedded_object_is_the_briefs_shape_in_its_key_order() {
        let embedded = record().embedded("0.1.0").unwrap();
        let json = embedded.to_compact_json().unwrap();
        assert_eq!(
            json,
            concat!(
                r#"{"name":"ods-data","version":"2026-09-25_0.1.0","#,
                r#""licenses":[{"name":"OGL-UK-3.0","path":"https://www.nationalarchives.gov.uk/doc/open-government-licence/version/3/","title":"Open Government Licence v3.0","attribution":"Contains information from NHS England, licensed under the current version of the Open Government Licence."}],"#,
                r#""contributors":[{"title":"NHS England","roles":["rightsHolder"]},{"title":"NHS TRUD","roles":["distributor"]}],"#,
                r#""sources":[{"title":"NHS Organisation Data Service XML Data","version":"2026-09-25","path":"https://isd.digital.nhs.uk/trud/users/guest/filters/0/categories/5/items/341/releases","hash":"sha256:ca0fee7512f593ada1fa9b95bf1372b41911167da463a98fecf33adfd86697e5","bytes":38138574}]}"#
            )
        );
    }

    #[test]
    fn test_facts_refuse_a_version_that_disagrees_with_its_source() {
        let mut embedded = record().embedded("0.1.0").unwrap();
        embedded.version = "2026-09-24_0.1.0".to_string();
        let err = ReleaseFacts::from_embedded(embedded).unwrap_err();
        assert!(err.contains("doesn't start with its source's version"), "{err}");
    }

    #[test]
    fn test_trud_archive_package_loader_passes_over_a_releases_view() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join(TRUD_ARCHIVE_PACKAGE_FILENAME), r#"{"name":"ods-data"}"#).unwrap();
        assert!(matches!(TrudArchivePackage::load_from_dir(tmp.path()), RecordLoad::Absent));

        record().write_to_dir(tmp.path()).unwrap();
        let loaded = TrudArchivePackage::load_from_dir(tmp.path()).ok().expect("the record under trud/ is read");
        assert_eq!(loaded, record());
    }

    #[test]
    fn test_sanitize_trud_url() {
        let url = "https://isd.digital.nhs.uk/trud/api/v1/keys/SECRET123/items/341";
        let sanitized = sanitize_trud_url(url, Some("SECRET123"));
        assert_eq!(sanitized, "https://isd.digital.nhs.uk/trud/api/v1/keys/<REDACTED_API_KEY>/items/341");
    }

    #[test]
    fn test_find_archive_by_sha() {
        let tmp = tempfile::TempDir::new().unwrap();
        let trud_dir = tmp.path().join("trud");
        std::fs::create_dir(&trud_dir).unwrap();

        let zip1 = trud_dir.join("release_1.zip");
        let zip2 = trud_dir.join("release_2.zip");
        std::fs::write(&zip1, b"first-archive-content").unwrap();
        std::fs::write(&zip2, b"second-archive-content").unwrap();

        let sha1 = compute_file_sha256(&zip1).unwrap();
        let (matched, all) = find_archive_by_sha(&trud_dir, &sha1);
        assert_eq!(matched, Some(zip1.clone()));
        assert_eq!(all.len(), 2);

        // Case insensitivity
        let (matched_lower, _) = find_archive_by_sha(&trud_dir, &sha1.to_lowercase());
        assert_eq!(matched_lower, Some(zip1));

        // Unknown sha
        let (matched_none, all) = find_archive_by_sha(&trud_dir, "0000000000000000000000000000000000000000000000000000000000000000");
        assert_eq!(matched_none, None);
        assert_eq!(all.len(), 2);

        // Empty dir
        let empty_dir = tmp.path().join("empty");
        std::fs::create_dir(&empty_dir).unwrap();
        let (matched_empty, all_empty) = find_archive_by_sha(&empty_dir, &sha1);
        assert_eq!(matched_empty, None);
        assert!(all_empty.is_empty());
    }
}

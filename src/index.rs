use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const RELEASES_SCHEMA_V1_URL: &str = "https://ods.fyi/schema/releases.v1.json";
pub const RELEASES_JSON: &str = include_str!("../data/releases.json");
pub const BAKED_RELEASES_JSON_BYTES: &[u8] = include_bytes!("../data/releases.json");
pub const RELEASES_JSON_FILENAME: &str = "_releases.json";

/// Where each known source issue's page lives: `<base><id>.md`. The index holds only the IDs, so
/// no location is frozen into it.
pub const SOURCE_ISSUES_BASE_URL: &str = "https://github.com/olizilla/ods/blob/main/docs/source-issues/";

/// The page for a known source issue.
pub fn source_issue_url(id: &str) -> String {
    format!("{SOURCE_ISSUES_BASE_URL}{id}.md")
}

/// The line `ods pull` names a known source issue with.
pub fn source_issue_line(id: &str) -> String {
    format!("* known source issue: {}  {}", id, source_issue_url(id))
}

/// The release index: the source releases one dataset family is built from, and the datasets
/// built from each. The names are the Data Package vocabulary the Parquet metadata and the
/// `datapackage.json` view use, so one fact has one name wherever it appears.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OdsReleaseIndex {
    #[serde(rename = "$schema")]
    pub schema: String,
    /// The dataset family this index lists: the Parquet metadata's `name`, and the OCI repository.
    pub name: String,
    /// The source family every release comes from.
    pub source: IndexSource,
    pub mirrors: Vec<MirrorEntry>,
    pub releases: Vec<Release>,
}

impl Default for OdsReleaseIndex {
    fn default() -> Self {
        Self {
            schema: RELEASES_SCHEMA_V1_URL.to_string(),
            name: crate::datapackage::NAME.to_string(),
            source: IndexSource {
                title: crate::provenance::SOURCE_TITLE.to_string(),
                path: crate::terms::LANDING_PAGE.to_string(),
                signing_key_fingerprints: None,
            },
            mirrors: vec![
                MirrorEntry {
                    url: "https://ods.fyi/v2/ods-data".to_string(),
                },
                MirrorEntry {
                    url: "https://ghcr.io/v2/olizilla/ods-data".to_string(),
                },
            ],
            releases: Vec::new(),
        }
    }
}

/// The source family, constant across releases: its title and landing page as the view's
/// `sources[0]` has them, and the publisher's signing keys when it signs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexSource {
    pub title: String,
    pub path: String,
    /// Upper-case hex fingerprints of the keys the source publisher signs with, oldest first.
    /// A source that signs nothing omits it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing_key_fingerprints: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MirrorEntry {
    pub url: String,
}

impl MirrorEntry {
    pub fn blob_url(&self, digest: &str) -> String {
        format!("{}/blobs/{}", self.url.trim_end_matches('/'), digest)
    }

    pub fn manifest_url(&self, tag_or_digest: &str) -> String {
        format!("{}/manifests/{}", self.url.trim_end_matches('/'), tag_or_digest)
    }

    pub fn host(&self) -> String {
        let trimmed = self.url.trim_start_matches("https://").trim_start_matches("http://");
        let host_part = trimmed.split('/').next().unwrap_or(trimmed);
        host_part.split(':').next().unwrap_or(host_part).to_string()
    }
}

/// One source release, identified by its source version, and the datasets built from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub source: SourceRelease,
    pub datasets: Vec<Dataset>,
}

/// What the source published: its version (for TRUD, the release date), the archive's hash and
/// size, and any known issues with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRelease {
    pub version: String,
    /// `sha256:` and lower-case hex, as the Parquet metadata and the view write it.
    pub hash: String,
    pub bytes: u64,
    /// IDs of known issues with this source release, each a page under `docs/source-issues/`.
    /// May appear on a row later; never removed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<String>,
}

impl SourceRelease {
    /// The hash's hex, without its `sha256:` prefix, in lower case.
    pub fn sha256_hex(&self) -> &str {
        self.hash.trim_start_matches("sha256:")
    }

    /// Whether `hex` (any case, with or without `sha256:`) is this release's hash.
    pub fn hash_matches(&self, hex: &str) -> bool {
        self.sha256_hex().eq_ignore_ascii_case(hex.trim_start_matches("sha256:"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dataset {
    /// `<source version>_<dataset version>`: the Parquet metadata's `version` and the OCI tag.
    pub version: String,
    pub manifest_digest: String,
    /// The sum of the manifest's layer sizes: what `ods pull` downloads.
    pub bytes: u64,
    /// The ods that built this dataset: the version of its tag, without the `v`. Absent from the
    /// JSON reads as empty, so `validate` names the problem instead of the parser hiding it.
    #[serde(default)]
    pub tool_version: String,
    /// The commit that tag points at, 40 lower-case hex characters.
    #[serde(default)]
    pub tool_git_sha: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doi: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub withdrawn: Option<String>,
}

impl Dataset {
    pub fn is_withdrawn(&self) -> bool {
        self.withdrawn.is_some()
    }

    /// The dataset version: the SemVer after the `_` in `version`, which says which schema the
    /// files have (docs/tests.md R8).
    pub fn dataset_version(&self) -> &str {
        self.version.split_once('_').map(|(_, v)| v).unwrap_or(&self.version)
    }

    /// The dataset version parsed as SemVer, `(0, 0, 0)` when it doesn't parse, for ordering.
    pub fn semver(&self) -> (u64, u64, u64) {
        parse_semver(self.dataset_version()).unwrap_or((0, 0, 0))
    }
}

/// Sorts a release's datasets oldest dataset version first, the order the index keeps them in.
pub fn sort_datasets(datasets: &mut [Dataset]) {
    datasets.sort_by_key(|d| d.semver());
}

pub fn parse_semver(v: &str) -> Result<(u64, u64, u64)> {
    let parts: Vec<&str> = v.split('.').collect();
    if parts.len() != 3 {
        bail!("Invalid semver '{v}': expected 3 dot-separated parts");
    }
    let major: u64 = parts[0]
        .parse()
        .with_context(|| format!("Invalid major version in '{v}'"))?;
    let minor: u64 = parts[1]
        .parse()
        .with_context(|| format!("Invalid minor version in '{v}'"))?;
    let patch: u64 = parts[2]
        .parse()
        .with_context(|| format!("Invalid patch version in '{v}'"))?;
    Ok((major, minor, patch))
}

fn is_sha256_digest(s: &str) -> bool {
    s.strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')))
}

/// A lower-case slug: `t1201-recorded-twice`, `ods-data`.
fn is_slug(s: &str) -> bool {
    !s.is_empty()
        && s.split('-')
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()))
}

fn is_yyyy_mm_dd(s: &str) -> bool {
    s.len() == 10
        && s.chars().enumerate().all(|(idx, c)| {
            if idx == 4 || idx == 7 {
                c == '-'
            } else {
                c.is_ascii_digit()
            }
        })
        && chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

#[derive(Debug)]
pub struct SecurityError(pub String);

impl std::fmt::Display for SecurityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Security error: {}", self.0)
    }
}

impl std::error::Error for SecurityError {}

/// A release index in the shape used before publication: the same `$schema`, with TRUD-named
/// fields (`trud_release_date`, `dataset_version`, …). This ods reads only the current shape.
#[derive(Debug)]
pub struct OldFormatIndex;

impl std::fmt::Display for OldFormatIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "it's a release index in the old format, with trud_release_date and dataset_version")
    }
}

impl std::error::Error for OldFormatIndex {}

/// Whether `bytes` hold a release index in the old format: TRUD-named fields at the top level
/// or on any release row.
pub fn is_old_format(bytes: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return false;
    };
    value.get("trud_signing_key_fingerprints").is_some()
        || value
            .get("releases")
            .and_then(|r| r.as_array())
            .is_some_and(|rows| rows.iter().any(|r| r.get("trud_release_date").is_some()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRelease<'a> {
    pub release: &'a Release,
    pub dataset: &'a Dataset,
    pub skipped: Vec<(&'a Release, &'a Dataset)>,
}

impl OdsReleaseIndex {
    /// Parses an index, without validating it. An index in the old format fails with
    /// `OldFormatIndex`, so a caller can say so instead of reporting a missing field.
    pub fn from_slice(bytes: &[u8]) -> Result<Self> {
        if is_old_format(bytes) {
            return Err(OldFormatIndex.into());
        }
        Ok(serde_json::from_slice(bytes)?)
    }

    /// Loads and validates the release index from a workspace directory.
    pub fn load_from_workspace(workspace_root: &std::path::Path) -> Result<Option<Self>> {
        let path = workspace_root.join(RELEASES_JSON_FILENAME);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = std::fs::read(&path)
            .with_context(|| format!("reading index at {}", path.display()))?;
        let index = Self::from_slice(&bytes)
            .with_context(|| format!("parsing index at {}", path.display()))?;
        index.validate()?;
        Ok(Some(index))
    }

    /// Saves raw index bytes verbatim to `_releases.json` in the workspace directory.
    pub fn save_to_workspace_bytes(bytes: &[u8], workspace_root: &std::path::Path) -> Result<()> {
        let path = workspace_root.join(RELEASES_JSON_FILENAME);
        std::fs::write(&path, bytes)
            .with_context(|| format!("writing index to {}", path.display()))?;
        Ok(())
    }

    /// Loads and validates the checked-in baked release index.
    pub fn baked() -> Result<Self> {
        let index = Self::from_slice(BAKED_RELEASES_JSON_BYTES)
            .context("Failed to parse baked data/releases.json")?;
        index.validate()?;
        Ok(index)
    }

    /// Serialises the index deterministically with two-space indentation and a trailing newline.
    pub fn to_json_pretty(&self) -> Result<String> {
        let mut s = serde_json::to_string_pretty(self)?;
        s.push('\n');
        Ok(s)
    }

    /// The release row for a source version.
    pub fn release(&self, source_version: &str) -> Option<&Release> {
        self.releases.iter().find(|r| r.source.version == source_version)
    }

    /// The dataset row for a dataset `version` (`<source version>_<dataset version>`).
    pub fn dataset(&self, version: &str) -> Option<(&Release, &Dataset)> {
        let (source_version, _) = version.split_once('_')?;
        let release = self.release(source_version)?;
        release.datasets.iter().find(|d| d.version == version).map(|d| (release, d))
    }

    /// Validates the release index against structural invariants:
    /// - $schema is exactly https://ods.fyi/schema/releases.v1.json
    /// - name is a lower-case slug; source.title and source.path are present
    /// - source.signing_key_fingerprints, when present, has at least one entry, each 40
    ///   upper-case hex characters, no duplicates
    /// - each source.version is YYYY-MM-DD, unique, and the list runs newest first
    /// - each source.hash is sha256: plus 64 lower-case hex characters, bytes > 0
    /// - each source issue is a lower-case slug, no duplicates
    /// - each dataset version is `<source.version>_<SemVer>`, unique, datasets run oldest first
    /// - each manifest_digest is sha256: plus 64 lower-case hex characters, bytes > 0
    /// - each tool_version parses with parse_semver and each tool_git_sha is 40 lower-case hex characters
    pub fn validate(&self) -> Result<()> {
        if self.schema != RELEASES_SCHEMA_V1_URL {
            bail!(
                "Invalid $schema: expected '{}', got '{}'",
                RELEASES_SCHEMA_V1_URL,
                self.schema
            );
        }

        if !is_slug(&self.name) {
            bail!("Invalid name: expected a lower-case name such as ods-data, got '{}'", self.name);
        }
        if self.source.title.is_empty() {
            bail!("Invalid source.title: expected the source's title, got an empty string");
        }
        if self.source.path.is_empty() {
            bail!("Invalid source.path: expected the source's landing page, got an empty string");
        }

        if let Some(fingerprints) = &self.source.signing_key_fingerprints {
            if fingerprints.is_empty() {
                bail!("Invalid source.signing_key_fingerprints: expected at least one entry, got empty list (a source that signs nothing omits it)");
            }
            let mut seen_fingerprints = HashSet::new();
            for fp in fingerprints {
                if fp.len() != 40 || !fp.chars().all(|c| matches!(c, '0'..='9' | 'A'..='F')) {
                    bail!(
                        "Invalid source.signing_key_fingerprints: expected 40 upper-case hex characters, got '{}'",
                        fp
                    );
                }
                if !seen_fingerprints.insert(fp) {
                    bail!("Duplicate source.signing_key_fingerprints in index: '{}'", fp);
                }
            }
        }

        let mut seen_versions_of_source = HashSet::new();
        for (i, rel) in self.releases.iter().enumerate() {
            let src = &rel.source;
            if !is_yyyy_mm_dd(&src.version) {
                bail!(
                    "Invalid source.version: expected a YYYY-MM-DD date, got '{}'",
                    src.version
                );
            }

            if !seen_versions_of_source.insert(&src.version) {
                bail!("Duplicate source.version in index: '{}'", src.version);
            }

            if i > 0 && src.version >= self.releases[i - 1].source.version {
                bail!(
                    "Releases must run newest first: '{}' is not newer than '{}'",
                    self.releases[i - 1].source.version,
                    src.version
                );
            }

            if !is_sha256_digest(&src.hash) {
                bail!(
                    "Invalid source.hash for release '{}': expected 'sha256:' followed by 64 lower-case hex characters, got '{}'",
                    src.version,
                    src.hash
                );
            }

            if src.bytes == 0 {
                bail!(
                    "Invalid source.bytes for release '{}': must be greater than zero, got 0",
                    src.version
                );
            }

            let mut seen_issues = HashSet::new();
            for issue in &src.issues {
                if !is_slug(issue) {
                    bail!(
                        "Invalid source.issues for release '{}': expected an issue ID such as t1201-recorded-twice, got '{}'",
                        src.version,
                        issue
                    );
                }
                if !seen_issues.insert(issue) {
                    bail!("Duplicate source.issues for release '{}': '{}'", src.version, issue);
                }
            }

            let mut seen_versions = HashSet::new();
            let mut prev_semver: Option<(u64, u64, u64)> = None;

            for (j, ds) in rel.datasets.iter().enumerate() {
                let semver_part = ds
                    .version
                    .strip_prefix(src.version.as_str())
                    .and_then(|rest| rest.strip_prefix('_'));
                let Some(semver_part) = semver_part else {
                    bail!(
                        "Invalid dataset version '{}' for release '{}': expected {}_<dataset version>, e.g. {}_0.1.0",
                        ds.version,
                        src.version,
                        src.version,
                        src.version
                    );
                };
                let sem = parse_semver(semver_part).with_context(|| {
                    format!(
                        "Invalid dataset version '{}' for release '{}': the part after the _ isn't SemVer",
                        ds.version, src.version
                    )
                })?;

                if !seen_versions.insert(&ds.version) {
                    bail!(
                        "Duplicate dataset version '{}' for release '{}'",
                        ds.version,
                        src.version
                    );
                }

                if let Some(prev) = prev_semver {
                    if sem <= prev {
                        bail!(
                            "Datasets must run oldest version first: '{}' is not older than '{}' for release '{}'",
                            rel.datasets[j - 1].version,
                            ds.version,
                            src.version
                        );
                    }
                }
                prev_semver = Some(sem);

                if !is_sha256_digest(&ds.manifest_digest) {
                    bail!(
                        "Invalid manifest_digest: must be 'sha256:' followed by 64 lower-case hex characters, got '{}'",
                        ds.manifest_digest
                    );
                }

                if ds.bytes == 0 {
                    bail!(
                        "Invalid bytes for dataset {}: must be greater than zero, got 0",
                        ds.version
                    );
                }

                if ds.tool_version.is_empty() {
                    bail!(
                        "Missing tool_version for dataset {}: each dataset row records the ods that built it",
                        ds.version
                    );
                }
                parse_semver(&ds.tool_version).with_context(|| {
                    format!(
                        "Invalid tool_version '{}' for dataset {}: expected the tool's tag without its 'v', e.g. 0.2.0",
                        ds.tool_version, ds.version
                    )
                })?;
                if ds.tool_git_sha.is_empty() {
                    bail!(
                        "Missing tool_git_sha for dataset {}: each dataset row records the commit of the ods that built it",
                        ds.version
                    );
                }
                if ds.tool_git_sha.len() != 40
                    || !ds.tool_git_sha.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
                {
                    bail!(
                        "Invalid tool_git_sha for dataset {}: expected 40 lower-case hex characters, got '{}'",
                        ds.version,
                        ds.tool_git_sha
                    );
                }
            }
        }

        Ok(())
    }

    /// Merges a candidate index into self (the existing index).
    ///
    /// Rules:
    /// - Both name the same dataset family.
    /// - Release in both: source hash and bytes must match (SecurityError). Candidate copy may add
    ///   source issues; none is ever removed.
    /// - Dataset in both: manifest_digest, bytes, tool_version and tool_git_sha must match
    ///   (SecurityError). Candidate copy may add withdrawn and doi.
    /// - Additions: candidate index may add releases and datasets.
    /// - Mirrors, and the source's title, path and signing keys: the candidate's when it names them.
    pub fn merge(&self, fetched: &OdsReleaseIndex) -> Result<OdsReleaseIndex> {
        self.validate().context("validating existing index")?;
        fetched.validate().context("validating candidate index")?;

        if self.name != fetched.name {
            bail!(
                "the candidate index lists {}, not {}",
                fetched.name,
                self.name
            );
        }

        for baked_rel in &self.releases {
            let version = &baked_rel.source.version;
            if let Some(fetched_rel) = fetched.release(version) {
                if baked_rel.source.hash != fetched_rel.source.hash {
                    return Err(SecurityError(format!(
                        "fetched index contradicts baked release {}: baked source hash {} != fetched source hash {}",
                        version, baked_rel.source.hash, fetched_rel.source.hash
                    ))
                    .into());
                }
                if baked_rel.source.bytes != fetched_rel.source.bytes {
                    return Err(SecurityError(format!(
                        "fetched index contradicts baked release {}: baked size {} != fetched size {}",
                        version, baked_rel.source.bytes, fetched_rel.source.bytes
                    ))
                    .into());
                }
                for baked_ds in &baked_rel.datasets {
                    if let Some(fetched_ds) =
                        fetched_rel.datasets.iter().find(|d| d.version == baked_ds.version)
                    {
                        if baked_ds.manifest_digest != fetched_ds.manifest_digest {
                            return Err(SecurityError(format!(
                                "fetched index contradicts baked dataset {}: baked digest {} != fetched digest {}",
                                baked_ds.version, baked_ds.manifest_digest, fetched_ds.manifest_digest
                            ))
                            .into());
                        }
                        if baked_ds.bytes != fetched_ds.bytes {
                            return Err(SecurityError(format!(
                                "fetched index contradicts baked dataset {}: baked size {} != fetched size {}",
                                baked_ds.version, baked_ds.bytes, fetched_ds.bytes
                            ))
                            .into());
                        }
                        if baked_ds.tool_version != fetched_ds.tool_version {
                            return Err(SecurityError(format!(
                                "fetched index contradicts baked dataset {}: baked tool_version {} != fetched tool_version {}",
                                baked_ds.version, baked_ds.tool_version, fetched_ds.tool_version
                            ))
                            .into());
                        }
                        if baked_ds.tool_git_sha != fetched_ds.tool_git_sha {
                            return Err(SecurityError(format!(
                                "fetched index contradicts baked dataset {}: baked tool_git_sha {} != fetched tool_git_sha {}",
                                baked_ds.version, baked_ds.tool_git_sha, fetched_ds.tool_git_sha
                            ))
                            .into());
                        }
                    }
                }
            }
        }

        let mut merged_releases = Vec::new();
        for baked_rel in &self.releases {
            let mut r = baked_rel.clone();
            if let Some(fetched_rel) = fetched.release(&baked_rel.source.version) {
                for issue in &fetched_rel.source.issues {
                    if !r.source.issues.contains(issue) {
                        r.source.issues.push(issue.clone());
                    }
                }
                for ds in &mut r.datasets {
                    if let Some(fetched_ds) =
                        fetched_rel.datasets.iter().find(|fd| fd.version == ds.version)
                    {
                        if ds.withdrawn.is_none() && fetched_ds.withdrawn.is_some() {
                            ds.withdrawn = fetched_ds.withdrawn.clone();
                        }
                        if ds.doi.is_none() && fetched_ds.doi.is_some() {
                            ds.doi = fetched_ds.doi.clone();
                        }
                    }
                }
                for fetched_ds in &fetched_rel.datasets {
                    if !r.datasets.iter().any(|d| d.version == fetched_ds.version) {
                        r.datasets.push(fetched_ds.clone());
                    }
                }
                sort_datasets(&mut r.datasets);
            }
            merged_releases.push(r);
        }

        for fetched_rel in &fetched.releases {
            if self.release(&fetched_rel.source.version).is_none() {
                merged_releases.push(fetched_rel.clone());
            }
        }

        merged_releases.sort_by(|a, b| b.source.version.cmp(&a.source.version));

        let mirrors = if !fetched.mirrors.is_empty() {
            fetched.mirrors.clone()
        } else {
            self.mirrors.clone()
        };

        let mut source = fetched.source.clone();
        if source.signing_key_fingerprints.is_none() {
            source.signing_key_fingerprints = self.source.signing_key_fingerprints.clone();
        }

        Ok(OdsReleaseIndex {
            schema: self.schema.clone(),
            name: self.name.clone(),
            source,
            mirrors,
            releases: merged_releases,
        })
    }

    /// Resolves the release and dataset to install:
    /// - requested source version, or the newest release with a non-withdrawn dataset
    /// - within it, the highest dataset version
    /// - a requested release with no datasets is refused, saying so: the index records it (for a
    ///   known source issue, say) but has published nothing built from it.
    pub fn resolve(&self, requested_date: Option<&str>) -> Result<ResolvedRelease<'_>> {
        match requested_date {
            Some(date) => {
                let release = self.release(date).ok_or_else(|| {
                    anyhow::anyhow!("Release date '{}' is not known to the release index", date)
                })?;
                let (dataset, _) = select_dataset(release).ok_or_else(|| no_dataset_yet(release))?;
                Ok(ResolvedRelease {
                    release,
                    dataset,
                    skipped: Vec::new(),
                })
            }
            None => {
                let mut skipped = Vec::new();
                let mut had_any_datasets = false;

                for release in &self.releases {
                    let Some((dataset, is_withdrawn)) = select_dataset(release) else {
                        continue;
                    };
                    had_any_datasets = true;

                    if is_withdrawn {
                        skipped.push((release, dataset));
                    } else {
                        return Ok(ResolvedRelease {
                            release,
                            dataset,
                            skipped,
                        });
                    }
                }

                if had_any_datasets {
                    bail!("Every release in the index is withdrawn\n  See them: ods pull --list");
                } else {
                    bail!("Release index contains no releases");
                }
            }
        }
    }
}

/// The refusal for a release the index records with no dataset built from it yet: its known
/// source issues, and how to build it from TRUD's archive.
fn no_dataset_yet(release: &Release) -> anyhow::Error {
    let version = &release.source.version;
    let mut message = format!("✖ {version} is in the release index, but no dataset has been published for it yet");
    for id in &release.source.issues {
        message.push_str(&format!("\n  {}", source_issue_line(id)));
    }
    message.push_str(&format!("\n  Build it yourself: ods trud pull {version} && ods make"));
    anyhow::anyhow!(message)
}

/// Selects the preferred dataset for a release:
/// - the highest dataset version not withdrawn, if any, returning `Some((dataset, false))`
/// - otherwise the highest withdrawn one, returning `Some((dataset, true))`
/// - `None` if the release has no datasets
pub fn select_dataset(release: &Release) -> Option<(&Dataset, bool)> {
    if release.datasets.is_empty() {
        return None;
    }

    if let Some(ds) = release
        .datasets
        .iter()
        .filter(|d| !d.is_withdrawn())
        .max_by_key(|d| d.semver())
    {
        Some((ds, false))
    } else {
        let ds = release
            .datasets
            .iter()
            .max_by_key(|d| d.semver())
            .expect("datasets is non-empty");
        Some((ds, true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA_A: &str = "sha256:abdd194b1569d5ff3cdd81d618847f05642bd43c5b15d6cd43d8289b7466d801";
    const SHA_B: &str = "sha256:8151248d1569d5ff3cdd81d618847f05642bd43c5b15d6cd43d8289b7466d801";
    const SHA_NEW: &str = "sha256:9999999999999999999999999999999999999999999999999999999999999999";
    const GIT_SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    fn dataset(version: &str, digest: &str, bytes: u64) -> Dataset {
        Dataset {
            version: version.to_string(),
            manifest_digest: digest.to_string(),
            bytes,
            tool_version: "0.1.0".to_string(),
            tool_git_sha: GIT_SHA.to_string(),
            doi: None,
            withdrawn: None,
        }
    }

    fn release(version: &str, hash: &str, bytes: u64, datasets: Vec<Dataset>) -> Release {
        Release {
            source: SourceRelease {
                version: version.to_string(),
                hash: hash.to_string(),
                bytes,
                issues: Vec::new(),
            },
            datasets,
        }
    }

    fn valid_test_index() -> OdsReleaseIndex {
        OdsReleaseIndex {
            source: IndexSource {
                signing_key_fingerprints: Some(vec!["71ED5964BAE53E83556320A42BE59DADEE84BEB0".to_string()]),
                ..OdsReleaseIndex::default().source
            },
            mirrors: vec![MirrorEntry {
                url: "https://ods.fyi/v2/ods-data".to_string(),
            }],
            releases: vec![
                release(
                    "2026-08-28",
                    SHA_A,
                    38064419,
                    vec![
                        dataset(
                            "2026-08-28_0.1.0",
                            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                            29_700_000,
                        ),
                        dataset(
                            "2026-08-28_0.2.0",
                            "sha256:abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
                            29_800_000,
                        ),
                    ],
                ),
                release(
                    "2026-07-31",
                    SHA_B,
                    37983173,
                    vec![dataset(
                        "2026-07-31_0.1.0",
                        "sha256:fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210",
                        29_600_000,
                    )],
                ),
            ],
            ..OdsReleaseIndex::default()
        }
    }

    fn fingerprints(idx: &mut OdsReleaseIndex) -> &mut Option<Vec<String>> {
        &mut idx.source.signing_key_fingerprints
    }

    #[test]
    fn test_validate_accepts_valid_index() {
        let idx = valid_test_index();
        assert!(idx.validate().is_ok());
    }

    #[test]
    fn test_validate_accepts_a_source_that_signs_nothing() {
        let mut idx = valid_test_index();
        *fingerprints(&mut idx) = None;
        assert!(idx.validate().is_ok());
        let json = serde_json::to_value(&idx).unwrap();
        assert!(json["source"].get("signing_key_fingerprints").is_none());
    }

    #[test]
    fn test_validate_rejects_malformed_index() {
        struct TestCase {
            name: &'static str,
            mutate: fn(&mut OdsReleaseIndex),
            expected_field: &'static str,
        }

        let cases = vec![
            TestCase {
                name: "wrong $schema",
                mutate: |idx| idx.schema = "https://ods.fyi/schema/releases.v2.json".to_string(),
                expected_field: "$schema",
            },
            TestCase {
                name: "name empty",
                mutate: |idx| idx.name = String::new(),
                expected_field: "Invalid name",
            },
            TestCase {
                name: "name upper case",
                mutate: |idx| idx.name = "ODS-data".to_string(),
                expected_field: "Invalid name",
            },
            TestCase {
                name: "source title empty",
                mutate: |idx| idx.source.title = String::new(),
                expected_field: "source.title",
            },
            TestCase {
                name: "source path empty",
                mutate: |idx| idx.source.path = String::new(),
                expected_field: "source.path",
            },
            TestCase {
                name: "fingerprints empty",
                mutate: |idx| *fingerprints(idx) = Some(vec![]),
                expected_field: "source.signing_key_fingerprints",
            },
            TestCase {
                name: "fingerprint too short",
                mutate: |idx| *fingerprints(idx) = Some(vec!["71ED5964".to_string()]),
                expected_field: "source.signing_key_fingerprints",
            },
            TestCase {
                name: "fingerprint lowercase",
                mutate: |idx| {
                    *fingerprints(idx) = Some(vec!["71ed5964bae53e83556320a42be59dadee84beb0".to_string()])
                },
                expected_field: "source.signing_key_fingerprints",
            },
            TestCase {
                name: "fingerprint non-hex",
                mutate: |idx| {
                    *fingerprints(idx) = Some(vec!["71ED5964BAE53E83556320A42BE59DADEE84BEZZ".to_string()])
                },
                expected_field: "source.signing_key_fingerprints",
            },
            TestCase {
                name: "fingerprint duplicate",
                mutate: |idx| {
                    *fingerprints(idx) = Some(vec![
                        "71ED5964BAE53E83556320A42BE59DADEE84BEB0".to_string(),
                        "71ED5964BAE53E83556320A42BE59DADEE84BEB0".to_string(),
                    ])
                },
                expected_field: "Duplicate source.signing_key_fingerprints",
            },
            TestCase {
                name: "invalid date format",
                mutate: |idx| idx.releases[0].source.version = "2026/08/28".to_string(),
                expected_field: "source.version",
            },
            TestCase {
                name: "date unpadded month (2026-8-28)",
                mutate: |idx| idx.releases[0].source.version = "2026-8-28".to_string(),
                expected_field: "source.version",
            },
            TestCase {
                name: "date unpadded day (2026-08-8)",
                mutate: |idx| idx.releases[0].source.version = "2026-08-8".to_string(),
                expected_field: "source.version",
            },
            TestCase {
                name: "date plus prefix (+2026-08-28)",
                mutate: |idx| idx.releases[0].source.version = "+2026-08-28".to_string(),
                expected_field: "source.version",
            },
            TestCase {
                name: "date trailing space (2026-08-28 )",
                mutate: |idx| idx.releases[0].source.version = "2026-08-28 ".to_string(),
                expected_field: "source.version",
            },
            TestCase {
                name: "duplicate date",
                mutate: |idx| {
                    idx.releases[1].source.version = "2026-08-28".to_string();
                    idx.releases[1].datasets[0].version = "2026-08-28_0.1.0".to_string();
                },
                expected_field: "Duplicate source.version",
            },
            TestCase {
                name: "releases not newest first",
                mutate: |idx| {
                    idx.releases.swap(0, 1);
                },
                expected_field: "Releases must run newest first",
            },
            TestCase {
                name: "hash upper case",
                mutate: |idx| {
                    idx.releases[0].source.hash = format!("sha256:{}", idx.releases[0].source.sha256_hex().to_uppercase())
                },
                expected_field: "source.hash",
            },
            TestCase {
                name: "hash without its sha256: prefix",
                mutate: |idx| idx.releases[0].source.hash = idx.releases[0].source.sha256_hex().to_string(),
                expected_field: "source.hash",
            },
            TestCase {
                name: "hash wrong length",
                mutate: |idx| idx.releases[0].source.hash = "sha256:abdd194b".to_string(),
                expected_field: "source.hash",
            },
            TestCase {
                name: "source bytes zero",
                mutate: |idx| idx.releases[0].source.bytes = 0,
                expected_field: "source.bytes",
            },
            TestCase {
                name: "issue ID not a slug",
                mutate: |idx| idx.releases[0].source.issues = vec!["T1201 recorded twice".to_string()],
                expected_field: "source.issues",
            },
            TestCase {
                name: "issue ID a URL",
                mutate: |idx| {
                    idx.releases[0].source.issues =
                        vec![source_issue_url("t1201-recorded-twice")]
                },
                expected_field: "source.issues",
            },
            TestCase {
                name: "issue ID duplicate",
                mutate: |idx| {
                    idx.releases[0].source.issues =
                        vec!["t1201-recorded-twice".to_string(), "t1201-recorded-twice".to_string()]
                },
                expected_field: "Duplicate source.issues",
            },
            TestCase {
                name: "dataset version without its source version",
                mutate: |idx| idx.releases[0].datasets[0].version = "0.1.0".to_string(),
                expected_field: "expected 2026-08-28_<dataset version>",
            },
            TestCase {
                name: "dataset version naming another release",
                mutate: |idx| idx.releases[0].datasets[0].version = "2026-07-31_0.1.0".to_string(),
                expected_field: "expected 2026-08-28_<dataset version>",
            },
            TestCase {
                name: "dataset version invalid semver",
                mutate: |idx| idx.releases[0].datasets[0].version = "2026-08-28_v0.1".to_string(),
                expected_field: "isn't SemVer",
            },
            TestCase {
                name: "duplicate dataset version within release",
                mutate: |idx| {
                    idx.releases[0].datasets[1].version = "2026-08-28_0.1.0".to_string();
                },
                expected_field: "Duplicate dataset version",
            },
            TestCase {
                name: "datasets not oldest first",
                mutate: |idx| {
                    idx.releases[0].datasets.swap(0, 1);
                },
                expected_field: "Datasets must run oldest version first",
            },
            TestCase {
                name: "dataset bytes zero",
                mutate: |idx| idx.releases[0].datasets[0].bytes = 0,
                expected_field: "Invalid bytes for dataset 2026-08-28_0.1.0",
            },
            TestCase {
                name: "manifest digest missing sha256 prefix",
                mutate: |idx| {
                    idx.releases[0].datasets[0].manifest_digest =
                        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                            .to_string();
                },
                expected_field: "manifest_digest",
            },
            TestCase {
                name: "manifest digest uppercase",
                mutate: |idx| {
                    idx.releases[0].datasets[0].manifest_digest =
                        "sha256:0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF"
                            .to_string();
                },
                expected_field: "manifest_digest",
            },
            TestCase {
                name: "manifest digest wrong length",
                mutate: |idx| {
                    idx.releases[0].datasets[0].manifest_digest = "sha256:012345".to_string();
                },
                expected_field: "manifest_digest",
            },
        ];

        for case in cases {
            let mut idx = valid_test_index();
            (case.mutate)(&mut idx);
            let result = idx.validate();
            assert!(
                result.is_err(),
                "Case '{}' should fail validation",
                case.name
            );
            let err_msg = format!("{:#}", result.unwrap_err());
            assert!(
                err_msg.contains(case.expected_field),
                "Case '{}' error '{}' must mention '{}'",
                case.name,
                err_msg,
                case.expected_field
            );
        }
    }

    #[test]
    fn test_dataset_version_is_the_part_after_the_underscore() {
        let idx = valid_test_index();
        let ds = &idx.releases[0].datasets[1];
        assert_eq!(ds.version, "2026-08-28_0.2.0");
        assert_eq!(ds.dataset_version(), "0.2.0");
        assert_eq!(ds.semver(), (0, 2, 0));
    }

    #[test]
    fn test_from_slice_names_an_old_format_index() {
        let old = br#"{
          "$schema": "https://ods.fyi/schema/releases.v1.json",
          "trud_signing_key_fingerprints": ["71ED5964BAE53E83556320A42BE59DADEE84BEB0"],
          "mirrors": [],
          "releases": []
        }"#;
        let err = OdsReleaseIndex::from_slice(old).unwrap_err();
        assert!(err.downcast_ref::<OldFormatIndex>().is_some(), "{err}");

        let old_row_only = br#"{"releases": [{"trud_release_date": "2026-09-25"}]}"#;
        assert!(is_old_format(old_row_only));
        assert!(!is_old_format(BAKED_RELEASES_JSON_BYTES));
        assert!(!is_old_format(b"not json"));
    }

    #[test]
    fn test_merge_refuses_contradicted_release_hash() {
        let baked = valid_test_index();
        let mut fetched = valid_test_index();
        fetched.releases[0].source.hash = SHA_NEW.to_string();

        let res = baked.merge(&fetched);
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("contradicts baked release"));
    }

    #[test]
    fn test_merge_refuses_contradicted_release_bytes() {
        let baked = valid_test_index();
        let mut fetched = valid_test_index();
        fetched.releases[0].source.bytes = 99999999;

        let res = baked.merge(&fetched);
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("contradicts baked release"));
    }

    #[test]
    fn test_merge_refuses_contradicted_dataset_digest() {
        let baked = valid_test_index();
        let mut fetched = valid_test_index();
        fetched.releases[0].datasets[0].manifest_digest =
            "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".to_string();

        let res = baked.merge(&fetched);
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("contradicts baked dataset"));
    }

    #[test]
    fn test_merge_refuses_a_changed_dataset_bytes() {
        let baked = valid_test_index();
        let mut fetched = valid_test_index();
        fetched.releases[0].datasets[0].bytes += 1;

        let res = baked.merge(&fetched);
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("contradicts baked dataset"));
    }

    #[test]
    fn test_merge_refuses_another_dataset_family() {
        let baked = valid_test_index();
        let mut fetched = valid_test_index();
        fetched.name = "postcodes".to_string();

        let err = baked.merge(&fetched).unwrap_err().to_string();
        assert!(err.contains("the candidate index lists postcodes, not ods-data"), "{err}");
    }

    // R2: a row's tool_version and tool_git_sha never change once written.
    #[test]
    fn test_merge_refuses_a_changed_tool_git_sha() {
        let baked = valid_test_index();
        let mut fetched = valid_test_index();
        fetched.releases[0].datasets[0].tool_git_sha = "f".repeat(40);

        let err = baked.merge(&fetched).unwrap_err();
        assert!(err.downcast_ref::<SecurityError>().is_some());
        println!("{err}");
        assert!(err.to_string().contains("contradicts baked dataset 2026-08-28_0.1.0: baked tool_git_sha"));
    }

    #[test]
    fn test_merge_refuses_a_changed_tool_version() {
        let baked = valid_test_index();
        let mut fetched = valid_test_index();
        fetched.releases[0].datasets[0].tool_version = "0.3.0".to_string();

        let err = baked.merge(&fetched).unwrap_err();
        assert!(err.downcast_ref::<SecurityError>().is_some());
        println!("{err}");
        assert!(err.to_string().contains("baked tool_version 0.1.0 != fetched tool_version 0.3.0"));
    }

    // R2: each dataset row names the ods that built it, and `validate` says which part is wrong.
    #[test]
    fn test_validate_names_a_row_missing_its_tool_version() {
        let mut json: serde_json::Value = serde_json::to_value(valid_test_index()).unwrap();
        json["releases"][0]["datasets"][0].as_object_mut().unwrap().remove("tool_version");
        let index: OdsReleaseIndex = serde_json::from_value(json).expect("a missing field parses, so validate can name it");

        let err = index.validate().unwrap_err().to_string();
        println!("{err}");
        assert!(err.contains("Missing tool_version for dataset 2026-08-28_0.1.0"));
    }

    #[test]
    fn test_validate_names_a_row_missing_its_tool_git_sha() {
        let mut index = valid_test_index();
        index.releases[0].datasets[1].tool_git_sha = String::new();

        let err = index.validate().unwrap_err().to_string();
        assert!(err.contains("Missing tool_git_sha for dataset 2026-08-28_0.2.0"));
    }

    #[test]
    fn test_validate_rejects_a_malformed_tool_version_and_commit() {
        for bad_version in ["v0.2.0", "0.2", "latest"] {
            let mut index = valid_test_index();
            index.releases[0].datasets[0].tool_version = bad_version.to_string();
            let err = format!("{:#}", index.validate().unwrap_err());
            assert!(err.contains(&format!("Invalid tool_version '{bad_version}'")), "{err}");
        }
        for bad_sha in ["0123456", &"F".repeat(40), &"g".repeat(40), &"a".repeat(41)] {
            let mut index = valid_test_index();
            index.releases[0].datasets[0].tool_git_sha = bad_sha.to_string();
            let err = index.validate().unwrap_err().to_string();
            assert!(err.contains("Invalid tool_git_sha") && err.contains("40 lower-case hex"), "{err}");
        }
    }

    #[test]
    fn test_merge_accepts_changed_signing_key_fingerprint() -> Result<()> {
        let baked = valid_test_index();
        let mut fetched = valid_test_index();
        *fingerprints(&mut fetched) = Some(vec!["AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string()]);

        let merged = baked.merge(&fetched)?;
        assert_eq!(
            merged.source.signing_key_fingerprints,
            Some(vec!["AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string()])
        );
        Ok(())
    }

    #[test]
    fn test_merge_accepts_added_release_and_dataset() -> Result<()> {
        let baked = valid_test_index();
        let mut fetched = valid_test_index();
        // Add a dataset to release 2026-07-31
        fetched.releases[1].datasets.push(dataset(
            "2026-07-31_0.2.0",
            "sha256:1111111111111111111111111111111111111111111111111111111111111111",
            29_900_000,
        ));
        // Add a brand new release
        fetched.releases.insert(
            0,
            release(
                "2026-09-25",
                SHA_NEW,
                40000000,
                vec![dataset(
                    "2026-09-25_0.1.0",
                    "sha256:2222222222222222222222222222222222222222222222222222222222222222",
                    30_000_000,
                )],
            ),
        );

        let merged = baked.merge(&fetched)?;
        assert_eq!(merged.releases.len(), 3);
        assert_eq!(merged.releases[0].source.version, "2026-09-25");
        assert_eq!(merged.releases[2].datasets.len(), 2);
        assert_eq!(merged.releases[2].datasets[1].version, "2026-07-31_0.2.0");
        Ok(())
    }

    #[test]
    fn test_merge_accepts_added_withdrawn_and_doi() -> Result<()> {
        let baked = valid_test_index();
        let mut fetched = valid_test_index();
        fetched.releases[0].datasets[0].withdrawn =
            Some("Data corruption in source".to_string());
        fetched.releases[0].datasets[0].doi =
            Some("10.5281/zenodo.12345".to_string());

        let merged = baked.merge(&fetched)?;
        assert_eq!(
            merged.releases[0].datasets[0].withdrawn.as_deref(),
            Some("Data corruption in source")
        );
        assert_eq!(
            merged.releases[0].datasets[0].doi.as_deref(),
            Some("10.5281/zenodo.12345")
        );
        Ok(())
    }

    // A source issue may appear on a row later, and is never removed.
    #[test]
    fn test_merge_adds_source_issues_and_never_removes_one() -> Result<()> {
        let mut baked = valid_test_index();
        baked.releases[1].source.issues = vec!["fef03-code-reused".to_string()];
        let mut fetched = valid_test_index();
        fetched.releases[0].source.issues = vec!["t1201-recorded-twice".to_string()];

        let merged = baked.merge(&fetched)?;
        assert_eq!(merged.releases[0].source.issues, vec!["t1201-recorded-twice"]);
        assert_eq!(merged.releases[1].source.issues, vec!["fef03-code-reused"]);
        Ok(())
    }

    #[test]
    fn test_resolve_newest_release_with_dataset() -> Result<()> {
        let mut idx = valid_test_index();
        // Insert a newer release that has NO datasets
        idx.releases.insert(0, release("2026-09-25", SHA_NEW, 40000000, vec![]));

        let res = idx.resolve(None)?;
        assert_eq!(res.release.source.version, "2026-08-28");
        assert_eq!(res.dataset.version, "2026-08-28_0.2.0");
        assert!(res.skipped.is_empty());
        Ok(())
    }

    #[test]
    fn test_resolve_requested_date() -> Result<()> {
        let idx = valid_test_index();
        let res = idx.resolve(Some("2026-07-31"))?;
        assert_eq!(res.release.source.version, "2026-07-31");
        assert_eq!(res.dataset.version, "2026-07-31_0.1.0");
        Ok(())
    }

    #[test]
    fn test_resolve_highest_version_within_date() -> Result<()> {
        let idx = valid_test_index();
        let res = idx.resolve(Some("2026-08-28"))?;
        assert_eq!(res.release.source.version, "2026-08-28");
        assert_eq!(res.dataset.version, "2026-08-28_0.2.0");
        Ok(())
    }

    // A release the index records with no dataset is refused as that, not as unknown, naming its
    // known issues and how to build it.
    #[test]
    fn test_resolve_date_with_empty_datasets_refuses() {
        let mut idx = valid_test_index();
        idx.releases.insert(0, release("2026-09-25", SHA_NEW, 40000000, vec![]));
        idx.releases[0].source.issues = vec!["two-full-files-2019-05".to_string()];

        let err = idx.resolve(Some("2026-09-25")).unwrap_err().to_string();
        assert_eq!(
            err,
            "✖ 2026-09-25 is in the release index, but no dataset has been published for it yet\n  \
             * known source issue: two-full-files-2019-05  https://github.com/olizilla/ods/blob/main/docs/source-issues/two-full-files-2019-05.md\n  \
             Build it yourself: ods trud pull 2026-09-25 && ods make"
        );

        idx.releases[0].source.issues.clear();
        let err = idx.resolve(Some("2026-09-25")).unwrap_err().to_string();
        assert_eq!(
            err,
            "✖ 2026-09-25 is in the release index, but no dataset has been published for it yet\n  \
             Build it yourself: ods trud pull 2026-09-25 && ods make"
        );
    }

    #[test]
    fn test_resolve_date_absent_from_the_index_refuses() {
        let err = valid_test_index().resolve(Some("2026-09-25")).unwrap_err().to_string();
        assert_eq!(err, "Release date '2026-09-25' is not known to the release index");
    }

    #[test]
    fn test_resolve_delivers_withdrawn_when_all_versions_for_date_withdrawn() -> Result<()> {
        let mut idx = valid_test_index();
        idx.releases[1].datasets[0].withdrawn =
            Some("critical corruption in roles".to_string());

        let res = idx.resolve(Some("2026-07-31"))?;
        assert_eq!(res.release.source.version, "2026-07-31");
        assert_eq!(res.dataset.version, "2026-07-31_0.1.0");
        assert!(res.dataset.is_withdrawn());
        Ok(())
    }

    #[test]
    fn test_resolve_none_skips_withdrawn_newest_date() -> Result<()> {
        let mut idx = valid_test_index();
        // idx has 2026-08-28 (newest) and 2026-07-31
        idx.releases[0].datasets[0].withdrawn = Some("withdrawn newest".to_string());
        idx.releases[0].datasets[1].withdrawn = Some("withdrawn newest v2".to_string());

        let res = idx.resolve(None)?;
        assert_eq!(res.release.source.version, "2026-07-31");
        assert_eq!(res.dataset.version, "2026-07-31_0.1.0");
        assert_eq!(res.skipped.len(), 1);
        assert_eq!(res.skipped[0].0.source.version, "2026-08-28");
        assert_eq!(res.skipped[0].1.version, "2026-08-28_0.2.0");
        Ok(())
    }

    #[test]
    fn test_resolve_none_fails_when_every_release_is_withdrawn() {
        let mut idx = valid_test_index();
        for r in &mut idx.releases {
            for d in &mut r.datasets {
                d.withdrawn = Some("withdrawn".to_string());
            }
        }

        let res = idx.resolve(None);
        assert!(res.is_err());
        let err = res.unwrap_err().to_string();
        assert!(err.contains("Every release in the index is withdrawn"));
        assert!(err.contains("See them: ods pull --list"));
    }
}

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const RELEASES_SCHEMA_V1_URL: &str = "https://ods.fyi/schema/releases.v1.json";
pub const RELEASES_JSON: &str = include_str!("../data/releases.json");
pub const BAKED_RELEASES_JSON_BYTES: &[u8] = include_bytes!("../data/releases.json");
pub const RELEASES_JSON_FILENAME: &str = "_releases.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OdsReleaseIndex {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub trud_signing_key_fingerprints: Vec<String>,
    pub mirrors: Vec<MirrorEntry>,
    pub releases: Vec<Release>,
}

impl Default for OdsReleaseIndex {
    fn default() -> Self {
        Self {
            schema: RELEASES_SCHEMA_V1_URL.to_string(),
            trud_signing_key_fingerprints: Vec::new(),
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub trud_release_date: String,
    pub trud_release_sha256: String,
    pub trud_release_filesize_bytes: u64,
    pub datasets: Vec<Dataset>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dataset {
    pub dataset_version: String,
    pub manifest_digest: String,
    pub dataset_filesize_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dataset_doi: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub withdrawn: Option<String>,
}

impl Dataset {
    pub fn is_withdrawn(&self) -> bool {
        self.withdrawn.is_some()
    }
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

#[derive(Debug)]
pub struct SecurityError(pub String);

impl std::fmt::Display for SecurityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Security error: {}", self.0)
    }
}

impl std::error::Error for SecurityError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRelease<'a> {
    pub release: &'a Release,
    pub dataset: &'a Dataset,
    pub skipped: Vec<(&'a Release, &'a Dataset)>,
}

impl OdsReleaseIndex {
    /// Loads and validates the release index from a workspace directory.
    pub fn load_from_workspace(workspace_root: &std::path::Path) -> Result<Option<Self>> {
        let path = workspace_root.join(RELEASES_JSON_FILENAME);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = std::fs::read(&path)
            .with_context(|| format!("reading index at {}", path.display()))?;
        let index: OdsReleaseIndex = serde_json::from_slice(&bytes)
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
        let index: Self = serde_json::from_str(RELEASES_JSON)
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

    /// Validates the release index against structural invariants:
    /// - $schema is exactly https://ods.fyi/schema/releases.v1.json
    /// - trud_signing_key_fingerprints has at least one entry, each 40 upper-case hex characters, no duplicates
    /// - each trud_release_date is YYYY-MM-DD, unique, and list runs newest first
    /// - each trud_release_sha256 is 64 upper-case hex characters, filesize > 0
    /// - each dataset_version parses with parse_semver and is unique, datasets run oldest version first
    /// - each manifest_digest is sha256: plus 64 lower-case hex characters
    pub fn validate(&self) -> Result<()> {
        if self.schema != RELEASES_SCHEMA_V1_URL {
            bail!(
                "Invalid $schema: expected '{}', got '{}'",
                RELEASES_SCHEMA_V1_URL,
                self.schema
            );
        }

        if self.trud_signing_key_fingerprints.is_empty() {
            bail!("Invalid trud_signing_key_fingerprints: expected at least one entry, got empty list");
        }

        let mut seen_fingerprints = HashSet::new();
        for fp in &self.trud_signing_key_fingerprints {
            if fp.len() != 40 || !fp.chars().all(|c| matches!(c, '0'..='9' | 'A'..='F')) {
                bail!(
                    "Invalid trud_signing_key_fingerprints: expected 40 upper-case hex characters, got '{}'",
                    fp
                );
            }
            if !seen_fingerprints.insert(fp) {
                bail!(
                    "Duplicate trud_signing_key_fingerprints in index: '{}'",
                    fp
                );
            }
        }

        let mut seen_dates = HashSet::new();
        for (i, rel) in self.releases.iter().enumerate() {
            // Validate exact YYYY-MM-DD date format (four digits, two digits, two digits)
            let is_yyyy_mm_dd = rel.trud_release_date.len() == 10
                && rel.trud_release_date.as_bytes()[4] == b'-'
                && rel.trud_release_date.as_bytes()[7] == b'-'
                && rel.trud_release_date.chars().enumerate().all(|(idx, c)| {
                    if idx == 4 || idx == 7 {
                        c == '-'
                    } else {
                        c.is_ascii_digit()
                    }
                });
            if !is_yyyy_mm_dd
                || chrono::NaiveDate::parse_from_str(&rel.trud_release_date, "%Y-%m-%d").is_err()
            {
                bail!(
                    "Invalid trud_release_date: expected YYYY-MM-DD date, got '{}'",
                    rel.trud_release_date
                );
            }

            // Uniqueness
            if !seen_dates.insert(&rel.trud_release_date) {
                bail!(
                    "Duplicate trud_release_date in index: '{}'",
                    rel.trud_release_date
                );
            }

            // Ordering: newest first
            if i > 0 && rel.trud_release_date >= self.releases[i - 1].trud_release_date {
                bail!(
                    "Releases must run newest first: '{}' is not newer than '{}'",
                    self.releases[i - 1].trud_release_date,
                    rel.trud_release_date
                );
            }

            // Validate sha256
            if rel.trud_release_sha256.len() != 64
                || !rel
                    .trud_release_sha256
                    .chars()
                    .all(|c| matches!(c, '0'..='9' | 'A'..='F'))
            {
                bail!(
                    "Invalid trud_release_sha256: expected 64 upper-case hex characters, got '{}'",
                    rel.trud_release_sha256
                );
            }

            // Validate filesize > 0
            if rel.trud_release_filesize_bytes == 0 {
                bail!("Invalid trud_release_filesize_bytes: must be greater than zero, got 0");
            }

            // Datasets within release
            let mut seen_versions = HashSet::new();
            let mut prev_semver: Option<(u64, u64, u64)> = None;

            for (j, ds) in rel.datasets.iter().enumerate() {
                let sem = parse_semver(&ds.dataset_version).with_context(|| {
                    format!(
                        "Invalid dataset_version '{}' for release '{}'",
                        ds.dataset_version, rel.trud_release_date
                    )
                })?;

                if !seen_versions.insert(&ds.dataset_version) {
                    bail!(
                        "Duplicate dataset_version '{}' for release '{}'",
                        ds.dataset_version,
                        rel.trud_release_date
                    );
                }

                if let Some(prev) = prev_semver {
                    if sem <= prev {
                        bail!(
                            "Datasets must run oldest version first: '{}' is not older than '{}' for release '{}'",
                            rel.datasets[j - 1].dataset_version,
                            ds.dataset_version,
                            rel.trud_release_date
                        );
                    }
                }
                prev_semver = Some(sem);

                // Verify manifest_digest is sha256: plus 64 lowercase hex
                if !ds.manifest_digest.starts_with("sha256:") {
                    bail!(
                        "Invalid manifest_digest: must start with 'sha256:', got '{}'",
                        ds.manifest_digest
                    );
                }
                let hex_part = &ds.manifest_digest["sha256:".len()..];
                if hex_part.len() != 64
                    || !hex_part
                        .chars()
                        .all(|c| matches!(c, '0'..='9' | 'a'..='f'))
                {
                    bail!(
                        "Invalid manifest_digest: must be 'sha256:' followed by 64 lower-case hex characters, got '{}'",
                        ds.manifest_digest
                    );
                }

                if ds.dataset_filesize_bytes == 0 {
                    bail!("Invalid dataset_filesize_bytes: must be greater than zero, got 0");
                }
            }
        }

        Ok(())
    }

    /// Merges a candidate index into self (the existing index).
    ///
    /// Rules:
    /// - Release in both: TRUD SHA-256 and filesize must match (SecurityError).
    /// - Dataset in both: manifest_digest must match (SecurityError). Candidate copy may add withdrawn and dataset_doi.
    /// - Additions: candidate index may add releases and datasets.
    /// - Mirrors: replaced by candidate list when non-empty.
    pub fn merge(&self, fetched: &OdsReleaseIndex) -> Result<OdsReleaseIndex> {
        self.validate().context("validating existing index")?;
        fetched.validate().context("validating candidate index")?;

        for baked_rel in &self.releases {
            if let Some(fetched_rel) = fetched
                .releases
                .iter()
                .find(|r| r.trud_release_date == baked_rel.trud_release_date)
            {
                if !baked_rel
                    .trud_release_sha256
                    .eq_ignore_ascii_case(&fetched_rel.trud_release_sha256)
                {
                    return Err(SecurityError(format!(
                        "fetched index contradicts baked release {}: baked TRUD SHA-256 {} != fetched TRUD SHA-256 {}",
                        baked_rel.trud_release_date,
                        baked_rel.trud_release_sha256,
                        fetched_rel.trud_release_sha256
                    ))
                    .into());
                }
                if baked_rel.trud_release_filesize_bytes != fetched_rel.trud_release_filesize_bytes {
                    return Err(SecurityError(format!(
                        "fetched index contradicts baked release {}: baked size {} != fetched size {}",
                        baked_rel.trud_release_date,
                        baked_rel.trud_release_filesize_bytes,
                        fetched_rel.trud_release_filesize_bytes
                    ))
                    .into());
                }
                for baked_ds in &baked_rel.datasets {
                    if let Some(fetched_ds) = fetched_rel
                        .datasets
                        .iter()
                        .find(|d| d.dataset_version == baked_ds.dataset_version)
                    {
                        if baked_ds.manifest_digest != fetched_ds.manifest_digest {
                            return Err(SecurityError(format!(
                                "fetched index contradicts baked dataset ({}, {}): baked digest {} != fetched digest {}",
                                baked_rel.trud_release_date,
                                baked_ds.dataset_version,
                                baked_ds.manifest_digest,
                                fetched_ds.manifest_digest
                            ))
                            .into());
                        }
                        if baked_ds.dataset_filesize_bytes != fetched_ds.dataset_filesize_bytes {
                            return Err(SecurityError(format!(
                                "fetched index contradicts baked dataset ({}, {}): baked size {} != fetched size {}",
                                baked_rel.trud_release_date,
                                baked_ds.dataset_version,
                                baked_ds.dataset_filesize_bytes,
                                fetched_ds.dataset_filesize_bytes
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
            if let Some(fetched_rel) = fetched
                .releases
                .iter()
                .find(|fr| fr.trud_release_date == baked_rel.trud_release_date)
            {
                for ds in &mut r.datasets {
                    if let Some(fetched_ds) = fetched_rel
                        .datasets
                        .iter()
                        .find(|fd| fd.dataset_version == ds.dataset_version)
                    {
                        if ds.withdrawn.is_none() && fetched_ds.withdrawn.is_some() {
                            ds.withdrawn = fetched_ds.withdrawn.clone();
                        }
                        if ds.dataset_doi.is_none() && fetched_ds.dataset_doi.is_some() {
                            ds.dataset_doi = fetched_ds.dataset_doi.clone();
                        }
                    }
                }
                for fetched_ds in &fetched_rel.datasets {
                    if !r
                        .datasets
                        .iter()
                        .any(|d| d.dataset_version == fetched_ds.dataset_version)
                    {
                        r.datasets.push(fetched_ds.clone());
                    }
                }
                r.datasets.sort_by(|a, b| {
                    let sem_a = parse_semver(&a.dataset_version).unwrap_or((0, 0, 0));
                    let sem_b = parse_semver(&b.dataset_version).unwrap_or((0, 0, 0));
                    sem_a.cmp(&sem_b)
                });
            }
            merged_releases.push(r);
        }

        for fetched_rel in &fetched.releases {
            if !self
                .releases
                .iter()
                .any(|r| r.trud_release_date == fetched_rel.trud_release_date)
            {
                merged_releases.push(fetched_rel.clone());
            }
        }

        merged_releases.sort_by(|a, b| b.trud_release_date.cmp(&a.trud_release_date));

        let mirrors = if !fetched.mirrors.is_empty() {
            fetched.mirrors.clone()
        } else {
            self.mirrors.clone()
        };

        let fingerprints = if !fetched.trud_signing_key_fingerprints.is_empty() {
            fetched.trud_signing_key_fingerprints.clone()
        } else {
            self.trud_signing_key_fingerprints.clone()
        };

        Ok(OdsReleaseIndex {
            schema: self.schema.clone(),
            trud_signing_key_fingerprints: fingerprints,
            mirrors,
            releases: merged_releases,
        })
    }

    /// Resolves the release and dataset to install:
    /// - requested date, or the newest release with a non-withdrawn dataset
    /// - within it, the highest dataset_version
    /// - a date with no datasets resolves the way a date absent from the index does today.
    pub fn resolve(&self, requested_date: Option<&str>) -> Result<ResolvedRelease<'_>> {
        match requested_date {
            Some(date) => {
                let release = self
                    .releases
                    .iter()
                    .find(|r| r.trud_release_date == date)
                    .ok_or_else(|| {
                        anyhow::anyhow!("Release date '{}' is not known to the release index", date)
                    })?;
                let (dataset, _) = select_dataset(release).ok_or_else(|| {
                    anyhow::anyhow!("Release date '{}' is not known to the release index", date)
                })?;
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

/// Selects the preferred dataset for a release:
/// - the highest semver non-withdrawn dataset if any exist, returning `Some((dataset, false))`
/// - otherwise the highest semver withdrawn dataset, returning `Some((dataset, true))`
/// - `None` if the release has no datasets
pub fn select_dataset(release: &Release) -> Option<(&Dataset, bool)> {
    if release.datasets.is_empty() {
        return None;
    }

    if let Some(ds) = release
        .datasets
        .iter()
        .filter(|d| !d.is_withdrawn())
        .max_by_key(|d| parse_semver(&d.dataset_version).unwrap_or((0, 0, 0)))
    {
        Some((ds, false))
    } else {
        let ds = release
            .datasets
            .iter()
            .max_by_key(|d| parse_semver(&d.dataset_version).unwrap_or((0, 0, 0)))
            .expect("datasets is non-empty");
        Some((ds, true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_test_index() -> OdsReleaseIndex {
        OdsReleaseIndex {
            schema: RELEASES_SCHEMA_V1_URL.to_string(),
            trud_signing_key_fingerprints: vec![
                "71ED5964BAE53E83556320A42BE59DADEE84BEB0".to_string()
            ],
            mirrors: vec![MirrorEntry {
                url: "https://ods.fyi/v2/ods-data".to_string(),
            }],
            releases: vec![
                Release {
                    trud_release_date: "2026-08-28".to_string(),
                    trud_release_sha256:
                        "ABDD194B1569D5FF3CDD81D618847F05642BD43C5B15D6CD43D8289B7466D801"
                            .to_string(),
                    trud_release_filesize_bytes: 38064419,
                    datasets: vec![
                        Dataset {
                            dataset_version: "0.1.0".to_string(),
                            manifest_digest:
                                "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                                    .to_string(),
                            dataset_filesize_bytes: 29_700_000,
                            dataset_doi: None,
                            withdrawn: None,
                        },
                        Dataset {
                            dataset_version: "0.2.0".to_string(),
                            manifest_digest:
                                "sha256:abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
                                    .to_string(),
                            dataset_filesize_bytes: 29_800_000,
                            dataset_doi: None,
                            withdrawn: None,
                        },
                    ],
                },
                Release {
                    trud_release_date: "2026-07-31".to_string(),
                    trud_release_sha256:
                        "8151248D1569D5FF3CDD81D618847F05642BD43C5B15D6CD43D8289B7466D801"
                            .to_string(),
                    trud_release_filesize_bytes: 37983173,
                    datasets: vec![Dataset {
                        dataset_version: "0.1.0".to_string(),
                        manifest_digest:
                            "sha256:fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210"
                                .to_string(),
                        dataset_filesize_bytes: 29_600_000,
                        dataset_doi: None,
                        withdrawn: None,
                    }],
                },
            ],
        }
    }

    #[test]
    fn test_validate_accepts_valid_index() {
        let idx = valid_test_index();
        assert!(idx.validate().is_ok());
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
                name: "fingerprints empty",
                mutate: |idx| idx.trud_signing_key_fingerprints = vec![],
                expected_field: "trud_signing_key_fingerprints",
            },
            TestCase {
                name: "fingerprint too short",
                mutate: |idx| idx.trud_signing_key_fingerprints = vec!["71ED5964".to_string()],
                expected_field: "trud_signing_key_fingerprints",
            },
            TestCase {
                name: "fingerprint lowercase",
                mutate: |idx| {
                    idx.trud_signing_key_fingerprints =
                        vec!["71ed5964bae53e83556320a42be59dadee84beb0".to_string()]
                },
                expected_field: "trud_signing_key_fingerprints",
            },
            TestCase {
                name: "fingerprint non-hex",
                mutate: |idx| {
                    idx.trud_signing_key_fingerprints =
                        vec!["71ED5964BAE53E83556320A42BE59DADEE84BEZZ".to_string()]
                },
                expected_field: "trud_signing_key_fingerprints",
            },
            TestCase {
                name: "fingerprint duplicate",
                mutate: |idx| {
                    idx.trud_signing_key_fingerprints = vec![
                        "71ED5964BAE53E83556320A42BE59DADEE84BEB0".to_string(),
                        "71ED5964BAE53E83556320A42BE59DADEE84BEB0".to_string(),
                    ]
                },
                expected_field: "Duplicate trud_signing_key_fingerprints",
            },
            TestCase {
                name: "invalid date format",
                mutate: |idx| idx.releases[0].trud_release_date = "2026/08/28".to_string(),
                expected_field: "trud_release_date",
            },
            TestCase {
                name: "date unpadded month (2026-8-28)",
                mutate: |idx| idx.releases[0].trud_release_date = "2026-8-28".to_string(),
                expected_field: "trud_release_date",
            },
            TestCase {
                name: "date unpadded day (2026-08-8)",
                mutate: |idx| idx.releases[0].trud_release_date = "2026-08-8".to_string(),
                expected_field: "trud_release_date",
            },
            TestCase {
                name: "date plus prefix (+2026-08-28)",
                mutate: |idx| idx.releases[0].trud_release_date = "+2026-08-28".to_string(),
                expected_field: "trud_release_date",
            },
            TestCase {
                name: "date trailing space (2026-08-28 )",
                mutate: |idx| idx.releases[0].trud_release_date = "2026-08-28 ".to_string(),
                expected_field: "trud_release_date",
            },
            TestCase {
                name: "duplicate date",
                mutate: |idx| idx.releases[1].trud_release_date = "2026-08-28".to_string(),
                expected_field: "trud_release_date",
            },
            TestCase {
                name: "releases not newest first",
                mutate: |idx| {
                    idx.releases.swap(0, 1);
                },
                expected_field: "Releases must run newest first",
            },
            TestCase {
                name: "sha256 lowercase",
                mutate: |idx| {
                    idx.releases[0].trud_release_sha256 =
                        idx.releases[0].trud_release_sha256.to_lowercase()
                },
                expected_field: "trud_release_sha256",
            },
            TestCase {
                name: "sha256 wrong length",
                mutate: |idx| idx.releases[0].trud_release_sha256 = "ABDD194B".to_string(),
                expected_field: "trud_release_sha256",
            },
            TestCase {
                name: "filesize zero",
                mutate: |idx| idx.releases[0].trud_release_filesize_bytes = 0,
                expected_field: "trud_release_filesize_bytes",
            },
            TestCase {
                name: "dataset version invalid semver",
                mutate: |idx| idx.releases[0].datasets[0].dataset_version = "v0.1".to_string(),
                expected_field: "dataset_version",
            },
            TestCase {
                name: "duplicate dataset version within release",
                mutate: |idx| {
                    idx.releases[0].datasets[1].dataset_version = "0.1.0".to_string();
                },
                expected_field: "dataset_version",
            },
            TestCase {
                name: "datasets not oldest first",
                mutate: |idx| {
                    idx.releases[0].datasets.swap(0, 1);
                },
                expected_field: "Datasets must run oldest version first",
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
            let err_msg = result.unwrap_err().to_string();
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
    fn test_merge_refuses_contradicted_release_hash() {
        let baked = valid_test_index();
        let mut fetched = valid_test_index();
        fetched.releases[0].trud_release_sha256 =
            "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF".to_string();

        let res = baked.merge(&fetched);
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("contradicts baked release"));
    }

    #[test]
    fn test_merge_refuses_contradicted_release_filesize() {
        let baked = valid_test_index();
        let mut fetched = valid_test_index();
        fetched.releases[0].trud_release_filesize_bytes = 99999999;

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
    fn test_merge_refuses_a_changed_dataset_filesize_bytes() {
        let baked = valid_test_index();
        let mut fetched = valid_test_index();
        fetched.releases[0].datasets[0].dataset_filesize_bytes += 1;

        let res = baked.merge(&fetched);
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("contradicts baked dataset"));
    }

    #[test]
    fn test_merge_accepts_changed_signing_key_fingerprint() -> Result<()> {
        let baked = valid_test_index();
        let mut fetched = valid_test_index();
        fetched.trud_signing_key_fingerprints =
            vec!["AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string()];

        let merged = baked.merge(&fetched)?;
        assert_eq!(
            merged.trud_signing_key_fingerprints,
            vec!["AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string()]
        );
        Ok(())
    }

    #[test]
    fn test_merge_accepts_added_release_and_dataset() -> Result<()> {
        let baked = valid_test_index();
        let mut fetched = valid_test_index();
        // Add a dataset to release 2026-07-31
        fetched.releases[1].datasets.push(Dataset {
            dataset_version: "0.2.0".to_string(),
            manifest_digest:
                "sha256:1111111111111111111111111111111111111111111111111111111111111111"
                    .to_string(),
            dataset_filesize_bytes: 29_900_000,
            dataset_doi: None,
            withdrawn: None,
        });
        // Add a brand new release
        fetched.releases.insert(
            0,
            Release {
                trud_release_date: "2026-09-25".to_string(),
                trud_release_sha256:
                    "9999999999999999999999999999999999999999999999999999999999999999".to_string(),
                trud_release_filesize_bytes: 40000000,
                datasets: vec![Dataset {
                    dataset_version: "0.1.0".to_string(),
                    manifest_digest:
                        "sha256:2222222222222222222222222222222222222222222222222222222222222222"
                            .to_string(),
                    dataset_filesize_bytes: 30_000_000,
                    dataset_doi: None,
                    withdrawn: None,
                }],
            },
        );

        let merged = baked.merge(&fetched)?;
        assert_eq!(merged.releases.len(), 3);
        assert_eq!(merged.releases[0].trud_release_date, "2026-09-25");
        assert_eq!(merged.releases[2].datasets.len(), 2);
        assert_eq!(merged.releases[2].datasets[1].dataset_version, "0.2.0");
        Ok(())
    }

    #[test]
    fn test_merge_accepts_added_withdrawn_and_doi() -> Result<()> {
        let baked = valid_test_index();
        let mut fetched = valid_test_index();
        fetched.releases[0].datasets[0].withdrawn =
            Some("Data corruption in source".to_string());
        fetched.releases[0].datasets[0].dataset_doi =
            Some("10.5281/zenodo.12345".to_string());

        let merged = baked.merge(&fetched)?;
        assert_eq!(
            merged.releases[0].datasets[0].withdrawn.as_deref(),
            Some("Data corruption in source")
        );
        assert_eq!(
            merged.releases[0].datasets[0].dataset_doi.as_deref(),
            Some("10.5281/zenodo.12345")
        );
        Ok(())
    }

    #[test]
    fn test_resolve_newest_release_with_dataset() -> Result<()> {
        let mut idx = valid_test_index();
        // Insert a newer release that has NO datasets
        idx.releases.insert(
            0,
            Release {
                trud_release_date: "2026-09-25".to_string(),
                trud_release_sha256:
                    "9999999999999999999999999999999999999999999999999999999999999999".to_string(),
                trud_release_filesize_bytes: 40000000,
                datasets: vec![],
            },
        );

        let res = idx.resolve(None)?;
        assert_eq!(res.release.trud_release_date, "2026-08-28");
        assert_eq!(res.dataset.dataset_version, "0.2.0");
        assert!(res.skipped.is_empty());
        Ok(())
    }

    #[test]
    fn test_resolve_requested_date() -> Result<()> {
        let idx = valid_test_index();
        let res = idx.resolve(Some("2026-07-31"))?;
        assert_eq!(res.release.trud_release_date, "2026-07-31");
        assert_eq!(res.dataset.dataset_version, "0.1.0");
        Ok(())
    }

    #[test]
    fn test_resolve_highest_version_within_date() -> Result<()> {
        let idx = valid_test_index();
        let res = idx.resolve(Some("2026-08-28"))?;
        assert_eq!(res.release.trud_release_date, "2026-08-28");
        assert_eq!(res.dataset.dataset_version, "0.2.0");
        Ok(())
    }

    #[test]
    fn test_resolve_date_with_empty_datasets_refuses() {
        let mut idx = valid_test_index();
        idx.releases.insert(
            0,
            Release {
                trud_release_date: "2026-09-25".to_string(),
                trud_release_sha256:
                    "9999999999999999999999999999999999999999999999999999999999999999".to_string(),
                trud_release_filesize_bytes: 40000000,
                datasets: vec![],
            },
        );

        let res = idx.resolve(Some("2026-09-25"));
        assert!(res.is_err());
        assert_eq!(
            res.unwrap_err().to_string(),
            "Release date '2026-09-25' is not known to the release index"
        );
    }

    #[test]
    fn test_resolve_delivers_withdrawn_when_all_versions_for_date_withdrawn() -> Result<()> {
        let mut idx = valid_test_index();
        idx.releases[1].datasets[0].withdrawn =
            Some("critical corruption in roles".to_string());

        let res = idx.resolve(Some("2026-07-31"))?;
        assert_eq!(res.release.trud_release_date, "2026-07-31");
        assert_eq!(res.dataset.dataset_version, "0.1.0");
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
        assert_eq!(res.release.trud_release_date, "2026-07-31");
        assert_eq!(res.dataset.dataset_version, "0.1.0");
        assert_eq!(res.skipped.len(), 1);
        assert_eq!(res.skipped[0].0.trud_release_date, "2026-08-28");
        assert_eq!(res.skipped[0].1.dataset_version, "0.2.0");
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

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const RELEASES_JSON: &str = include_str!("../data/releases.json");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OdsReleaseIndex {
    #[serde(rename = "_type")]
    pub type_tag: String,
    pub index_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concept_doi: Option<String>,
    pub mirrors: Vec<MirrorEntry>,
    pub releases: Vec<ReleaseIndexEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MirrorEntry {
    pub operator: String,
    pub kind: String,
    pub url: String,
}

impl MirrorEntry {
    pub fn expand_url(&self, release_date: &str, version: &str, file: &str) -> String {
        self.url
            .replace("{release}", release_date)
            .replace("{version}", version)
            .replace("{file}", file)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseIndexEntry {
    pub trud_release_date: String,
    pub dataset_version: String,
    pub tag: String,
    pub manifest_digest: String,
    pub trud_release_sha256: String,
    pub tool_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dataset_doi: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub withdrawn: Option<String>,
}

impl ReleaseIndexEntry {
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

impl OdsReleaseIndex {
    /// Loads and validates the checked-in baked release index.
    pub fn baked() -> Result<Self> {
        let index: Self = serde_json::from_str(RELEASES_JSON)
            .context("Failed to parse baked data/releases.json")?;
        index.validate()?;
        Ok(index)
    }

    /// Validates the release index against structural invariants:
    /// - type tag and schema version
    /// - lowercase hex SHA-256 digests
    /// - unique (date, version) pairs
    /// - valid semver version strings
    pub fn validate(&self) -> Result<()> {
        if self.type_tag != "ods_release_index" {
            bail!(
                "Invalid index _type: expected 'ods_release_index', got '{}'",
                self.type_tag
            );
        }
        if self.index_version != 2 {
            bail!(
                "Invalid index_version: expected 2, got {}",
                self.index_version
            );
        }

        let mut seen_pairs = HashSet::new();

        for rel in &self.releases {
            // Verify unique (date, version) pair
            let pair = (&rel.trud_release_date, &rel.dataset_version);
            if !seen_pairs.insert(pair) {
                bail!(
                    "Duplicate (date, version) pair in index: ({}, {})",
                    rel.trud_release_date,
                    rel.dataset_version
                );
            }

            // Verify semver
            parse_semver(&rel.dataset_version)?;

            // Verify manifest_digest is lowercase hex
            if !rel.manifest_digest.starts_with("sha256:") {
                bail!(
                    "manifest_digest must start with 'sha256:', got '{}'",
                    rel.manifest_digest
                );
            }
            let hex_part = &rel.manifest_digest["sha256:".len()..];
            if hex_part.len() != 64 {
                bail!(
                    "manifest_digest hex part must be 64 characters, got {} ('{}')",
                    hex_part.len(),
                    rel.manifest_digest
                );
            }
            if !hex_part.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()) {
                bail!(
                    "manifest_digest must be lowercase hex, got '{}'",
                    rel.manifest_digest
                );
            }
        }

        Ok(())
    }

    /// Merges a fetched index into self (the baked index).
    ///
    /// Rules:
    /// 1. A fetched index may add releases the binary doesn't know.
    /// 2. It may never override or contradict digests on a baked release (security error).
    /// 3. It CAN update `withdrawn` advisory on a baked release (how clients learn of systemic withdrawals).
    pub fn merge(&self, fetched: &OdsReleaseIndex) -> Result<OdsReleaseIndex> {
        self.validate().context("validating baked index")?;
        fetched.validate().context("validating fetched index")?;

        // Check for contradictions on any release known to self
        for baked_rel in &self.releases {
            if let Some(fetched_rel) = fetched
                .releases
                .iter()
                .find(|r| r.trud_release_date == baked_rel.trud_release_date && r.dataset_version == baked_rel.dataset_version)
            {
                if baked_rel.manifest_digest != fetched_rel.manifest_digest {
                    bail!(
                        "Security error: fetched index contradicts baked release ({}, {}): baked digest {} != fetched digest {}",
                        baked_rel.trud_release_date,
                        baked_rel.dataset_version,
                        baked_rel.manifest_digest,
                        fetched_rel.manifest_digest
                    );
                }
                if !baked_rel.trud_release_sha256.eq_ignore_ascii_case(&fetched_rel.trud_release_sha256) {
                    bail!(
                        "Security error: fetched index contradicts baked release ({}, {}): baked TRUD SHA-256 {} != fetched TRUD SHA-256 {}",
                        baked_rel.trud_release_date,
                        baked_rel.dataset_version,
                        baked_rel.trud_release_sha256,
                        fetched_rel.trud_release_sha256
                    );
                }
            }
        }

        let mut merged_releases = Vec::new();
        for baked_rel in &self.releases {
            let mut r = baked_rel.clone();
            if let Some(fetched_rel) = fetched
                .releases
                .iter()
                .find(|fr| fr.trud_release_date == baked_rel.trud_release_date && fr.dataset_version == baked_rel.dataset_version)
            {
                // Propagate withdrawal advisory if added upstream
                if r.withdrawn.is_none() && fetched_rel.withdrawn.is_some() {
                    r.withdrawn = fetched_rel.withdrawn.clone();
                }
                if r.dataset_doi.is_none() && fetched_rel.dataset_doi.is_some() {
                    r.dataset_doi = fetched_rel.dataset_doi.clone();
                }
            }
            merged_releases.push(r);
        }

        for fetched_rel in &fetched.releases {
            if !self.releases.iter().any(|r| {
                r.trud_release_date == fetched_rel.trud_release_date
                    && r.dataset_version == fetched_rel.dataset_version
            }) {
                merged_releases.push(fetched_rel.clone());
            }
        }

        let mirrors = if !fetched.mirrors.is_empty() {
            fetched.mirrors.clone()
        } else {
            self.mirrors.clone()
        };

        Ok(OdsReleaseIndex {
            type_tag: self.type_tag.clone(),
            index_version: self.index_version,
            concept_doi: fetched.concept_doi.clone().or_else(|| self.concept_doi.clone()),
            mirrors,
            releases: merged_releases,
        })
    }

    /// Resolves the release entry according to release resolution rules:
    /// - ods pull <date> -> the highest dataset_version for that date without withdrawn.
    ///   If none, refuse and print the reason.
    /// - ods pull -> the newest date, then the rule above.
    pub fn resolve(&self, requested_date: Option<&str>) -> Result<&ReleaseIndexEntry> {
        if self.releases.is_empty() {
            bail!("Release index contains no releases");
        }

        let target_date = match requested_date {
            Some(date) => date.to_string(),
            None => {
                // "the newest date, then the rule above"
                let mut all_dates: Vec<&str> = self
                    .releases
                    .iter()
                    .map(|r| r.trud_release_date.as_str())
                    .collect();
                all_dates.sort();
                all_dates.dedup();
                all_dates.last().unwrap().to_string()
            }
        };

        let for_date: Vec<&ReleaseIndexEntry> = self
            .releases
            .iter()
            .filter(|r| r.trud_release_date == target_date)
            .collect();

        if for_date.is_empty() {
            bail!("Release date '{}' is not known to the release index", target_date);
        }

        let mut non_withdrawn: Vec<&ReleaseIndexEntry> =
            for_date.iter().filter(|r| !r.is_withdrawn()).copied().collect();

        if non_withdrawn.is_empty() {
            // Sort withdrawn by semver descending to show the highest version's reason
            let mut withdrawn_list = for_date;
            withdrawn_list.sort_by(|a, b| {
                let sem_a = parse_semver(&a.dataset_version).unwrap_or((0, 0, 0));
                let sem_b = parse_semver(&b.dataset_version).unwrap_or((0, 0, 0));
                sem_b.cmp(&sem_a)
            });
            let top_withdrawn = withdrawn_list[0];
            let reason = top_withdrawn
                .withdrawn
                .as_deref()
                .unwrap_or("withdrawn by maintainer");
            bail!(
                "{} has no valid release\n  {} was withdrawn: {}",
                target_date,
                top_withdrawn.dataset_version,
                reason
            );
        }

        // Sort non_withdrawn by semver descending
        non_withdrawn.sort_by(|a, b| {
            let sem_a = parse_semver(&a.dataset_version).unwrap_or((0, 0, 0));
            let sem_b = parse_semver(&b.dataset_version).unwrap_or((0, 0, 0));
            sem_b.cmp(&sem_a)
        });

        Ok(non_withdrawn[0])
    }
}


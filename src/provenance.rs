use anyhow::{Context, Result};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

pub const NDJSON_TYPE_TAG: &str = "ods_provenance";

fn default_type_tag() -> String {
    NDJSON_TYPE_TAG.to_string()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrudVerificationSource {
    TrudApi,
    // Reserved for 07-pull-index-and-trust-chain; local archive yields Unverified until 07
    PublishedRelease,
    Unverified,
}

/// Unified dataset and build provenance metadata emitted as line 1 of canonical `ods.ndjson`
/// and saved as `_provenance.json` in workspace release directories.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdsProvenance {
    /// Discriminator tag distinguishing provenance header from organisation records
    #[serde(rename = "_type", default = "default_type_tag")]
    pub type_tag: String,

    // --- 1. Official TRUD API Release Metadata (trud_*) ---
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trud_release_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub trud_release_date: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub trud_release_file: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub trud_release_filesize_bytes: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub trud_release_sha256: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub trud_release_sha256_verified: Option<TrudVerificationSource>,

    // --- 2. Inner XML Manifest Metadata (publication_*) ---
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publication_date: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub publication_seq_num: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub publication_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub publication_source: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub publication_schema_version: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub publication_record_count: Option<usize>,

    // --- 3. Tool Build Info (tool_*) ---
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_version: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_git_sha: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_git_dirty: Option<bool>,

    // --- 4. Dataset Identity (dataset_*) ---
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dataset_version: Option<String>,
}

pub const PROVENANCE_FILENAME: &str = "_provenance.json";

impl Default for OdsProvenance {
    fn default() -> Self {
        Self {
            type_tag: NDJSON_TYPE_TAG.to_string(),
            trud_release_name: None,
            trud_release_date: None,
            trud_release_file: None,
            trud_release_filesize_bytes: None,
            trud_release_sha256: None,
            trud_release_sha256_verified: None,
            publication_date: None,
            publication_seq_num: None,
            publication_type: None,
            publication_source: None,
            publication_schema_version: None,
            publication_record_count: None,
            tool_version: None,
            tool_git_sha: None,
            tool_git_dirty: None,
            dataset_version: None,
        }
    }
}

impl OdsProvenance {
    pub fn to_parquet_metadata(&self) -> std::collections::BTreeMap<String, String> {
        let mut meta = std::collections::BTreeMap::new();
        if let Some(ref d) = self.trud_release_date {
            meta.insert("ods.trud_release_date".to_string(), d.clone());
        }
        if let Some(ref n) = self.trud_release_name {
            meta.insert("ods.trud_release_name".to_string(), n.clone());
        }
        if let Some(ref f) = self.trud_release_file {
            meta.insert("ods.trud_release_file".to_string(), f.clone());
        }
        if let Some(ref s) = self.trud_release_sha256 {
            meta.insert("ods.trud_release_sha256".to_string(), s.clone());
        }
        if let Some(ref d) = self.publication_date {
            meta.insert("ods.publication_date".to_string(), d.clone());
        }
        if let Some(ref s) = self.publication_seq_num {
            meta.insert("ods.publication_seq_num".to_string(), s.clone());
        }
        if let Some(ref t) = self.publication_type {
            meta.insert("ods.publication_type".to_string(), t.clone());
        }
        if let Some(ref s) = self.publication_source {
            meta.insert("ods.publication_source".to_string(), s.clone());
        }
        if let Some(ref v) = self.tool_version {
            meta.insert("ods.tool_version".to_string(), v.clone());
        }
        if let Some(ref sha) = self.tool_git_sha {
            meta.insert("ods.tool_git_sha".to_string(), sha.clone());
        }
        if let Some(dirty) = self.tool_git_dirty {
            if dirty {
                meta.insert("ods.tool_git_dirty".to_string(), "true".to_string());
            }
        }
        if let Some(ref ver) = self.dataset_version {
            meta.insert("ods.dataset_version".to_string(), ver.clone());
        }
        meta
    }

    pub fn load_from_dir(dir: &Path) -> Option<Self> {
        let mut curr = if dir.is_file() {
            dir.parent().map(|p| p.to_path_buf())
        } else {
            Some(dir.to_path_buf())
        };

        for _ in 0..4 {
            if let Some(ref path) = curr {
                let prov_file = path.join(PROVENANCE_FILENAME);
                if prov_file.exists() {
                    if let Ok(content) = std::fs::read_to_string(&prov_file) {
                        if let Ok(mut prov) = serde_json::from_str::<Self>(&content) {
                            if prov.trud_release_date.is_none() || prov.trud_release_file.is_none() {
                                if let Some(zip_prov) = Self::try_extract_trud_zip_provenance(path) {
                                    if prov.trud_release_date.is_none() { prov.trud_release_date = zip_prov.trud_release_date; }
                                    if prov.trud_release_file.is_none() { prov.trud_release_file = zip_prov.trud_release_file; }
                                    if prov.trud_release_name.is_none() { prov.trud_release_name = zip_prov.trud_release_name; }
                                    if prov.trud_release_sha256.is_none() { prov.trud_release_sha256 = zip_prov.trud_release_sha256; }
                                }
                            }
                            return Some(prov);
                        }
                    }
                }
                curr = path.parent().map(|p| p.to_path_buf());
            } else {
                break;
            }
        }
        None
    }

    pub fn try_extract_trud_zip_provenance(input_path: &Path) -> Option<Self> {
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
        let file_name = zip_path.file_name()?.to_string_lossy().to_string();

        let (_version, release_name, release_date) = crate::archive::parse_trud_archive_filename(&file_name)?;

        let mut prov = Self::new(None, Some(&zip_path));
        prov.trud_release_file = Some(file_name);
        prov.trud_release_date = Some(release_date);
        prov.trud_release_name = Some(release_name);

        if let Ok(meta) = std::fs::metadata(&zip_path) {
            prov.trud_release_filesize_bytes = Some(meta.len());
        }
        if let Ok(hash) = compute_file_sha256(&zip_path) {
            prov.trud_release_sha256 = Some(hash);
            prov.trud_release_sha256_verified = Some(TrudVerificationSource::Unverified);
        }

        Some(prov)
    }

    pub fn new(
        publication_date: Option<String>,
        source_path: Option<&Path>,
    ) -> Self {
        let source_file = source_path.and_then(|p| p.file_name()).map(|f| f.to_string_lossy().to_string());
        Self {
            type_tag: NDJSON_TYPE_TAG.to_string(),
            trud_release_name: None,
            trud_release_date: None,
            trud_release_file: source_file,
            trud_release_filesize_bytes: None,
            trud_release_sha256: None,
            trud_release_sha256_verified: None,
            publication_date,
            publication_seq_num: None,
            publication_type: None,
            publication_source: None,
            publication_schema_version: None,
            publication_record_count: None,
            tool_version: None,
            tool_git_sha: None,
            tool_git_dirty: None,
            dataset_version: None,
        }
    }

    pub fn validate_baseline(&self) -> Result<()> {
        let name = self.trud_release_name.as_deref().unwrap_or("");
        if name.is_empty() {
            anyhow::bail!("Missing trud_release_name in _provenance.json");
        }
        let date = self.trud_release_date.as_deref().unwrap_or("");
        if date.is_empty() {
            anyhow::bail!("Missing trud_release_date in _provenance.json");
        }
        let file = self.trud_release_file.as_deref().unwrap_or("");
        if file.is_empty() {
            anyhow::bail!("Missing trud_release_file in _provenance.json");
        }
        if self.trud_release_sha256.as_deref().unwrap_or("").is_empty() {
            anyhow::bail!("Missing trud_release_sha256 in _provenance.json");
        }

        // Validate release file matches version and date
        let date_digits: String = date.chars().filter(|c| c.is_ascii_digit()).collect();
        if !date_digits.is_empty() && !file.contains(&date_digits) {
            anyhow::bail!("trud_release_file '{}' does not match release date '{}'", file, date);
        }
        if let Some(ver) = name.strip_prefix("Release ").or_else(|| name.strip_prefix("release ")) {
            if !file.contains(ver) {
                anyhow::bail!("trud_release_file '{}' does not match release version '{}'", file, ver);
            }
        }

        Ok(())
    }

    pub fn validate_publishable(&self) -> Vec<String> {
        let mut failures = Vec::new();
        match self.trud_release_sha256_verified {
            Some(TrudVerificationSource::TrudApi) | Some(TrudVerificationSource::PublishedRelease) => {}
            Some(TrudVerificationSource::Unverified) => {
                failures.push("source not verified against the TRUD API — recorded in _provenance.json as unverified".to_string());
            }
            None => {
                failures.push("trud_release_sha256_verified is missing in _provenance.json".to_string());
            }
        }

        if let Some(sz) = self.trud_release_filesize_bytes {
            if sz < 1_000_000 {
                failures.push(format!("trud_release_filesize_bytes is implausibly small ({} bytes)", sz));
            }
        } else {
            failures.push("trud_release_filesize_bytes is missing in _provenance.json".to_string());
        }

        failures
    }
}

pub fn try_parse_provenance_line(line: &str) -> Option<OdsProvenance> {
    if !line.contains(NDJSON_TYPE_TAG) {
        return None;
    }
    serde_json::from_str::<OdsProvenance>(line).ok()
}

pub fn compute_file_sha256(path: &Path) -> Result<String> {
    let file = File::open(path).with_context(|| format!("opening file for sha256: {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    std::io::copy(&mut reader, &mut hasher)?;
    Ok(format!("{:X}", hasher.finalize()))
}

pub fn sanitize_trud_url(url: &str, api_key: Option<&str>) -> String {
    if let Some(key) = api_key {
        if !key.is_empty() {
            return url.replace(key, "<REDACTED_API_KEY>");
        }
    }
    url.to_string()
}

pub fn update_provenance(output_dir: &Path, dataset_version: Option<&str>) -> Result<()> {
    let prov_path = output_dir.join(PROVENANCE_FILENAME);
    let mut prov = if prov_path.exists() {
        let content = std::fs::read_to_string(&prov_path)?;
        serde_json::from_str::<OdsProvenance>(&content)?
    } else if let Some(loaded) = OdsProvenance::load_from_dir(output_dir) {
        loaded
    } else {
        OdsProvenance::try_extract_trud_zip_provenance(output_dir).unwrap_or_default()
    };

    // Pre-amend verification: Check archive in trud/ matches trud_release_file and trud_release_sha256
    if let (Some(ref file), Some(ref expected_sha)) = (&prov.trud_release_file, &prov.trud_release_sha256) {
        let archive_path = output_dir.join("trud").join(file);
        if archive_path.exists() {
            let actual_sha = compute_file_sha256(&archive_path)?;
            if !actual_sha.eq_ignore_ascii_case(expected_sha) {
                anyhow::bail!(
                    "✖ Pre-build archive verification mismatch for {}: expected SHA-256 {}, got {}",
                    file,
                    expected_sha,
                    actual_sha
                );
            }
        }
    }

    // Freshly parsed XML manifest publication_* fields win over stale or missing fields on disk
    if let Ok(header) = crate::commands::ndjson::extract_manifest_header(output_dir) {
        if header.publication_date.is_some() {
            prov.publication_date = header.publication_date;
        }
        if header.publication_seq_num.is_some() {
            prov.publication_seq_num = header.publication_seq_num;
        }
        if header.publication_type.is_some() {
            prov.publication_type = header.publication_type;
        }
        if header.publication_source.is_some() {
            prov.publication_source = header.publication_source;
        }
        if header.publication_schema_version.is_some() {
            prov.publication_schema_version = header.publication_schema_version;
        }
        if header.publication_record_count.is_some() {
            prov.publication_record_count = header.publication_record_count;
        }
    }

    if prov.type_tag.is_empty() {
        prov.type_tag = NDJSON_TYPE_TAG.to_string();
    }
    prov.tool_version = Some(env!("CARGO_PKG_VERSION").to_string());
    prov.tool_git_sha = option_env!("ODS_GIT_SHA").map(|s| s.to_string());
    prov.tool_git_dirty = if option_env!("ODS_GIT_DIRTY").is_some() {
        Some(true)
    } else {
        None
    };

    prov.dataset_version = Some(
        dataset_version
            .map(|s| s.to_string())
            .unwrap_or_else(|| crate::datapackage::dataset_version().to_string()),
    );

    let updated_json = serde_json::to_string_pretty(&prov)?;
    std::fs::write(&prov_path, updated_json)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provenance_serde() {
        let prov = OdsProvenance::new(
            Some("2026-07-31".to_string()),
            Some(Path::new("HSCOrgRefData.xml")),
        );

        let json = serde_json::to_string(&prov).unwrap();
        assert!(json.contains("ods_provenance"));
        assert!(json.contains("2026-07-31"));

        let parsed = try_parse_provenance_line(&json).unwrap();
        assert_eq!(parsed.publication_date, Some("2026-07-31".to_string()));
    }

    #[test]
    fn test_sanitize_trud_url() {
        let url = "https://isd.digital.nhs.uk/trud/api/v1/keys/SECRET123/items/341";
        let sanitized = sanitize_trud_url(url, Some("SECRET123"));
        assert_eq!(sanitized, "https://isd.digital.nhs.uk/trud/api/v1/keys/<REDACTED_API_KEY>/items/341");
    }
}

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

pub const NDJSON_TYPE_TAG: &str = "ods_provenance";

fn default_type_tag() -> String {
    NDJSON_TYPE_TAG.to_string()
}

/// Unified dataset and build provenance metadata emitted as line 1 of canonical `ods.ndjson`
/// and saved as `provenance.json` in workspace release directories.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdsProvenance {
    /// Discriminator tag distinguishing provenance header from organisation records
    #[serde(rename = "_type", default = "default_type_tag")]
    pub type_tag: String,

    // --- 1. Official TRUD API Release Metadata ---
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
    pub trud_release_sha256_verified: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub trud_release_url: Option<String>,

    // --- 2. Inner XML Manifest Metadata ---
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publication_seq_num: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub publication_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub publication_source: Option<String>,

    // --- 3. Tool Build Info & Artifact Hashes ---
    #[serde(skip_serializing_if = "Option::is_none", skip_deserializing)]
    pub output_path: Option<String>,

    pub ods_cmd_version: String,

    #[serde(skip_serializing_if = "Option::is_none", skip_deserializing)]
    pub fetched_at: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none", skip_deserializing)]
    pub created_at: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub derived_artifacts: Option<std::collections::BTreeMap<String, String>>,
}

pub const PROVENANCE_FILENAME: &str = "_provenance.json";

impl OdsProvenance {
    pub fn load_from_dir(dir: &Path) -> Option<Self> {
        let prov_file = dir.join(PROVENANCE_FILENAME);
        if prov_file.exists() {
            if let Ok(content) = std::fs::read_to_string(&prov_file) {
                if let Ok(prov) = serde_json::from_str::<Self>(&content) {
                    return Some(prov);
                }
            }
        }
        if let Some(parent) = dir.parent() {
            let parent_prov = parent.join(PROVENANCE_FILENAME);
            if parent_prov.exists() {
                if let Ok(content) = std::fs::read_to_string(&parent_prov) {
                    if let Ok(prov) = serde_json::from_str::<Self>(&content) {
                        return Some(prov);
                    }
                }
            }
        }
        None
    }

    pub fn new(
        publication_date: Option<String>,
        publication_seq_num: Option<String>,
        publication_type: Option<String>,
        publication_source: Option<String>,
        source_path: Option<&Path>,
    ) -> Self {
        let source_file = source_path.and_then(|p| p.file_name()).map(|f| f.to_string_lossy().to_string());
        let output_path = source_path.map(|p| p.to_string_lossy().to_string());
        Self {
            type_tag: NDJSON_TYPE_TAG.to_string(),
            trud_release_name: None,
            trud_release_date: publication_date,
            trud_release_file: source_file,
            trud_release_filesize_bytes: None,
            trud_release_sha256: None,
            trud_release_sha256_verified: None,
            trud_release_url: None,
            publication_seq_num,
            publication_type,
            publication_source,
            output_path,
            ods_cmd_version: env!("CARGO_PKG_VERSION").to_string(),
            fetched_at: None,
            created_at: Some(Utc::now().to_rfc3339()),
            derived_artifacts: None,
        }
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

pub fn update_provenance_and_write_sha256sums(output_dir: &Path) -> Result<()> {
    let parquet_files = vec![
        "orgs.parquet",
        "orgs_all.parquet",
        "roles.parquet",
        "rels.parquet",
        "successors.parquet",
    ];

    let mut hashes = std::collections::BTreeMap::new();
    let mut sha_lines = Vec::new();

    for file_name in &parquet_files {
        let file_path = output_dir.join(file_name);
        if file_path.exists() {
            let hash = compute_file_sha256(&file_path)?;
            hashes.insert((*file_name).to_string(), hash.clone());
            sha_lines.push(format!("{}  {}", hash.to_lowercase(), file_name));
        }
    }

    let prov_path = output_dir.join(PROVENANCE_FILENAME);
    if prov_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&prov_path) {
            if let Ok(mut prov) = serde_json::from_str::<OdsProvenance>(&content) {
                prov.derived_artifacts = Some(hashes);
                prov.created_at = Some(Utc::now().to_rfc3339());
                if let Ok(updated_json) = serde_json::to_string_pretty(&prov) {
                    let _ = std::fs::write(&prov_path, updated_json);
                    let prov_hash = compute_file_sha256(&prov_path)?;
                    sha_lines.push(format!("{}  {}", prov_hash.to_lowercase(), PROVENANCE_FILENAME));
                }
            }
        }
    }

    if !sha_lines.is_empty() {
        let sums_path = output_dir.join("SHA256SUMS");
        let sums_content = sha_lines.join("\n") + "\n";
        std::fs::write(&sums_path, sums_content)
            .with_context(|| format!("writing SHA256SUMS manifest to {}", sums_path.display()))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provenance_serde() {
        let prov = OdsProvenance::new(
            Some("2026-07-31".to_string()),
            Some("4574".to_string()),
            Some("Full".to_string()),
            Some("HSCIC".to_string()),
            Some(Path::new("HSCOrgRefData.xml")),
        );

        let json = serde_json::to_string(&prov).unwrap();
        assert!(json.contains("ods_provenance"));
        assert!(json.contains("2026-07-31"));

        let parsed = try_parse_provenance_line(&json).unwrap();
        assert_eq!(parsed.trud_release_date, Some("2026-07-31".to_string()));
    }

    #[test]
    fn test_sanitize_trud_url() {
        let url = "https://isd.digital.nhs.uk/trud/api/v1/keys/SECRET123/items/341";
        let sanitized = sanitize_trud_url(url, Some("SECRET123"));
        assert_eq!(sanitized, "https://isd.digital.nhs.uk/trud/api/v1/keys/<REDACTED_API_KEY>/items/341");
    }
}

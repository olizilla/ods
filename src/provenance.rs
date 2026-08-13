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

fn default_tool_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
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

    // --- 3. Tool Build Info & Artifact Hashes ---
    #[serde(default = "default_tool_version")]
    pub tool_version: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_parquet_version: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_arrow_version: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_zstd_level: Option<i32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub dataset_doi: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub derived_artifacts: Option<std::collections::BTreeMap<String, String>>,
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
            trud_release_url: None,
            publication_date: None,
            publication_seq_num: None,
            publication_type: None,
            publication_source: None,
            publication_schema_version: None,
            publication_record_count: None,
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
            tool_parquet_version: None,
            tool_arrow_version: None,
            tool_zstd_level: None,
            dataset_doi: None,
            derived_artifacts: None,
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
        if let Some(ref u) = self.trud_release_url {
            meta.insert("ods.trud_release_url".to_string(), u.clone());
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
        meta.insert("ods.tool_version".to_string(), self.tool_version.clone());
        if let Some(ref v) = self.tool_parquet_version {
            meta.insert("ods.tool_parquet_version".to_string(), v.clone());
        }
        if let Some(ref v) = self.tool_arrow_version {
            meta.insert("ods.tool_arrow_version".to_string(), v.clone());
        }
        if let Some(v) = self.tool_zstd_level {
            meta.insert("ods.tool_zstd_level".to_string(), v.to_string());
        }
        if let Some(ref doi) = self.dataset_doi {
            meta.insert("ods.dataset_doi".to_string(), doi.clone());
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
                                    if prov.trud_release_url.is_none() { prov.trud_release_url = zip_prov.trud_release_url; }
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
            if dir.is_file() && dir.extension().map_or(false, |ext| ext == "zip") {
                return Some(dir.to_path_buf());
            }
            if dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if p.is_file() && p.extension().map_or(false, |ext| ext == "zip") {
                            return Some(p);
                        }
                    }
                }
                let trud_sub = dir.join("trud");
                if trud_sub.is_dir() {
                    if let Ok(entries) = std::fs::read_dir(&trud_sub) {
                        for entry in entries.flatten() {
                            let p = entry.path();
                            if p.is_file() && p.extension().map_or(false, |ext| ext == "zip") {
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

        let mut prov = Self::new(None, Some(&zip_path));
        prov.trud_release_file = Some(file_name.clone());

        // Extract date YYYYMMDD (e.g. 20260731) from filename
        for chunk in file_name.split('_') {
            let digits: String = chunk.chars().filter(|c: &char| c.is_ascii_digit()).collect();
            if digits.len() >= 8 && (digits.starts_with("202") || digits.starts_with("203")) {
                let yyyy = &digits[0..4];
                let mm = &digits[4..6];
                let dd = &digits[6..8];
                prov.trud_release_date = Some(format!("{}-{}-{}", yyyy, mm, dd));
                break;
            }
        }

        // Extract version like 7.0.0 from filename
        for chunk in file_name.split('_') {
            if chunk.contains('.') && chunk.chars().any(|c: char| c.is_ascii_digit()) {
                prov.trud_release_name = Some(format!("Release {}", chunk));
                break;
            }
        }

        if let Ok(meta) = std::fs::metadata(&zip_path) {
            prov.trud_release_filesize_bytes = Some(meta.len());
        }
        if let Ok(hash) = compute_file_sha256(&zip_path) {
            prov.trud_release_sha256 = Some(hash);
            prov.trud_release_sha256_verified = Some(true);
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
            trud_release_url: None,
            publication_date,
            publication_seq_num: None,
            publication_type: None,
            publication_source: None,
            publication_schema_version: None,
            publication_record_count: None,
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
            tool_parquet_version: Some(env!("ODS_TOOL_PARQUET_VERSION").to_string()),
            tool_arrow_version: Some(env!("ODS_TOOL_ARROW_VERSION").to_string()),
            tool_zstd_level: Some(3),
            dataset_doi: None,
            derived_artifacts: None,
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
        if self.trud_release_sha256_verified != Some(true) {
            anyhow::bail!("trud_release_sha256_verified must be true in _provenance.json");
        }

        // Validate URL is not local file:// or local path
        if let Some(ref url) = self.trud_release_url {
            if url.starts_with("file://") || url.starts_with('/') || url.contains("/tmp/") {
                anyhow::bail!("trud_release_url must be a canonical remote TRUD URL, got: {}", url);
            }
        }

        // Validate plausible filesize (> 1 MB)
        if let Some(sz) = self.trud_release_filesize_bytes {
            if sz < 1_000_000 {
                anyhow::bail!("trud_release_filesize_bytes is implausibly small ({} bytes)", sz);
            }
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
    // Data and opinions alike: `category_rules.json` states where we disagree
    // with the ODS primary role, so it is hashed like any other artifact. Its
    // hash is what answers "did the opinion change between these releases?".
    let parquet_files = vec![
        "orgs.parquet",
        "orgs_all.parquet",
        "org_roles.parquet",
        "roles.parquet",
        "rels.parquet",
        "successors.parquet",
        "category_rules.json",
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
    let mut prov = if prov_path.exists() {
        let content = std::fs::read_to_string(&prov_path)?;
        serde_json::from_str::<OdsProvenance>(&content)?
    } else if let Some(loaded) = OdsProvenance::load_from_dir(output_dir) {
        loaded
    } else if let Some(extracted) = OdsProvenance::try_extract_trud_zip_provenance(output_dir) {
        extracted
    } else {
        OdsProvenance::default()
    };

    // Defect B Fix: freshly parsed XML manifest publication_* fields win over stale or missing fields on disk
    if let Ok(xml_path) = crate::commands::ndjson::find_xml_file(output_dir) {
        if let Ok((xml_prov, _, _)) = crate::commands::ndjson::parse_single_pass(&xml_path) {
            if xml_prov.publication_date.is_some() {
                prov.publication_date = xml_prov.publication_date;
            }
            if xml_prov.publication_seq_num.is_some() {
                prov.publication_seq_num = xml_prov.publication_seq_num;
            }
            if xml_prov.publication_type.is_some() {
                prov.publication_type = xml_prov.publication_type;
            }
            if xml_prov.publication_source.is_some() {
                prov.publication_source = xml_prov.publication_source;
            }
            if xml_prov.publication_schema_version.is_some() {
                prov.publication_schema_version = xml_prov.publication_schema_version;
            }
            if xml_prov.publication_record_count.is_some() {
                prov.publication_record_count = xml_prov.publication_record_count;
            }
        }
    }

    if prov.type_tag.is_empty() {
        prov.type_tag = NDJSON_TYPE_TAG.to_string();
    }
    if prov.tool_version.is_empty() {
        prov.tool_version = env!("CARGO_PKG_VERSION").to_string();
    }
    if prov.tool_parquet_version.is_none() {
        prov.tool_parquet_version = Some(env!("ODS_TOOL_PARQUET_VERSION").to_string());
    }
    if prov.tool_arrow_version.is_none() {
        prov.tool_arrow_version = Some(env!("ODS_TOOL_ARROW_VERSION").to_string());
    }
    if prov.tool_zstd_level.is_none() {
        prov.tool_zstd_level = Some(3);
    }

    prov.derived_artifacts = Some(hashes);
    if let Ok(updated_json) = serde_json::to_string_pretty(&prov) {
        let _ = std::fs::write(&prov_path, updated_json);
        if let Ok(prov_hash) = compute_file_sha256(&prov_path) {
            sha_lines.push(format!("{}  {}", prov_hash.to_lowercase(), PROVENANCE_FILENAME));
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
            Some(Path::new("HSCOrgRefData.xml")),
        );

        let json = serde_json::to_string(&prov).unwrap();
        assert!(json.contains("ods_provenance"));
        assert!(json.contains("2026-07-31"));

        let parsed = try_parse_provenance_line(&json).unwrap();
        assert_eq!(parsed.publication_date, Some("2026-07-31".to_string()));
    }

    /// `build.rs` reads the resolved `parquet` and `arrow` versions out of
    /// `Cargo.lock`. If it ever fails to find the lock file — a packaged crate,
    /// a vendored build, an unusual workspace layout — it falls back to
    /// `"unknown"` and every release would silently start recording that.
    ///
    /// These versions exist to explain a byte-level hash after the fact, so an
    /// absent or stale value defeats the field's whole purpose. Assert both that
    /// it was derived at all, and that it still matches the lock file.
    #[test]
    fn tool_crate_versions_are_derived_from_cargo_lock() {
        let parquet_ver = env!("ODS_TOOL_PARQUET_VERSION");
        let arrow_ver = env!("ODS_TOOL_ARROW_VERSION");

        for (name, ver) in [("parquet", parquet_ver), ("arrow", arrow_ver)] {
            assert_ne!(
                ver, "unknown",
                "build.rs could not read Cargo.lock, so the {name} version was not recorded"
            );
            assert_eq!(
                ver.split('.').count(),
                3,
                "expected a semver for {name}, got {ver:?}"
            );
        }

        // Re-derive independently and compare, so a dependency bump that does
        // not trigger a rebuild is caught rather than shipped.
        let lock = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.lock"),
        )
        .expect("Cargo.lock is present in the repo");

        let version_of = |pkg: &str| -> Option<String> {
            let mut current = String::new();
            for line in lock.lines().map(str::trim) {
                if line == "[[package]]" {
                    current.clear();
                } else if let Some(v) = line.strip_prefix("name = ") {
                    current = v.trim_matches('"').to_string();
                } else if let Some(v) = line.strip_prefix("version = ") {
                    if current == pkg {
                        return Some(v.trim_matches('"').to_string());
                    }
                }
            }
            None
        };

        assert_eq!(
            version_of("parquet").as_deref(),
            Some(parquet_ver),
            "recorded parquet version has drifted from Cargo.lock"
        );
        assert_eq!(
            version_of("arrow").as_deref(),
            Some(arrow_ver),
            "recorded arrow version has drifted from Cargo.lock"
        );
    }

    #[test]
    fn test_sanitize_trud_url() {
        let url = "https://isd.digital.nhs.uk/trud/api/v1/keys/SECRET123/items/341";
        let sanitized = sanitize_trud_url(url, Some("SECRET123"));
        assert_eq!(sanitized, "https://isd.digital.nhs.uk/trud/api/v1/keys/<REDACTED_API_KEY>/items/341");
    }
}

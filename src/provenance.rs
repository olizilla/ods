use anyhow::{Context, Result};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

pub const PROVENANCE_SCHEMA_V1_URL: &str = "https://ods.fyi/schema/provenance.v1.json";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrudVerificationSource {
    TrudApi,
    // Reserved for 07-pull-index-and-trust-chain; local archive yields Unverified until 07
    PublishedRelease,
    Unverified,
}

/// Unified dataset and build provenance metadata stored in `_provenance.json`
/// and saved as `_provenance.json` in workspace release directories.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdsProvenance {
    /// JSON Schema URI identifying this provenance document format
    #[serde(rename = "$schema")]
    pub schema: String,

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
            schema: PROVENANCE_SCHEMA_V1_URL.to_string(),
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
    /// Declared Parquet key-value metadata subset:
    /// - ods.trud_release_date
    /// - ods.trud_release_name
    /// - ods.trud_release_file
    /// - ods.trud_release_sha256
    /// - ods.publication_date
    /// - ods.publication_seq_num
    /// - ods.publication_type
    /// - ods.publication_source
    ///
    /// Tool fields and verification source are excluded so Parquet bytes
    /// are a function of the source archive and the derivation.
    pub fn to_parquet_declared_metadata(&self) -> std::collections::BTreeMap<String, String> {
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
        meta
    }
}

#[derive(Debug, Clone)]
pub enum ProvenanceLoad {
    Read(Box<OdsProvenance>, PathBuf),
    Unreadable {
        path: PathBuf,
        date: String,
    },
    Absent,
}

pub fn format_unreadable_provenance_error(path: &Path, date: &str) -> String {
    let disp = format_provenance_display_path(path);
    format!(
        "✖ {} isn't provenance this ods can read\n  Expected $schema {}\n  Pull the archive again with `ods trud pull {} --force`, then run `ods make`.",
        disp,
        PROVENANCE_SCHEMA_V1_URL,
        date
    )
}

impl ProvenanceLoad {
    pub fn ok(self) -> Option<OdsProvenance> {
        match self {
            Self::Read(prov, _) => Some(*prov),
            _ => None,
        }
    }

    pub fn ok_with_path(self) -> Option<(OdsProvenance, PathBuf)> {
        match self {
            Self::Read(prov, path) => Some((*prov, path)),
            _ => None,
        }
    }

    pub fn unwrap(self) -> OdsProvenance {
        match self {
            Self::Read(prov, _) => *prov,
            Self::Unreadable { path, date } => {
                panic!("unreadable provenance at {:?} (date: {})", path, date)
            }
            Self::Absent => panic!("expected provenance to be present, but was absent"),
        }
    }

    pub fn unwrap_or_default(self) -> OdsProvenance {
        self.ok().unwrap_or_default()
    }

    pub fn as_ref(&self) -> Option<&OdsProvenance> {
        match self {
            Self::Read(prov, _) => Some(prov.as_ref()),
            _ => None,
        }
    }

    pub fn warn_reading(self) -> Option<OdsProvenance> {
        match self {
            Self::Read(prov, _) => Some(*prov),
            Self::Unreadable { path, .. } => {
                let disp = format_provenance_display_path(&path);
                eprintln!(
                    "! {} isn't provenance this ods can read. Rebuild the release with `ods make`, or pull it again.",
                    disp
                );
                None
            }
            Self::Absent => None,
        }
    }

    pub fn error_building(self) -> Result<Option<OdsProvenance>> {
        match self {
            Self::Read(prov, _) => Ok(Some(*prov)),
            Self::Unreadable { path, date } => {
                anyhow::bail!("{}", format_unreadable_provenance_error(&path, &date));
            }
            Self::Absent => Ok(None),
        }
    }

    pub fn error_building_with_path(self) -> Result<Option<(OdsProvenance, PathBuf)>> {
        match self {
            Self::Read(prov, path) => Ok(Some((*prov, path))),
            Self::Unreadable { path, date } => {
                anyhow::bail!("{}", format_unreadable_provenance_error(&path, &date));
            }
            Self::Absent => Ok(None),
        }
    }
}

fn extract_date_for_unreadable_provenance(prov_path: &Path, content: Option<&str>) -> String {
    if let Some(s) = content {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(s) {
            if let Some(d) = val.get("trud_release_date").and_then(|d| d.as_str()) {
                if !d.is_empty() {
                    return d.to_string();
                }
            }
        }
    }
    if let Some(parent) = prov_path.parent() {
        if let Some(name) = parent.file_name().and_then(|n| n.to_str()) {
            if chrono::NaiveDate::parse_from_str(name, "%Y-%m-%d").is_ok() {
                return name.to_string();
            }
        }
    }
    "<date>".to_string()
}

impl OdsProvenance {
    pub fn load_from_file(prov_file: &Path) -> ProvenanceLoad {
        if !prov_file.exists() {
            return ProvenanceLoad::Absent;
        }
        let content = match std::fs::read_to_string(prov_file) {
            Ok(c) => c,
            Err(_) => {
                let date = extract_date_for_unreadable_provenance(prov_file, None);
                return ProvenanceLoad::Unreadable {
                    path: prov_file.to_path_buf(),
                    date,
                };
            }
        };

        match serde_json::from_str::<Self>(&content) {
            Ok(prov) => {
                if prov.schema == PROVENANCE_SCHEMA_V1_URL {
                    ProvenanceLoad::Read(Box::new(prov), prov_file.to_path_buf())
                } else {
                    let date = extract_date_for_unreadable_provenance(prov_file, Some(&content));
                    ProvenanceLoad::Unreadable {
                        path: prov_file.to_path_buf(),
                        date,
                    }
                }
            }
            Err(_) => {
                let date = extract_date_for_unreadable_provenance(prov_file, Some(&content));
                ProvenanceLoad::Unreadable {
                    path: prov_file.to_path_buf(),
                    date,
                }
            }
        }
    }

    pub fn load_from_dir_with_path(dir: &Path) -> ProvenanceLoad {
        let mut curr = if dir.is_file() {
            dir.parent().map(|p| p.to_path_buf())
        } else {
            Some(dir.to_path_buf())
        };

        for _ in 0..4 {
            if let Some(ref path) = curr {
                let prov_file = path.join(PROVENANCE_FILENAME);
                if prov_file.exists() {
                    match Self::load_from_file(&prov_file) {
                        ProvenanceLoad::Read(mut prov, prov_path) => {
                            if prov.trud_release_date.is_none() || prov.trud_release_file.is_none() {
                                if let Some(zip_prov) = Self::try_extract_trud_zip_provenance(path) {
                                    if prov.trud_release_date.is_none() { prov.trud_release_date = zip_prov.trud_release_date; }
                                    if prov.trud_release_file.is_none() { prov.trud_release_file = zip_prov.trud_release_file; }
                                    if prov.trud_release_name.is_none() { prov.trud_release_name = zip_prov.trud_release_name; }
                                    if prov.trud_release_sha256.is_none() { prov.trud_release_sha256 = zip_prov.trud_release_sha256; }
                                }
                            }
                            return ProvenanceLoad::Read(prov, prov_path);
                        }
                        ProvenanceLoad::Unreadable { path, date } => {
                            return ProvenanceLoad::Unreadable { path, date };
                        }
                        ProvenanceLoad::Absent => {}
                    }
                }
                curr = path.parent().map(|p| p.to_path_buf());
            } else {
                break;
            }
        }
        ProvenanceLoad::Absent
    }

    pub fn load_from_dir(dir: &Path) -> ProvenanceLoad {
        Self::load_from_dir_with_path(dir)
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
            schema: PROVENANCE_SCHEMA_V1_URL.to_string(),
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

pub fn update_provenance(output_dir: &Path, dataset_version: Option<&str>) -> Result<()> {
    let prov_path = output_dir.join(PROVENANCE_FILENAME);
    let mut prov = match OdsProvenance::load_from_file(&prov_path) {
        ProvenanceLoad::Read(p, _) => *p,
        ProvenanceLoad::Unreadable { path, date } => {
            anyhow::bail!("{}", format_unreadable_provenance_error(&path, &date));
        }
        ProvenanceLoad::Absent => {
            match OdsProvenance::load_from_dir(output_dir) {
                ProvenanceLoad::Read(p, _) => *p,
                ProvenanceLoad::Unreadable { path, date } => {
                    anyhow::bail!("{}", format_unreadable_provenance_error(&path, &date));
                }
                ProvenanceLoad::Absent => {
                    OdsProvenance::try_extract_trud_zip_provenance(output_dir).unwrap_or_default()
                }
            }
        }
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
    if let Ok(header) = crate::ods_xml::extract_manifest_header(output_dir) {
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

    prov.schema = PROVENANCE_SCHEMA_V1_URL.to_string();
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
        assert!(json.contains("https://ods.fyi/schema/provenance.v1.json"));
        assert!(json.contains("2026-07-31"));

        let parsed: OdsProvenance = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.publication_date, Some("2026-07-31".to_string()));
    }

    #[test]
    fn test_sanitize_trud_url() {
        let url = "https://isd.digital.nhs.uk/trud/api/v1/keys/SECRET123/items/341";
        let sanitized = sanitize_trud_url(url, Some("SECRET123"));
        assert_eq!(sanitized, "https://isd.digital.nhs.uk/trud/api/v1/keys/<REDACTED_API_KEY>/items/341");
    }

    #[test]
    fn test_to_parquet_declared_metadata_pins_exact_eight_keys() {
        let mut prov = OdsProvenance::default();
        prov.trud_release_date = Some("2026-07-31".to_string());
        prov.trud_release_name = Some("Release 7.0.0".to_string());
        prov.trud_release_file = Some("hscorgrefdataxml_data_7.0.0_20260731000001.zip".to_string());
        prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());
        prov.publication_date = Some("2026-07-28".to_string());
        prov.publication_seq_num = Some("4700".to_string());
        prov.publication_type = Some("Full".to_string());
        prov.publication_source = Some("HSCIC".to_string());
        // Populate dataset_version and tool fields to verify they are strictly excluded
        prov.dataset_version = Some("0.1.0".to_string());
        prov.tool_version = Some("0.1.0".to_string());
        prov.tool_git_sha = Some("abcdef123456".to_string());
        prov.tool_git_dirty = Some(true);
        prov.trud_release_sha256_verified = Some(TrudVerificationSource::TrudApi);

        let meta = prov.to_parquet_declared_metadata();
        let expected_keys = vec![
            "ods.publication_date",
            "ods.publication_seq_num",
            "ods.publication_source",
            "ods.publication_type",
            "ods.trud_release_date",
            "ods.trud_release_file",
            "ods.trud_release_name",
            "ods.trud_release_sha256",
        ];
        let actual_keys: Vec<&String> = meta.keys().collect();
        assert_eq!(actual_keys, expected_keys);
    }
}

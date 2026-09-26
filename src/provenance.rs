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

/// What `_provenance.json` holds: facts about NHS's archive and nothing about who built the
/// dataset or how the archive was checked, so a dataset's manifest digest is a function of the
/// archive and the dataset version. The `ods` that built a published dataset is recorded in the
/// release index, and the check that vouched for the archive is shown when it runs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdsProvenance {
    /// JSON Schema URI identifying this provenance document format
    #[serde(rename = "$schema")]
    pub schema: String,

    // --- Official TRUD API Release Metadata (trud_*) ---
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trud_release_date: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub trud_release_sha256: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub trud_release_filesize_bytes: Option<u64>,

    // --- Terms ---
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribution: Option<String>,
}

pub const PROVENANCE_FILENAME: &str = "_provenance.json";

impl Default for OdsProvenance {
    fn default() -> Self {
        Self {
            schema: PROVENANCE_SCHEMA_V1_URL.to_string(),
            trud_release_date: None,
            trud_release_sha256: None,
            trud_release_filesize_bytes: None,
            license: None,
            attribution: None,
        }
    }
}

impl OdsProvenance {
    pub fn from_trud_statement(date: &str, sha256: &str, filesize_bytes: u64) -> Self {
        Self {
            schema: PROVENANCE_SCHEMA_V1_URL.to_string(),
            trud_release_date: Some(date.to_string()),
            trud_release_sha256: Some(sha256.to_string()),
            trud_release_filesize_bytes: Some(filesize_bytes),
            license: Some(crate::terms::LICENSE.to_string()),
            attribution: Some(crate::terms::ATTRIBUTION.to_string()),
        }
    }

    pub fn to_json_string(&self) -> Result<String> {
        serde_json::to_string_pretty(self).map_err(Into::into)
    }

    pub fn write_to_dir(&self, release_dir: &Path) -> Result<()> {
        let prov_path = release_dir.join(PROVENANCE_FILENAME);
        let json = self.to_json_string()?;
        std::fs::write(&prov_path, json).with_context(|| format!("writing {}", prov_path.display()))?;
        Ok(())
    }

    pub fn has_current_terms(&self) -> bool {
        self.license.as_deref() == Some(crate::terms::LICENSE)
            && self.attribution.as_deref() == Some(crate::terms::ATTRIBUTION)
    }

    /// Declared Parquet key-value metadata subset:
    /// - ods.trud_release_date
    /// - ods.trud_release_sha256
    ///
    /// Nothing else is carried, so Parquet bytes are a function of the source archive and
    /// survive a dataset relabel or tool re-tag.
    pub fn to_parquet_declared_metadata(&self) -> std::collections::BTreeMap<String, String> {
        let mut meta = std::collections::BTreeMap::new();
        if let Some(ref d) = self.trud_release_date {
            meta.insert("ods.trud_release_date".to_string(), d.clone());
        }
        if let Some(ref s) = self.trud_release_sha256 {
            meta.insert("ods.trud_release_sha256".to_string(), s.clone());
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

pub fn format_no_provenance_error(dir: &Path) -> String {
    let dir_display = crate::workspace::relative_to_cwd(dir);
    format!(
        "✖ {} has no provenance: it was built from an archive ods couldn't match to a TRUD release\n  To cite or publish it, get the archive through ods trud pull.",
        dir_display.display()
    )
}

pub fn format_older_terms_error(prov_path: &Path, date: &str) -> String {
    let disp = format_provenance_display_path(prov_path);
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

    pub fn warn_reading(self, dir: &Path) -> Option<OdsProvenance> {
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
            Self::Absent => {
                let disp = crate::workspace::relative_to_cwd(dir);
                eprintln!(
                    "! {} has no provenance: it was built from an archive ods couldn't match to a TRUD release",
                    disp.display()
                );
                None
            }
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
                        ProvenanceLoad::Read(prov, prov_path) => {
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

    pub fn validate_baseline(&self) -> Result<()> {
        let date = self.trud_release_date.as_deref().unwrap_or("");
        if date.is_empty() {
            anyhow::bail!("Missing trud_release_date in _provenance.json");
        }
        if self.trud_release_sha256.as_deref().unwrap_or("").is_empty() {
            anyhow::bail!("Missing trud_release_sha256 in _provenance.json");
        }

        Ok(())
    }

    pub fn validate_publishable(&self) -> Vec<String> {
        let mut failures = Vec::new();

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

pub fn write_provenance(
    release_dir: &Path,
    date: &str,
    sha256: &str,
    filesize_bytes: u64,
) -> Result<OdsProvenance> {
    let prov = OdsProvenance::from_trud_statement(date, sha256, filesize_bytes);
    prov.write_to_dir(release_dir)?;
    Ok(prov)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provenance_serde() {
        let prov = OdsProvenance::from_trud_statement(
            "2026-07-31",
            "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
            38064419,
        );

        let json = serde_json::to_string(&prov).unwrap();
        assert!(json.contains("https://ods.fyi/schema/provenance.v1.json"));
        assert!(json.contains("2026-07-31"));

        let parsed: OdsProvenance = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.trud_release_date, Some("2026-07-31".to_string()));
    }

    #[test]
    fn test_sanitize_trud_url() {
        let url = "https://isd.digital.nhs.uk/trud/api/v1/keys/SECRET123/items/341";
        let sanitized = sanitize_trud_url(url, Some("SECRET123"));
        assert_eq!(sanitized, "https://isd.digital.nhs.uk/trud/api/v1/keys/<REDACTED_API_KEY>/items/341");
    }

    #[test]
    fn test_to_parquet_declared_metadata_pins_exact_two_keys() {
        let prov = OdsProvenance {
            trud_release_date: Some("2026-07-31".to_string()),
            trud_release_sha256: Some(
                "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string(),
            ),
            ..Default::default()
        };

        // The Parquet files carry two metadata keys: ods.trud_release_date and ods.trud_release_sha256.
        // The rule is nothing that changes when a release is relabelled or the tool is re-tagged.
        let meta = prov.to_parquet_declared_metadata();
        let expected_keys = vec![
            "ods.trud_release_date",
            "ods.trud_release_sha256",
        ];
        let actual_keys: Vec<&String> = meta.keys().collect();
        assert_eq!(actual_keys, expected_keys);
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

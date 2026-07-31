use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const NDJSON_TYPE_TAG: &str = "ods_provenance";

fn default_type_tag() -> String {
    NDJSON_TYPE_TAG.to_string()
}

/// Unified dataset and build provenance metadata emitted as line 1 of canonical `ods.ndjson`.
/// Combines TRUD source XML metadata (publication date, sequence number, version) with
/// tool compilation provenance (ods version, timestamp, source path).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdsProvenance {
    /// Discriminator tag distinguishing provenance header from organisation records
    #[serde(rename = "_type", default = "default_type_tag")]
    pub type_tag: String,

    // --- TRUD Source Data Metadata ---
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publication_date: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub publication_seq_num: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub publication_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub publication_source: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub xml_version: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub xml_file_creation_date_time: Option<String>,

    // --- Tool Provenance Metadata ---
    pub ods_version: String,
    pub compiled_at: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
}

impl OdsProvenance {
    pub fn new(
        publication_date: Option<String>,
        publication_seq_num: Option<String>,
        publication_type: Option<String>,
        publication_source: Option<String>,
        xml_version: Option<String>,
        xml_file_creation_date_time: Option<String>,
        source_path: Option<&Path>,
    ) -> Self {
        Self {
            type_tag: NDJSON_TYPE_TAG.to_string(),
            publication_date,
            publication_seq_num,
            publication_type,
            publication_source,
            xml_version,
            xml_file_creation_date_time,
            ods_version: env!("CARGO_PKG_VERSION").to_string(),
            compiled_at: Utc::now().to_rfc3339(),
            source_path: source_path.and_then(|p| p.file_name()).map(|f| f.to_string_lossy().to_string()),
        }
    }
}

/// Helper to parse line 1 of an NDJSON file if it is an `ods_provenance` header.
pub fn try_parse_provenance_line(line: &str) -> Option<OdsProvenance> {
    if line.contains("\"_type\":\"ods_provenance\"") || line.contains("\"_type\": \"ods_provenance\"") {
        serde_json::from_str::<OdsProvenance>(line).ok()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provenance_serde() {
        let prov = OdsProvenance::new(
            Some("2026-06-22".to_string()),
            Some("4631".to_string()),
            Some("Full".to_string()),
            Some("HSCIC".to_string()),
            Some("2-0-0".to_string()),
            Some("2026-06-22T16:28:38".to_string()),
            Some(Path::new("HSCOrgRefData.xml")),
        );

        let json = serde_json::to_string(&prov).unwrap();
        assert!(json.contains("\"_type\":\"ods_provenance\""));
        assert!(json.contains("\"publication_date\":\"2026-06-22\""));

        let parsed = try_parse_provenance_line(&json).unwrap();
        assert_eq!(parsed.publication_date, Some("2026-06-22".to_string()));
    }
}

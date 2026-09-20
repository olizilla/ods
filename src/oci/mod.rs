use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MEDIA_TYPE_MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";
pub const MEDIA_TYPE_INDEX: &str = "application/vnd.oci.image.index.v1+json";
pub const ARTIFACT_TYPE_DATASET: &str = "application/vnd.fyi.ods.dataset.v1";
pub const MEDIA_TYPE_PROVENANCE: &str = "application/vnd.fyi.ods.provenance.v1+json";
pub const MEDIA_TYPE_PARQUET: &str = "application/vnd.apache.parquet";
pub const MEDIA_TYPE_JSON: &str = "application/json";
pub const MEDIA_TYPE_TEXT_PLAIN: &str = "text/plain";
pub const MEDIA_TYPE_MARKDOWN: &str = "text/markdown";

pub const ANNOTATION_TITLE: &str = "org.opencontainers.image.title";
pub const ANNOTATION_REF_NAME: &str = "org.opencontainers.image.ref.name";
pub const ANNOTATION_CREATED: &str = "org.opencontainers.image.created";
pub const ANNOTATION_LICENSES: &str = "org.opencontainers.image.licenses";
pub const ANNOTATION_SOURCE: &str = "org.opencontainers.image.source";
pub const ANNOTATION_VERSION: &str = "org.opencontainers.image.version";

pub const ANNOTATION_FYI_TRUD_RELEASE_DATE: &str = "fyi.ods.trud-release-date";
pub const ANNOTATION_FYI_DATASET_VERSION: &str = "fyi.ods.dataset-version";
pub const ANNOTATION_FYI_TRUD_RELEASE_SHA256: &str = "fyi.ods.trud-release-sha256";
pub const ANNOTATION_FYI_TOOL_VERSION: &str = "fyi.ods.tool-version";
pub const ANNOTATION_FYI_TOOL_GIT_SHA: &str = "fyi.ods.tool-git-sha";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OciDescriptor {
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub digest: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<BTreeMap<String, String>>,
}

impl OciDescriptor {
    pub fn new(media_type: &str, digest: &str, size: u64) -> Self {
        Self {
            media_type: media_type.to_string(),
            digest: digest.to_lowercase(),
            size,
            annotations: None,
        }
    }

    pub fn with_title(mut self, title: &str) -> Self {
        let mut ann = self.annotations.unwrap_or_default();
        ann.insert(ANNOTATION_TITLE.to_string(), title.to_string());
        self.annotations = Some(ann);
        self
    }
}

/// The OCI Image Manifest representing a complete release.
/// Field order is strict: schemaVersion, mediaType, artifactType, config, layers, annotations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OciManifest {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    #[serde(rename = "mediaType")]
    pub media_type: String,
    #[serde(rename = "artifactType")]
    pub artifact_type: String,
    pub config: OciDescriptor,
    pub layers: Vec<OciDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<BTreeMap<String, String>>,
}

impl OciManifest {
    /// Sorts layers in-place by title in ASCII order.
    pub fn sort_layers_by_title(&mut self) {
        self.layers.sort_by(|a, b| {
            let title_a = a
                .annotations
                .as_ref()
                .and_then(|ann| ann.get(ANNOTATION_TITLE))
                .map(|s| s.as_str())
                .unwrap_or("");
            let title_b = b
                .annotations
                .as_ref()
                .and_then(|ann| ann.get(ANNOTATION_TITLE))
                .map(|s| s.as_str())
                .unwrap_or("");
            title_a.cmp(title_b)
        });
    }

    /// Serializes the manifest into compact JSON without extra whitespace or trailing newline.
    pub fn to_canonical_bytes(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(self).context("serialising OCI manifest to canonical bytes")
    }

    /// Computes the manifest digest in standard lowercase `sha256:<hex>` format.
    pub fn digest(&self) -> Result<String> {
        use sha2::{Digest, Sha256};
        let bytes = self.to_canonical_bytes()?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let hex = format!("{:x}", hasher.finalize());
        Ok(format!("sha256:{}", hex))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OciIndex {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub manifests: Vec<OciIndexManifestEntry>,
}

impl OciIndex {
    pub fn to_canonical_bytes(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(self).context("serialising OCI index to canonical bytes")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OciIndexManifestEntry {
    #[serde(rename = "mediaType")]
    pub media_type: String,
    #[serde(rename = "artifactType")]
    pub artifact_type: String,
    pub digest: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OciLayout {
    #[serde(rename = "imageLayoutVersion")]
    pub image_layout_version: String,
}

impl Default for OciLayout {
    fn default() -> Self {
        Self {
            image_layout_version: "1.0.0".to_string(),
        }
    }
}

impl OciLayout {
    pub fn to_canonical_bytes(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(self).context("serialising oci-layout to canonical bytes")
    }
}

/// Derives media type for a given file name in the release directory.
pub fn media_type_for_file(filename: &str) -> &'static str {
    if filename.ends_with(".parquet") {
        MEDIA_TYPE_PARQUET
    } else if filename == crate::provenance::PROVENANCE_FILENAME || filename == "provenance.json" {
        MEDIA_TYPE_PROVENANCE
    } else if filename.ends_with(".json") {
        MEDIA_TYPE_JSON
    } else if filename.ends_with(".md") {
        MEDIA_TYPE_MARKDOWN
    } else if filename.ends_with(".txt") {
        MEDIA_TYPE_TEXT_PLAIN
    } else {
        "application/octet-stream"
    }
}

/// Derives `org.opencontainers.image.created` timestamp strictly from `trud_release_date`.
/// Never calls wall clock APIs.
pub fn derive_created_timestamp(release_date: &str) -> String {
    format!("{}T00:00:00Z", release_date)
}

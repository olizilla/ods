//! A dataset's OCI manifest: a pure function of its Parquet files.
//!
//! The config is the fixed empty descriptor (`application/vnd.oci.empty.v1+json`),
//! `artifactType` is `application/vnd.fyi.ods.dataset.v1`, the layers are the Parquet files, and
//! the annotations are the image spec's own keys, derived from the object every file carries
//! under its `datapackage` key-value metadata (`crate::provenance`). The source's facts stay in
//! the files. Nothing else in a release directory contributes, so
//! a directory holding only the Parquet files rebuilds the manifest, and `ods` verifies a
//! release by rebuilding it and comparing its digest with the release index. No stored `oci/`
//! is read; `ods make oci` writes one only for publishing.

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use super::source::MEDIA_TYPE_EMPTY;
use super::*;
use crate::provenance::{compute_file_sha256, ReleaseFacts, ReleaseRecord};

/// OCI's empty descriptor content: the two bytes `{}`.
pub const EMPTY_CONFIG: &[u8] = b"{}";

/// The repository `ods` publishes from.
pub const SOURCE_REPOSITORY: &str = "https://github.com/olizilla/ods";

fn empty_config_descriptor() -> OciDescriptor {
    let digest = format!("sha256:{:x}", Sha256::digest(EMPTY_CONFIG));
    OciDescriptor::new(MEDIA_TYPE_EMPTY, &digest, EMPTY_CONFIG.len() as u64)
}

/// Whether `config` is exactly the fixed empty descriptor a dataset manifest always carries.
pub fn is_empty_config(config: &OciDescriptor) -> bool {
    *config == empty_config_descriptor()
}

/// The manifest's annotations: the image spec's own keys, derived from the embedded object.
/// `description` is the dataset's summary, then the licence's attribution.
fn annotations(facts: &ReleaseFacts) -> BTreeMap<String, String> {
    let license = facts.license();
    let mut annotations = BTreeMap::new();
    annotations.insert(ANNOTATION_TITLE.to_string(), facts.embedded.name.clone());
    annotations.insert(ANNOTATION_VERSION.to_string(), facts.version().to_string());
    annotations.insert(
        ANNOTATION_DESCRIPTION.to_string(),
        format!("{} {}", crate::datapackage::SUMMARY, license.attribution),
    );
    annotations.insert(ANNOTATION_LICENSES.to_string(), license.name.clone());
    // RFC 3339 date-time, as the image spec defines the key: midnight UTC on the source's date.
    annotations.insert(ANNOTATION_CREATED.to_string(), derive_created_timestamp(&facts.source.version));
    annotations.insert(ANNOTATION_SOURCE.to_string(), SOURCE_REPOSITORY.to_string());
    annotations
}

/// The manifest `release_dir`'s top-level `*.parquet` files (sorted by name) make, with
/// annotations from `facts`, the embedded object they carry. Returns it and its canonical bytes.
pub fn build(release_dir: &Path, facts: &ReleaseFacts) -> Result<(OciManifest, Vec<u8>)> {
    let names = crate::provenance::release_parquet_files(release_dir)?;
    let mut layers = Vec::with_capacity(names.len());
    for name in &names {
        let path = release_dir.join(name);
        let size = fs::metadata(&path).with_context(|| format!("reading {}", path.display()))?.len();
        let hash = compute_file_sha256(&path)?.to_lowercase();
        layers.push(OciDescriptor::new(MEDIA_TYPE_PARQUET, &format!("sha256:{}", hash), size).with_title(name));
    }

    let manifest = OciManifest {
        schema_version: 2,
        media_type: MEDIA_TYPE_MANIFEST.to_string(),
        artifact_type: ARTIFACT_TYPE_DATASET.to_string(),
        config: empty_config_descriptor(),
        layers,
        annotations: Some(annotations(facts)),
    };
    let bytes = manifest.to_canonical_bytes()?;
    Ok((manifest, bytes))
}

/// Reads the embedded object from `release_dir`'s files and builds the manifest they make.
/// Refuses a release whose files carry no provenance: there is nothing to annotate it with.
pub fn build_from_dir(release_dir: &Path) -> Result<(OciManifest, Vec<u8>, ReleaseFacts)> {
    let record = crate::provenance::read_release(release_dir)?;
    let ReleaseRecord::Provenanced(facts) = record else {
        anyhow::bail!(
            "{}",
            crate::provenance::format_record_refusal(release_dir, &record).unwrap_or_default()
        );
    };
    let (manifest, bytes) = build(release_dir, &facts)?;
    Ok((manifest, bytes, *facts))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_annotations_are_the_embedded_objects_facts() {
        let record = crate::provenance::PullRecord::for_trud_release(
            "2026-09-25",
            "hscorgrefdataxml_data_8.0.0_20260925000001.zip",
            "CA0FEE7512F593ADA1FA9B95BF1372B41911167DA463A98FECF33ADFD86697E5",
            38138574,
            &[],
        )
        .unwrap();
        let embedded = record.embedded("0.1.0").unwrap();
        let tmp = tempfile::tempdir().unwrap();
        crate::commands::parquet::write_stub_parquet(&tmp.path().join("orgs.parquet"), Some(&embedded), "orgs").unwrap();
        let (manifest, _, facts) = build_from_dir(tmp.path()).unwrap();
        assert_eq!(facts.dataset_version, "0.1.0");
        let ann = manifest.annotations.unwrap();
        let got: Vec<(&str, &str)> = ann.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(
            got,
            vec![
                ("org.opencontainers.image.created", "2026-09-25T00:00:00Z"),
                (
                    "org.opencontainers.image.description",
                    "All the organisations and sites in the NHS Organisation Data Service, as queryable & verifiable Parquet files. Contains information from NHS England, licensed under the current version of the Open Government Licence."
                ),
                ("org.opencontainers.image.licenses", "OGL-UK-3.0"),
                ("org.opencontainers.image.source", "https://github.com/olizilla/ods"),
                ("org.opencontainers.image.title", "ods-data"),
                ("org.opencontainers.image.version", "2026-09-25_0.1.0"),
            ]
        );
    }

    #[test]
    fn test_build_from_dir_refuses_a_release_without_provenance() {
        let tmp = tempfile::tempdir().unwrap();
        crate::commands::parquet::write_stub_parquet(&tmp.path().join("orgs.parquet"), None, "orgs").unwrap();
        let err = build_from_dir(tmp.path()).unwrap_err();
        assert!(format!("{:#}", err).contains("carry no provenance"), "{err:#}");
    }
}

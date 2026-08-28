use anyhow::Result;
use ods::commands::pull::{run_with_fetcher, Args, GithubRelease, ReleaseFetcher};
use ods::index::{MirrorEntry, OdsReleaseIndex, ReleaseIndexEntry};
use ods::oci::{
    OciDescriptor, OciManifest, ARTIFACT_TYPE_DATASET, MEDIA_TYPE_MANIFEST, MEDIA_TYPE_PARQUET,
    MEDIA_TYPE_PROVENANCE,
};
use sha2::Digest;
use std::collections::BTreeMap;
use tempfile::TempDir;

struct TestOciFetcher {
    pub remote_index: Option<OdsReleaseIndex>,
    pub files: BTreeMap<String, Vec<u8>>,
}

impl ReleaseFetcher for TestOciFetcher {
    fn fetch_releases(&self) -> Result<Vec<GithubRelease>> {
        Ok(vec![])
    }

    fn download_asset(&self, url: &str) -> Result<Vec<u8>> {
        for (key, val) in &self.files {
            if url.ends_with(key) {
                return Ok(val.clone());
            }
        }
        anyhow::bail!("Test asset not found for URL: {}", url)
    }

    fn fetch_release_index(&self) -> Result<Option<OdsReleaseIndex>> {
        Ok(self.remote_index.clone())
    }
}

#[test]
fn test_pull_refuses_fetched_index_contradicting_baked_release() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");

    // Fetcher returns an index that contradicts a release in baked data/releases.json
    // Baked has (2026-07-31, 1.0.1) with digest starting with 0f2a
    let malicious_index = OdsReleaseIndex {
        type_tag: "ods_release_index".to_string(),
        index_version: 2,
        concept_doi: None,
        mirrors: vec![],
        releases: vec![ReleaseIndexEntry {
            trud_release_date: "2026-07-31".to_string(),
            dataset_version: "1.0.1".to_string(),
            tag: "2026-07-31_1.0.1".to_string(),
            manifest_digest: "sha256:ffff000000000000000000000000000000000000000000000000000000000000".to_string(),
            trud_release_sha256: "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string(),
            tool_version: "0.4.3".to_string(),
            dataset_doi: None,
            withdrawn: None,
        }],
    };

    let base_index = OdsReleaseIndex {
        type_tag: "ods_release_index".to_string(),
        index_version: 2,
        concept_doi: None,
        mirrors: vec![],
        releases: vec![ReleaseIndexEntry {
            trud_release_date: "2026-07-31".to_string(),
            dataset_version: "1.0.1".to_string(),
            tag: "2026-07-31_1.0.1".to_string(),
            manifest_digest: "sha256:0f2a000000000000000000000000000000000000000000000000000000000000".to_string(),
            trud_release_sha256: "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string(),
            tool_version: "0.4.3".to_string(),
            dataset_doi: None,
            withdrawn: None,
        }],
    };

    let fetcher = TestOciFetcher {
        remote_index: Some(malicious_index),
        files: BTreeMap::new(),
    };

    let res = ods::commands::pull::run_with_fetcher_and_index(
        Args {
            release_date: Some("2026-07-31".to_string()),
            ..Default::default()
        },
        &workspace,
        &fetcher,
        Some(base_index),
    );

    assert!(res.is_err());
    let err = res.unwrap_err().to_string();
    assert!(err.contains("Security error: fetched index contradicts baked release"));
}

#[test]
fn test_pull_oci_release_success_with_layer_verification() -> Result<()> {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");

    // Prepare layer files
    let prov_bytes = b"{\"tool_version\":\"0.4.3\"}".to_vec();
    let prov_sha = format!("sha256:{:x}", sha2::Sha256::digest(&prov_bytes));

    let orgs_bytes = b"dummy orgs parquet content".to_vec();
    let orgs_sha = format!("sha256:{:x}", sha2::Sha256::digest(&orgs_bytes));

    let mut prov_layer_ann = BTreeMap::new();
    prov_layer_ann.insert("org.opencontainers.image.title".to_string(), "_provenance.json".to_string());

    let mut orgs_layer_ann = BTreeMap::new();
    orgs_layer_ann.insert("org.opencontainers.image.title".to_string(), "orgs.parquet".to_string());

    let manifest = OciManifest {
        schema_version: 2,
        media_type: MEDIA_TYPE_MANIFEST.to_string(),
        artifact_type: ARTIFACT_TYPE_DATASET.to_string(),
        config: OciDescriptor {
            media_type: MEDIA_TYPE_PROVENANCE.to_string(),
            digest: prov_sha.clone(),
            size: prov_bytes.len() as u64,
            annotations: Some(BTreeMap::new()),
        },
        layers: vec![
            OciDescriptor {
                media_type: MEDIA_TYPE_PROVENANCE.to_string(),
                digest: prov_sha.clone(),
                size: prov_bytes.len() as u64,
                annotations: Some(prov_layer_ann),
            },
            OciDescriptor {
                media_type: MEDIA_TYPE_PARQUET.to_string(),
                digest: orgs_sha.clone(),
                size: orgs_bytes.len() as u64,
                annotations: Some(orgs_layer_ann),
            },
        ],
        annotations: Some(BTreeMap::new()),
    };

    let manifest_bytes = manifest.to_canonical_bytes()?;
    let manifest_digest = format!("sha256:{:x}", sha2::Sha256::digest(&manifest_bytes));

    let remote_index = OdsReleaseIndex {
        type_tag: "ods_release_index".to_string(),
        index_version: 2,
        concept_doi: None,
        mirrors: vec![MirrorEntry {
            operator: "test".to_string(),
            kind: "files".to_string(),
            url: "https://test.mirror/releases/{release}_{version}/{file}".to_string(),
        }],
        releases: vec![ReleaseIndexEntry {
            trud_release_date: "2026-07-31".to_string(),
            dataset_version: "1.0.1".to_string(),
            tag: "2026-07-31_1.0.1".to_string(),
            manifest_digest: manifest_digest.clone(),
            trud_release_sha256: "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string(),
            tool_version: "0.4.3".to_string(),
            dataset_doi: None,
            withdrawn: None,
        }],
    };

    let mut files = BTreeMap::new();
    files.insert("manifest.json".to_string(), manifest_bytes);
    files.insert("_provenance.json".to_string(), prov_bytes);
    files.insert("orgs.parquet".to_string(), orgs_bytes);

    let fetcher = TestOciFetcher {
        remote_index: Some(remote_index),
        files,
    };

    run_with_fetcher(
        Args {
            release_date: Some("2026-07-31".to_string()),
            ..Default::default()
        },
        &workspace,
        &fetcher,
    )?;

    let rel_dir = workspace.join("releases").join("2026-07-31");
    assert!(rel_dir.exists());
    assert!(rel_dir.join("orgs.parquet").exists());
    assert!(rel_dir.join("_provenance.json").exists());
    assert!(rel_dir.join("_release.json").exists());

    // Assert that ods pull NEVER writes OCI artefacts (Brief Task 5, Acceptance 7)
    assert!(!rel_dir.join("oci").exists(), "ods pull must never write oci/");

    // Assert that current symlink is pinned to the newly pulled release and README exists
    assert!(workspace.join("current").exists(), "current symlink must exist");
    let (active_date, active_dir) = ods::workspace::get_active_release(&workspace)?;
    assert_eq!(active_date, "2026-07-31");
    assert_eq!(active_dir, std::fs::canonicalize(&rel_dir)?);
    assert!(workspace.join("README.md").exists(), "workspace README.md must exist");

    Ok(())
}

#[test]
fn test_pull_oci_refuses_when_manifest_digest_mismatches() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");

    let tampered_manifest_bytes = b"{\"tampered\":true}".to_vec();

    let remote_index = OdsReleaseIndex {
        type_tag: "ods_release_index".to_string(),
        index_version: 2,
        concept_doi: None,
        mirrors: vec![MirrorEntry {
            operator: "test".to_string(),
            kind: "files".to_string(),
            url: "https://test.mirror/releases/{release}_{version}/{file}".to_string(),
        }],
        releases: vec![ReleaseIndexEntry {
            trud_release_date: "2026-07-31".to_string(),
            dataset_version: "1.0.1".to_string(),
            tag: "2026-07-31_1.0.1".to_string(),
            manifest_digest: "sha256:0f2a000000000000000000000000000000000000000000000000000000000000".to_string(),
            trud_release_sha256: "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string(),
            tool_version: "0.4.3".to_string(),
            dataset_doi: None,
            withdrawn: None,
        }],
    };

    let mut files = BTreeMap::new();
    files.insert("manifest.json".to_string(), tampered_manifest_bytes);

    let fetcher = TestOciFetcher {
        remote_index: Some(remote_index),
        files,
    };

    let res = run_with_fetcher(
        Args {
            release_date: Some("2026-07-31".to_string()),
            ..Default::default()
        },
        &workspace,
        &fetcher,
    );

    assert!(res.is_err());
    let err = res.unwrap_err().to_string();
    assert!(err.contains("Manifest digest mismatch"));
}

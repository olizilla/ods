mod common;

use anyhow::Result;
use common::make_v1_index;
use ods::commands::pull::{run_with_fetcher, Args, OciBlobFetcher};
use ods::index::OdsReleaseIndex;
use sha2::Digest;
use std::collections::BTreeMap;
use tempfile::TempDir;

struct MockOciFetcher {
    pub remote_index: Option<OdsReleaseIndex>,
    pub responses: BTreeMap<String, Vec<u8>>,
}

impl OciBlobFetcher for MockOciFetcher {
    fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>> {
        for (key, val) in &self.responses {
            if url == key || url.ends_with(key) {
                return Ok(val.clone());
            }
        }
        anyhow::bail!("Mock asset not found for URL: {}", url)
    }

    fn fetch_release_index(&self) -> Result<Option<OdsReleaseIndex>> {
        Ok(self.remote_index.clone())
    }
}

fn create_mock_oci_dataset(tmp_dir: &std::path::Path) -> (OdsReleaseIndex, BTreeMap<String, Vec<u8>>) {
    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);
    let prov_bytes = serde_json::to_vec_pretty(&prov).unwrap();
    let prov_sha = format!("sha256:{:x}", sha2::Sha256::digest(&prov_bytes));

    let orgs_bytes = b"mock orgs parquet content".to_vec();
    let orgs_sha = format!("sha256:{:x}", sha2::Sha256::digest(&orgs_bytes));

    let fixture_dir = tmp_dir.join("fixture");
    std::fs::create_dir_all(&fixture_dir).unwrap();
    std::fs::write(fixture_dir.join("orgs.parquet"), &orgs_bytes).unwrap();
    std::fs::write(fixture_dir.join(ods::provenance::PROVENANCE_FILENAME), &prov_bytes).unwrap();

    let dp = serde_json::json!({
        "name": "ods",
        "version": "1.0.1",
        "resources": []
    });
    let dp_bytes = serde_json::to_vec_pretty(&dp).unwrap();
    let dp_sha = format!("sha256:{:x}", sha2::Sha256::digest(&dp_bytes));
    std::fs::write(fixture_dir.join(ods::datapackage::DATAPACKAGE_FILENAME), &dp_bytes).unwrap();

    let (manifest, manifest_bytes) = ods::commands::make_oci::build_manifest_from_dir(&fixture_dir, &prov, "1.0.1").unwrap();
    let manifest_digest = manifest.digest().unwrap();

    let index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("1.0.1", &manifest_digest)],
    )]);

    let mut responses = BTreeMap::new();
    responses.insert(format!("manifests/{}", manifest_digest), manifest_bytes);
    responses.insert(format!("blobs/{}", prov_sha), prov_bytes);
    responses.insert(format!("blobs/{}", orgs_sha), orgs_bytes);
    responses.insert(format!("blobs/{}", dp_sha), dp_bytes);

    (index, responses)
}

#[test]
fn test_pull_latest_release_downloads_verifies_and_links() -> Result<()> {
    let tmp = TempDir::new()?;
    let (index, responses) = create_mock_oci_dataset(tmp.path());
    let fetcher = MockOciFetcher {
        remote_index: Some(index),
        responses,
    };

    let workspace_root = tmp.path().join("ods_data");

    run_with_fetcher(Args::default(), &workspace_root, &fetcher)?;

    let current_dir = workspace_root.join("current");
    assert!(current_dir.exists(), "ods_data/current symlink must exist");
    assert!(
        current_dir.join("orgs.parquet").exists(),
        "orgs.parquet must exist in current"
    );
    assert!(
        current_dir.join("_provenance.json").exists(),
        "_provenance.json must exist in current"
    );
    assert!(
        !current_dir.join("SHA256SUMS").exists(),
        "SHA256SUMS must not exist in current"
    );
    assert!(
        !current_dir.join("oci").exists(),
        "oci/ must not exist in current"
    );

    Ok(())
}

#[test]
fn test_pull_idempotent_cache_hit() -> Result<()> {
    let tmp = TempDir::new()?;
    let (index, responses) = create_mock_oci_dataset(tmp.path());
    let fetcher = MockOciFetcher {
        remote_index: Some(index),
        responses,
    };

    let workspace_root = tmp.path().join("ods_data");

    run_with_fetcher(Args::default(), &workspace_root, &fetcher)?;
    // Second run should use cached release without re-downloading
    run_with_fetcher(Args::default(), &workspace_root, &fetcher)?;

    Ok(())
}

#[test]
fn test_pull_list_format_json() -> Result<()> {
    let tmp = TempDir::new()?;
    let (index, responses) = create_mock_oci_dataset(tmp.path());
    let fetcher = MockOciFetcher {
        remote_index: Some(index),
        responses,
    };

    let workspace_root = tmp.path().join("ods_data");

    let args = Args {
        list: true,
        format: Some("json".to_string()),
        ..Default::default()
    };

    run_with_fetcher(args, &workspace_root, &fetcher)?;
    Ok(())
}

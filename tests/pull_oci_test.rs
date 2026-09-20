mod common;

use anyhow::Result;
use common::make_v1_index;
use ods::commands::pull::{run_with_fetcher, Args, OciBlobFetcher};
use ods::index::{MirrorEntry, OdsReleaseIndex};
use sha2::Digest;
use std::collections::BTreeMap;
use tempfile::TempDir;

struct TestOciFetcher {
    pub remote_index: Option<OdsReleaseIndex>,
    pub responses: BTreeMap<String, Vec<u8>>,
}

impl OciBlobFetcher for TestOciFetcher {
    fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>> {
        for (key, val) in &self.responses {
            if url == key || url.ends_with(key) {
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
fn test_pull_oci_release_success_with_layer_verification() -> Result<()> {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");

    // Prepare layer files
    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);
    let prov_bytes = serde_json::to_vec_pretty(&prov)?;
    let prov_sha = format!("sha256:{:x}", sha2::Sha256::digest(&prov_bytes));

    let orgs_bytes = b"dummy orgs parquet content".to_vec();
    let orgs_sha = format!("sha256:{:x}", sha2::Sha256::digest(&orgs_bytes));

    let dp = serde_json::json!({
        "name": "ods",
        "version": "1.0.1",
        "resources": []
    });
    let dp_bytes = serde_json::to_vec_pretty(&dp)?;
    let dp_sha = format!("sha256:{:x}", sha2::Sha256::digest(&dp_bytes));

    let fixture_dir = tmp.path().join("fixture_1");
    std::fs::create_dir_all(&fixture_dir)?;
    std::fs::write(fixture_dir.join("orgs.parquet"), &orgs_bytes)?;
    std::fs::write(fixture_dir.join(ods::provenance::PROVENANCE_FILENAME), &prov_bytes)?;
    std::fs::write(fixture_dir.join(ods::datapackage::DATAPACKAGE_FILENAME), &dp_bytes)?;

    let (manifest, manifest_bytes) = ods::commands::make_oci::build_manifest_from_dir(&fixture_dir, &prov, "1.0.1")?;
    let manifest_digest = manifest.digest()?;

    let remote_index = make_v1_index(&[(
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

    let fetcher = TestOciFetcher {
        remote_index: Some(remote_index),
        responses,
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

    // Assert that ods pull NEVER writes OCI artefacts or _release.json or SHA256SUMS
    assert!(!rel_dir.join("oci").exists(), "ods pull must never write oci/");
    assert!(!rel_dir.join("_release.json").exists(), "ods pull must never write _release.json");
    assert!(!rel_dir.join("SHA256SUMS").exists(), "ods pull must never write SHA256SUMS");

    // Check workspace _releases.json was updated with fetched index
    let ws_index_file = workspace.join(ods::index::RELEASES_JSON_FILENAME);
    assert!(ws_index_file.exists());
    let ws_index = ods::index::OdsReleaseIndex::load_from_workspace(&workspace)?.unwrap();
    assert_eq!(ws_index.releases.len(), 1);
    assert_eq!(ws_index.releases[0].trud_release_date, "2026-07-31");
    assert_eq!(ws_index.releases[0].datasets[0].dataset_version, "1.0.1");

    Ok(())
}

#[test]
fn test_pull_oci_refuses_when_manifest_digest_mismatches() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");

    let tampered_manifest_bytes = b"{\"tampered\":true}".to_vec();

    let remote_index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("1.0.1", "sha256:0f2a000000000000000000000000000000000000000000000000000000000000")],
    )]);

    let mut responses = BTreeMap::new();
    responses.insert("manifests/sha256:0f2a000000000000000000000000000000000000000000000000000000000000".to_string(), tampered_manifest_bytes);

    let fetcher = TestOciFetcher {
        remote_index: Some(remote_index),
        responses,
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
    assert!(err.to_lowercase().contains("manifest digest mismatch") || err.contains("All mirrors failed to pull release"));
}

#[test]
fn test_pull_oci_mirror_fallback_on_first_mirror_failure() -> Result<()> {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");

    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);
    let prov_bytes = serde_json::to_vec_pretty(&prov)?;
    let prov_sha = format!("sha256:{:x}", sha2::Sha256::digest(&prov_bytes));

    let orgs_bytes = b"dummy orgs parquet content".to_vec();
    let orgs_sha = format!("sha256:{:x}", sha2::Sha256::digest(&orgs_bytes));

    let dp = serde_json::json!({
        "name": "ods",
        "version": "1.0.1",
        "resources": []
    });
    let dp_bytes = serde_json::to_vec_pretty(&dp)?;
    let dp_sha = format!("sha256:{:x}", sha2::Sha256::digest(&dp_bytes));

    let fixture_dir = tmp.path().join("fixture_2");
    std::fs::create_dir_all(&fixture_dir)?;
    std::fs::write(fixture_dir.join("orgs.parquet"), &orgs_bytes)?;
    std::fs::write(fixture_dir.join(ods::provenance::PROVENANCE_FILENAME), &prov_bytes)?;
    std::fs::write(fixture_dir.join(ods::datapackage::DATAPACKAGE_FILENAME), &dp_bytes)?;

    let (manifest, manifest_bytes) = ods::commands::make_oci::build_manifest_from_dir(&fixture_dir, &prov, "1.0.1")?;
    let manifest_digest = manifest.digest()?;

    // Mirror 1 fails, Mirror 2 succeeds
    let mut remote_index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("1.0.1", &manifest_digest)],
    )]);
    remote_index.mirrors = vec![
        MirrorEntry {
            url: "https://broken-mirror.example.com/v2/ods-data".to_string(),
        },
        MirrorEntry {
            url: "https://working-mirror.example.com/v2/ods-data".to_string(),
        },
    ];

    let mut responses = BTreeMap::new();
    responses.insert(format!("https://working-mirror.example.com/v2/ods-data/manifests/{}", manifest_digest), manifest_bytes);
    responses.insert(format!("https://working-mirror.example.com/v2/ods-data/blobs/{}", prov_sha), prov_bytes);
    responses.insert(format!("https://working-mirror.example.com/v2/ods-data/blobs/{}", orgs_sha), orgs_bytes);
    responses.insert(format!("https://working-mirror.example.com/v2/ods-data/blobs/{}", dp_sha), dp_bytes);

    let fetcher = TestOciFetcher {
        remote_index: Some(remote_index),
        responses,
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

    Ok(())
}

#[test]
fn test_pull_oci_self_healing_on_corrupted_local_file() -> Result<()> {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");

    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);
    let prov_bytes = serde_json::to_vec_pretty(&prov)?;
    let prov_sha = format!("sha256:{:x}", sha2::Sha256::digest(&prov_bytes));

    let orgs_bytes = b"legitimate orgs parquet content".to_vec();
    let orgs_sha = format!("sha256:{:x}", sha2::Sha256::digest(&orgs_bytes));

    let dp = serde_json::json!({
        "name": "ods",
        "version": "1.0.1",
        "resources": []
    });
    let dp_bytes = serde_json::to_vec_pretty(&dp)?;
    let dp_sha = format!("sha256:{:x}", sha2::Sha256::digest(&dp_bytes));

    let fixture_dir = tmp.path().join("fixture_heal");
    std::fs::create_dir_all(&fixture_dir)?;
    std::fs::write(fixture_dir.join("orgs.parquet"), &orgs_bytes)?;
    std::fs::write(fixture_dir.join(ods::provenance::PROVENANCE_FILENAME), &prov_bytes)?;
    std::fs::write(fixture_dir.join(ods::datapackage::DATAPACKAGE_FILENAME), &dp_bytes)?;

    let (manifest, manifest_bytes) = ods::commands::make_oci::build_manifest_from_dir(&fixture_dir, &prov, "1.0.1")?;
    let manifest_digest = manifest.digest()?;

    let remote_index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("1.0.1", &manifest_digest)],
    )]);

    let mut responses = BTreeMap::new();
    responses.insert(format!("manifests/{}", manifest_digest), manifest_bytes);
    responses.insert(format!("blobs/{}", prov_sha), prov_bytes);
    responses.insert(format!("blobs/{}", orgs_sha), orgs_bytes.clone());
    responses.insert(format!("blobs/{}", dp_sha), dp_bytes);

    let fetcher = TestOciFetcher {
        remote_index: Some(remote_index),
        responses,
    };

    // Pre-create corrupted orgs.parquet in the target release directory
    let rel_dir = workspace.join("releases").join("2026-07-31");
    std::fs::create_dir_all(&rel_dir)?;
    std::fs::write(rel_dir.join("orgs.parquet"), b"corrupted bytes on disk")?;

    run_with_fetcher(
        Args {
            release_date: Some("2026-07-31".to_string()),
            ..Default::default()
        },
        &workspace,
        &fetcher,
    )?;

    // Assert that corrupted file was replaced with valid file
    let final_bytes = std::fs::read(rel_dir.join("orgs.parquet"))?;
    assert_eq!(final_bytes, orgs_bytes);

    Ok(())
}

#[test]
fn test_pull_oci_security_contradiction_aborts_immediately() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");

    // Baked index knows 2026-07-31 with a legitimate digest
    let baked_index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("1.0.1", "sha256:1111111111111111111111111111111111111111111111111111111111111111")],
    )]);

    // Attack / contradiction: remote index serves a different manifest_digest for baked release 2026-07-31
    let mut tampered_index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("1.0.1", "sha256:0000000000000000000000000000000000000000000000000000000000000000")],
    )]);
    tampered_index.mirrors = vec![MirrorEntry {
        url: "https://evil-mirror.example.com/v2/ods-data".to_string(),
    }];

    let fetcher = TestOciFetcher {
        remote_index: Some(tampered_index),
        responses: BTreeMap::new(),
    };

    let res = ods::commands::pull::run_with_fetcher_and_baked(
        Args {
            release_date: Some("2026-07-31".to_string()),
            ..Default::default()
        },
        &workspace,
        &fetcher,
        Some(baked_index),
    );

    assert!(res.is_err(), "Security contradiction must abort immediately");
    let err = res.unwrap_err().to_string();
    assert!(err.contains("Security error"), "Error must be Security error, got: {}", err);
}

#[test]
fn test_failed_pull_removes_scratch_staging_directory() -> Result<()> {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");

    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);
    let prov_bytes = serde_json::to_vec_pretty(&prov)?;
    let prov_sha = format!("sha256:{:x}", sha2::Sha256::digest(&prov_bytes));

    let orgs_bytes = b"dummy orgs parquet content".to_vec();
    let orgs_sha = format!("sha256:{:x}", sha2::Sha256::digest(&orgs_bytes));

    let fixture_dir = tmp.path().join("fixture_fail");
    std::fs::create_dir_all(&fixture_dir)?;
    std::fs::write(fixture_dir.join("orgs.parquet"), &orgs_bytes)?;
    std::fs::write(fixture_dir.join(ods::provenance::PROVENANCE_FILENAME), &prov_bytes)?;

    let (manifest, manifest_bytes) = ods::commands::make_oci::build_manifest_from_dir(&fixture_dir, &prov, "1.0.1")?;
    let manifest_digest = manifest.digest()?;

    let remote_index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("1.0.1", &manifest_digest)],
    )]);

    let mut responses = BTreeMap::new();
    responses.insert(format!("manifests/{}", manifest_digest), manifest_bytes);
    // First layer succeeds
    responses.insert(format!("blobs/{}", prov_sha), prov_bytes);
    // Second layer fails checksum (deliberately corrupt bytes)
    responses.insert(format!("blobs/{}", orgs_sha), b"corrupted bytes".to_vec());

    let fetcher = TestOciFetcher {
        remote_index: Some(remote_index),
        responses,
    };

    let res = run_with_fetcher(
        Args {
            release_date: Some("2026-07-31".to_string()),
            ..Default::default()
        },
        &workspace,
        &fetcher,
    );

    assert!(res.is_err(), "pull must fail on checksum mismatch");
    let err = res.unwrap_err().to_string();
    assert!(err.contains("checksum mismatch") || err.contains("All mirrors failed"));

    let scratch_dir = workspace.join("scratch");
    if scratch_dir.exists() {
        let pull_entries: Vec<_> = std::fs::read_dir(&scratch_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("pull_"))
            .collect();
        assert!(
            pull_entries.is_empty(),
            "scratch directory must hold no pull_* entry on failure, found: {:?}",
            pull_entries
        );
    }

    Ok(())
}

#[test]
fn test_successful_pull_leaves_no_scratch_staging_directory() -> Result<()> {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");

    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);
    let prov_bytes = serde_json::to_vec_pretty(&prov)?;
    let prov_sha = format!("sha256:{:x}", sha2::Sha256::digest(&prov_bytes));

    let orgs_bytes = b"dummy orgs parquet content".to_vec();
    let orgs_sha = format!("sha256:{:x}", sha2::Sha256::digest(&orgs_bytes));

    let dp = serde_json::json!({
        "name": "ods",
        "version": "1.0.1",
        "resources": []
    });
    let dp_bytes = serde_json::to_vec_pretty(&dp)?;
    let dp_sha = format!("sha256:{:x}", sha2::Sha256::digest(&dp_bytes));

    let fixture_dir = tmp.path().join("fixture_success");
    std::fs::create_dir_all(&fixture_dir)?;
    std::fs::write(fixture_dir.join("orgs.parquet"), &orgs_bytes)?;
    std::fs::write(fixture_dir.join(ods::provenance::PROVENANCE_FILENAME), &prov_bytes)?;
    std::fs::write(fixture_dir.join(ods::datapackage::DATAPACKAGE_FILENAME), &dp_bytes)?;

    let (manifest, manifest_bytes) = ods::commands::make_oci::build_manifest_from_dir(&fixture_dir, &prov, "1.0.1")?;
    let manifest_digest = manifest.digest()?;

    let remote_index = make_v1_index(&[(
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

    let fetcher = TestOciFetcher {
        remote_index: Some(remote_index),
        responses,
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
    assert!(rel_dir.exists(), "release directory must exist");

    let scratch_dir = workspace.join("scratch");
    if scratch_dir.exists() {
        let pull_entries: Vec<_> = std::fs::read_dir(&scratch_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("pull_"))
            .collect();
        assert!(
            pull_entries.is_empty(),
            "scratch directory must hold no pull_* entry on success, found: {:?}",
            pull_entries
        );
    }

    Ok(())
}

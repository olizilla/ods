use anyhow::Result;
use ods::commands::pull::{run_with_fetcher, Args, OciBlobFetcher};
use ods::index::{MirrorEntry, OdsReleaseIndex, ReleaseIndexEntry};
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
    prov.dataset_version = Some("1.0.1".to_string());
    prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);
    let prov_bytes = serde_json::to_vec_pretty(&prov)?;
    let prov_sha = format!("sha256:{:x}", sha2::Sha256::digest(&prov_bytes));

    let orgs_bytes = b"dummy orgs parquet content".to_vec();
    let orgs_sha = format!("sha256:{:x}", sha2::Sha256::digest(&orgs_bytes));

    let fixture_dir = tmp.path().join("fixture_1");
    std::fs::create_dir_all(&fixture_dir)?;
    std::fs::write(fixture_dir.join("orgs.parquet"), &orgs_bytes)?;
    std::fs::write(fixture_dir.join(ods::provenance::PROVENANCE_FILENAME), &prov_bytes)?;

    let (manifest, manifest_bytes) = ods::commands::make_oci::build_manifest_from_dir(&fixture_dir, &prov, "1.0.1")?;
    let manifest_digest = manifest.digest()?;

    let remote_index = OdsReleaseIndex {
        type_tag: "ods_release_index".to_string(),
        index_version: 2,
        concept_doi: None,
        mirrors: vec![MirrorEntry {
            url: "https://ods.fyi/v2/ods-data".to_string(),
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

    let mut responses = BTreeMap::new();
    responses.insert(format!("manifests/{}", manifest_digest), manifest_bytes);
    responses.insert(format!("blobs/{}", prov_sha), prov_bytes);
    responses.insert(format!("blobs/{}", orgs_sha), orgs_bytes);

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

    // Assert that current symlink is pinned to the newly pulled release and README exists
    assert!(workspace.join("current").exists(), "current symlink must exist");
    let (active_date, active_dir) = ods::workspace::Workspace::open(Some(&workspace))?.active_release()?;
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
            url: "https://ods.fyi/v2/ods-data".to_string(),
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
    assert!(err.contains("All mirrors failed to pull release") || err.contains("manifest digest mismatch"));
}

#[test]
fn test_pull_oci_mirror_fallback_on_first_mirror_failure() -> Result<()> {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");

    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.dataset_version = Some("1.0.1".to_string());
    prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);
    let prov_bytes = serde_json::to_vec_pretty(&prov)?;
    let prov_sha = format!("sha256:{:x}", sha2::Sha256::digest(&prov_bytes));

    let orgs_bytes = b"sample orgs parquet bytes".to_vec();
    let orgs_sha = format!("sha256:{:x}", sha2::Sha256::digest(&orgs_bytes));

    let fixture_dir = tmp.path().join("fixture_2");
    std::fs::create_dir_all(&fixture_dir)?;
    std::fs::write(fixture_dir.join("orgs.parquet"), &orgs_bytes)?;
    std::fs::write(fixture_dir.join(ods::provenance::PROVENANCE_FILENAME), &prov_bytes)?;

    let (manifest, manifest_bytes) = ods::commands::make_oci::build_manifest_from_dir(&fixture_dir, &prov, "1.0.1")?;
    let manifest_digest = manifest.digest()?;

    // Mirror 1 fails, Mirror 2 succeeds
    let remote_index = OdsReleaseIndex {
        type_tag: "ods_release_index".to_string(),
        index_version: 2,
        concept_doi: None,
        mirrors: vec![
            MirrorEntry {
                url: "https://broken-mirror.example.com/v2/ods-data".to_string(),
            },
            MirrorEntry {
                url: "https://working-mirror.example.com/v2/ods-data".to_string(),
            },
        ],
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

    let mut responses = BTreeMap::new();
    responses.insert(format!("https://working-mirror.example.com/v2/ods-data/manifests/{}", manifest_digest), manifest_bytes);
    responses.insert(format!("https://working-mirror.example.com/v2/ods-data/blobs/{}", prov_sha), prov_bytes);
    responses.insert(format!("https://working-mirror.example.com/v2/ods-data/blobs/{}", orgs_sha), orgs_bytes);

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

    Ok(())
}

#[test]
fn test_pull_oci_self_healing_on_corrupted_local_file() -> Result<()> {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");

    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.dataset_version = Some("1.0.1".to_string());
    prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);
    let prov_bytes = serde_json::to_vec_pretty(&prov)?;
    let prov_sha = format!("sha256:{:x}", sha2::Sha256::digest(&prov_bytes));

    let orgs_bytes = b"valid orgs parquet bytes".to_vec();
    let orgs_sha = format!("sha256:{:x}", sha2::Sha256::digest(&orgs_bytes));

    let fixture_dir = tmp.path().join("fixture_heal");
    std::fs::create_dir_all(&fixture_dir)?;
    std::fs::write(fixture_dir.join("orgs.parquet"), &orgs_bytes)?;
    std::fs::write(fixture_dir.join(ods::provenance::PROVENANCE_FILENAME), &prov_bytes)?;

    let (manifest, manifest_bytes) = ods::commands::make_oci::build_manifest_from_dir(&fixture_dir, &prov, "1.0.1")?;
    let manifest_digest = manifest.digest()?;

    let remote_index = OdsReleaseIndex {
        type_tag: "ods_release_index".to_string(),
        index_version: 2,
        concept_doi: None,
        mirrors: vec![MirrorEntry {
            url: "https://ods.fyi/v2/ods-data".to_string(),
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

    let mut responses = BTreeMap::new();
    responses.insert(format!("manifests/{}", manifest_digest), manifest_bytes);
    responses.insert(format!("blobs/{}", prov_sha), prov_bytes);
    responses.insert(format!("blobs/{}", orgs_sha), orgs_bytes.clone());

    let fetcher = TestOciFetcher {
        remote_index: Some(remote_index),
        responses,
    };

    // Pre-create corrupted local release directory
    let rel_dir = workspace.join("releases").join("2026-07-31");
    std::fs::create_dir_all(&rel_dir)?;
    std::fs::write(rel_dir.join("orgs.parquet"), b"corrupted bytes")?;
    std::fs::write(rel_dir.join(ods::provenance::PROVENANCE_FILENAME), serde_json::to_vec(&prov)?)?;

    // Pull should detect corruption and self-heal by re-downloading
    run_with_fetcher(
        Args {
            release_date: Some("2026-07-31".to_string()),
            ..Default::default()
        },
        &workspace,
        &fetcher,
    )?;

    let repaired_bytes = std::fs::read(rel_dir.join("orgs.parquet"))?;
    assert_eq!(repaired_bytes, orgs_bytes);

    Ok(())
}

#[test]
fn test_pull_oci_frontier_tag_disagreement_refuses() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");

    let manifest1_bytes = b"{\"schemaVersion\":2,\"mediaType\":\"application/vnd.oci.image.manifest.v1+json\",\"artifactType\":\"application/vnd.fyi.ods.dataset.v1\",\"config\":{\"mediaType\":\"application/vnd.fyi.ods.provenance.v1+json\",\"digest\":\"sha256:1111\",\"size\":10},\"layers\":[],\"annotations\":{}}".to_vec();
    let manifest1_digest = format!("sha256:{:x}", sha2::Sha256::digest(&manifest1_bytes));

    let manifest2_bytes = b"{\"schemaVersion\":2,\"mediaType\":\"application/vnd.oci.image.manifest.v1+json\",\"artifactType\":\"application/vnd.fyi.ods.dataset.v1\",\"config\":{\"mediaType\":\"application/vnd.fyi.ods.provenance.v1+json\",\"digest\":\"sha256:2222\",\"size\":10},\"layers\":[],\"annotations\":{}}".to_vec();

    // Release 2099-01-01 is a frontier release (not in baked index)
    let remote_index = OdsReleaseIndex {
        type_tag: "ods_release_index".to_string(),
        index_version: 2,
        concept_doi: None,
        mirrors: vec![
            MirrorEntry {
                url: "https://ods.fyi/v2/ods-data".to_string(),
            },
            MirrorEntry {
                url: "https://ghcr.io/v2/olizilla/ods-data".to_string(),
            },
        ],
        releases: vec![ReleaseIndexEntry {
            trud_release_date: "2099-01-01".to_string(),
            dataset_version: "1.0.0".to_string(),
            tag: "2099-01-01_1.0.0".to_string(),
            manifest_digest: manifest1_digest.clone(),
            trud_release_sha256: "FFFF".to_string(),
            tool_version: "0.4.3".to_string(),
            dataset_doi: None,
            withdrawn: None,
        }],
    };

    let mut responses = BTreeMap::new();
    responses.insert(format!("https://ods.fyi/v2/ods-data/manifests/{}", manifest1_digest), manifest1_bytes);
    responses.insert("https://ghcr.io/v2/olizilla/ods-data/manifests/2099-01-01_1.0.0".to_string(), manifest2_bytes);

    let fetcher = TestOciFetcher {
        remote_index: Some(remote_index),
        responses,
    };

    let res = run_with_fetcher(
        Args {
            release_date: Some("2099-01-01".to_string()),
            ..Default::default()
        },
        &workspace,
        &fetcher,
    );

    assert!(res.is_err(), "Must refuse when mirrors disagree on frontier tag");
}

#[test]
fn test_pull_oci_security_contradiction_aborts_immediately() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");

    // Baked index knows 2026-07-31 with a legitimate digest
    let baked_index = OdsReleaseIndex {
        type_tag: "ods_release_index".to_string(),
        index_version: 2,
        concept_doi: None,
        mirrors: vec![],
        releases: vec![ReleaseIndexEntry {
            trud_release_date: "2026-07-31".to_string(),
            dataset_version: "1.0.1".to_string(),
            tag: "2026-07-31_1.0.1".to_string(),
            manifest_digest: "sha256:1111111111111111111111111111111111111111111111111111111111111111".to_string(),
            trud_release_sha256: "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string(),
            tool_version: "0.4.3".to_string(),
            dataset_doi: None,
            withdrawn: None,
        }],
    };

    // Attack / contradiction: remote index serves a different manifest_digest for baked release 2026-07-31
    let tampered_index = OdsReleaseIndex {
        type_tag: "ods_release_index".to_string(),
        index_version: 2,
        concept_doi: None,
        mirrors: vec![MirrorEntry {
            url: "https://evil-mirror.example.com/v2/ods-data".to_string(),
        }],
        releases: vec![ReleaseIndexEntry {
            trud_release_date: "2026-07-31".to_string(),
            dataset_version: "1.0.1".to_string(),
            tag: "2026-07-31_1.0.1".to_string(),
            manifest_digest: "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_string(),
            trud_release_sha256: "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string(),
            tool_version: "0.4.3".to_string(),
            dataset_doi: None,
            withdrawn: None,
        }],
    };

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
fn test_pull_oci_frontier_corroborated_two_mirrors_success() -> Result<()> {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");

    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_date = Some("2099-01-01".to_string());
    prov.dataset_version = Some("1.0.0".to_string());
    prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);
    let prov_bytes = serde_json::to_vec_pretty(&prov)?;
    let prov_sha = format!("sha256:{:x}", sha2::Sha256::digest(&prov_bytes));

    let orgs_bytes = b"frontier orgs parquet content".to_vec();
    let orgs_sha = format!("sha256:{:x}", sha2::Sha256::digest(&orgs_bytes));

    let fixture_dir = tmp.path().join("fixture_frontier");
    std::fs::create_dir_all(&fixture_dir)?;
    std::fs::write(fixture_dir.join("orgs.parquet"), &orgs_bytes)?;
    std::fs::write(fixture_dir.join(ods::provenance::PROVENANCE_FILENAME), &prov_bytes)?;

    let (manifest, manifest_bytes) = ods::commands::make_oci::build_manifest_from_dir(&fixture_dir, &prov, "1.0.0")?;
    let manifest_digest = manifest.digest()?;

    let frontier_index = OdsReleaseIndex {
        type_tag: "ods_release_index".to_string(),
        index_version: 2,
        concept_doi: None,
        mirrors: vec![
            MirrorEntry {
                url: "https://ods.fyi/v2/ods-data".to_string(),
            },
            MirrorEntry {
                url: "https://ghcr.io/v2/olizilla/ods-data".to_string(),
            },
        ],
        releases: vec![ReleaseIndexEntry {
            trud_release_date: "2099-01-01".to_string(),
            dataset_version: "1.0.0".to_string(),
            tag: "2099-01-01_1.0.0".to_string(),
            manifest_digest: manifest_digest.clone(),
            trud_release_sha256: "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string(),
            tool_version: "0.4.3".to_string(),
            dataset_doi: None,
            withdrawn: None,
        }],
    };

    let mut responses = BTreeMap::new();
    // Mirror 1 responses
    responses.insert(format!("https://ods.fyi/v2/ods-data/manifests/{}", manifest_digest), manifest_bytes.clone());
    responses.insert(format!("https://ods.fyi/v2/ods-data/blobs/{}", prov_sha), prov_bytes);
    responses.insert(format!("https://ods.fyi/v2/ods-data/blobs/{}", orgs_sha), orgs_bytes);

    // Mirror 2 tag response (same manifest bytes -> agreement)
    responses.insert("https://ghcr.io/v2/olizilla/ods-data/manifests/2099-01-01_1.0.0".to_string(), manifest_bytes);

    let fetcher = TestOciFetcher {
        remote_index: Some(frontier_index),
        responses,
    };

    run_with_fetcher(
        Args {
            release_date: Some("2099-01-01".to_string()),
            ..Default::default()
        },
        &workspace,
        &fetcher,
    )?;

    let rel_dir = workspace.join("releases").join("2099-01-01");
    assert!(rel_dir.exists());
    assert!(rel_dir.join("orgs.parquet").exists());
    assert_eq!(ods::workspace::Workspace::open(Some(&workspace))?.active_release()?.0, "2099-01-01");

    Ok(())
}

#[test]
fn test_pull_oci_frontier_second_mirror_unreachable_uncorroborated() -> Result<()> {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");

    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_date = Some("2099-01-01".to_string());
    prov.dataset_version = Some("1.0.0".to_string());
    prov.trud_release_sha256 = Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string());
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);
    let prov_bytes = serde_json::to_vec_pretty(&prov)?;
    let prov_sha = format!("sha256:{:x}", sha2::Sha256::digest(&prov_bytes));

    let orgs_bytes = b"frontier orgs parquet content".to_vec();
    let orgs_sha = format!("sha256:{:x}", sha2::Sha256::digest(&orgs_bytes));

    let fixture_dir = tmp.path().join("fixture_frontier_single");
    std::fs::create_dir_all(&fixture_dir)?;
    std::fs::write(fixture_dir.join("orgs.parquet"), &orgs_bytes)?;
    std::fs::write(fixture_dir.join(ods::provenance::PROVENANCE_FILENAME), &prov_bytes)?;

    let (manifest, manifest_bytes) = ods::commands::make_oci::build_manifest_from_dir(&fixture_dir, &prov, "1.0.0")?;
    let manifest_digest = manifest.digest()?;

    let frontier_index = OdsReleaseIndex {
        type_tag: "ods_release_index".to_string(),
        index_version: 2,
        concept_doi: None,
        mirrors: vec![
            MirrorEntry {
                url: "https://ods.fyi/v2/ods-data".to_string(),
            },
            MirrorEntry {
                url: "https://ghcr.io/v2/olizilla/ods-data".to_string(),
            },
        ],
        releases: vec![ReleaseIndexEntry {
            trud_release_date: "2099-01-01".to_string(),
            dataset_version: "1.0.0".to_string(),
            tag: "2099-01-01_1.0.0".to_string(),
            manifest_digest: manifest_digest.clone(),
            trud_release_sha256: "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string(),
            tool_version: "0.4.3".to_string(),
            dataset_doi: None,
            withdrawn: None,
        }],
    };

    let mut responses = BTreeMap::new();
    // Mirror 1 responses only (Mirror 2 is unreachable / missing from map)
    responses.insert(format!("https://ods.fyi/v2/ods-data/manifests/{}", manifest_digest), manifest_bytes);
    responses.insert(format!("https://ods.fyi/v2/ods-data/blobs/{}", prov_sha), prov_bytes);
    responses.insert(format!("https://ods.fyi/v2/ods-data/blobs/{}", orgs_sha), orgs_bytes);

    let fetcher = TestOciFetcher {
        remote_index: Some(frontier_index),
        responses,
    };

    run_with_fetcher(
        Args {
            release_date: Some("2099-01-01".to_string()),
            ..Default::default()
        },
        &workspace,
        &fetcher,
    )?;

    let rel_dir = workspace.join("releases").join("2099-01-01");
    assert!(rel_dir.exists());
    assert!(rel_dir.join("orgs.parquet").exists());
    assert_eq!(ods::workspace::Workspace::open(Some(&workspace))?.active_release()?.0, "2099-01-01");

    Ok(())
}

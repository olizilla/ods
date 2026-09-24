mod common;

use anyhow::Result;
use common::make_v1_index;
use ods::commands::pull::{
    run_with_fetcher, run_with_fetcher_and_writer, AlreadyReported, Args, OciBlobFetcher,
};
use ods::index::{MirrorEntry, OdsReleaseIndex};
use ods::workspace::Workspace;
use sha2::Digest;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

#[derive(Default)]
struct TestOciFetcher {
    pub remote_index: Option<OdsReleaseIndex>,
    pub responses: BTreeMap<String, Vec<u8>>,
    pub errors: BTreeMap<String, String>,
}

impl TestOciFetcher {
    #[allow(dead_code)]
    pub fn new(remote_index: Option<OdsReleaseIndex>, responses: BTreeMap<String, Vec<u8>>) -> Self {
        Self {
            remote_index,
            responses,
            errors: BTreeMap::new(),
        }
    }
}

impl OciBlobFetcher for TestOciFetcher {
    fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>> {
        if let Some(err) = self.errors.get(url) {
            anyhow::bail!("{}", err);
        }
        for (key, err) in &self.errors {
            if url.ends_with(key) {
                anyhow::bail!("{}", err);
            }
        }
        if let Some(val) = self.responses.get(url) {
            return Ok(val.clone());
        }
        for (key, val) in &self.responses {
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
        ..Default::default()
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
        ..Default::default()
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
        ..Default::default()
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
        ..Default::default()
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
fn test_pull_oci_remote_index_selected_without_merge_contradiction_error() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");
    fs::create_dir_all(&workspace).unwrap();

    // Contradiction: remote index serves a different manifest_digest for baked release 2026-07-31
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
        ..Default::default()
    };

    let res = ods::commands::pull::run_with_fetcher(
        Args {
            release_date: Some("2026-07-31".to_string()),
            ..Default::default()
        },
        &workspace,
        &fetcher,
    );

    // Selected remote index is used directly (no SecurityError merge refusal);
    // pull fails at mirror fetch time because evil-mirror has no asset.
    assert!(res.is_err());
    let err = res.unwrap_err().to_string();
    assert!(!err.contains("Security error"), "Select, don't merge: must not raise SecurityError on read");
    assert!(err.contains("evil-mirror.example.com"));
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
        ..Default::default()
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
    let err = res.unwrap_err();
    assert!(
        err.chain().any(|c| c.downcast_ref::<ods::commands::pull::AlreadyReported>().is_some()),
        "must exit via AlreadyReported"
    );

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
        ..Default::default()
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

#[test]
fn test_pull_named_withdrawn_release_delivers_and_exits_1() -> Result<()> {
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
        "version": "1.0.0",
        "resources": []
    });
    let dp_bytes = serde_json::to_vec_pretty(&dp)?;
    let dp_sha = format!("sha256:{:x}", sha2::Sha256::digest(&dp_bytes));

    let fixture_dir = tmp.path().join("fixture_withdrawn");
    std::fs::create_dir_all(&fixture_dir)?;
    std::fs::write(fixture_dir.join("orgs.parquet"), &orgs_bytes)?;
    std::fs::write(fixture_dir.join(ods::provenance::PROVENANCE_FILENAME), &prov_bytes)?;
    std::fs::write(fixture_dir.join(ods::datapackage::DATAPACKAGE_FILENAME), &dp_bytes)?;

    let (manifest, manifest_bytes) = ods::commands::make_oci::build_manifest_from_dir(&fixture_dir, &prov, "1.0.0")?;
    let manifest_digest = manifest.digest()?;

    let mut remote_index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("1.0.0", &manifest_digest)],
    )]);
    remote_index.releases[0].datasets[0].withdrawn =
        Some("roles table truncated at 65535 rows by a bad build".to_string());

    let mut responses = BTreeMap::new();
    responses.insert(format!("manifests/{}", manifest_digest), manifest_bytes);
    responses.insert(format!("blobs/{}", prov_sha), prov_bytes);
    responses.insert(format!("blobs/{}", orgs_sha), orgs_bytes);
    responses.insert(format!("blobs/{}", dp_sha), dp_bytes);

    let fetcher = TestOciFetcher {
        remote_index: Some(remote_index),
        responses,
        ..Default::default()
    };

    let res = run_with_fetcher(
        Args {
            release_date: Some("2026-07-31".to_string()),
            ..Default::default()
        },
        &workspace,
        &fetcher,
    );

    assert!(res.is_err(), "must exit with error (exit 1) for withdrawn release");
    let err = res.unwrap_err();
    assert!(
        err.chain().any(|c| c.downcast_ref::<ods::commands::pull::AlreadyReported>().is_some()),
        "must exit via AlreadyReported"
    );

    // Assert release was delivered
    let rel_dir = workspace.join("releases").join("2026-07-31");
    assert!(rel_dir.exists(), "release directory must be delivered");
    assert!(rel_dir.join("orgs.parquet").exists());

    // Assert current symlink was pinned
    let current = workspace.join("current");
    assert!(current.exists(), "current symlink must be set");
    let (active_date, _) = ods::workspace::Workspace::open(Some(&workspace))?.active_release()?;
    assert_eq!(active_date, "2026-07-31");

    Ok(())
}

#[test]
fn test_pull_cli_named_withdrawn_and_bare_pull_acceptance() -> Result<()> {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");
    std::fs::create_dir_all(workspace.join("releases"))?;

    // Create 2026-08-28 release
    let rel_28 = workspace.join("releases").join("2026-08-28");
    std::fs::create_dir_all(&rel_28)?;
    std::fs::write(rel_28.join("orgs.parquet"), b"dummy 28")?;
    let prov_28 = ods::provenance::OdsProvenance {
        trud_release_date: Some("2026-08-28".to_string()),
        trud_release_sha256: Some("ABDD194B1569D5FF3CDD81D618847F05642BD43C5B15D6CD43D8289B7466D801".to_string()),
        trud_release_sha256_verified: Some(ods::provenance::TrudVerificationSource::TrudApi),
        ..Default::default()
    };
    std::fs::write(rel_28.join(ods::provenance::PROVENANCE_FILENAME), serde_json::to_vec_pretty(&prov_28)?)?;
    let dp_28 = serde_json::json!({ "name": "ods", "version": "1.0.0", "resources": [] });
    std::fs::write(rel_28.join(ods::datapackage::DATAPACKAGE_FILENAME), serde_json::to_vec_pretty(&dp_28)?)?;
    let (m_28, _) = ods::commands::make_oci::build_manifest_from_dir(&rel_28, &prov_28, "1.0.0")?;
    let digest_28 = m_28.digest()?;

    // Create 2026-07-31 release
    let rel_31 = workspace.join("releases").join("2026-07-31");
    std::fs::create_dir_all(&rel_31)?;
    std::fs::write(rel_31.join("orgs.parquet"), b"dummy 31")?;
    let prov_31 = ods::provenance::OdsProvenance {
        trud_release_date: Some("2026-07-31".to_string()),
        trud_release_sha256: Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string()),
        trud_release_sha256_verified: Some(ods::provenance::TrudVerificationSource::TrudApi),
        ..Default::default()
    };
    std::fs::write(rel_31.join(ods::provenance::PROVENANCE_FILENAME), serde_json::to_vec_pretty(&prov_31)?)?;
    let dp_31 = serde_json::json!({ "name": "ods", "version": "1.0.0", "resources": [] });
    std::fs::write(rel_31.join(ods::datapackage::DATAPACKAGE_FILENAME), serde_json::to_vec_pretty(&dp_31)?)?;
    let (m_31, _) = ods::commands::make_oci::build_manifest_from_dir(&rel_31, &prov_31, "1.0.0")?;
    let digest_31 = m_31.digest()?;

    // Build supplied index with 2026-08-28 withdrawn
    let mut index = make_v1_index(&[
        (
            "2026-08-28",
            "ABDD194B1569D5FF3CDD81D618847F05642BD43C5B15D6CD43D8289B7466D801",
            38064419,
            &[("1.0.0", &digest_28)],
        ),
        (
            "2026-07-31",
            "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
            37983173,
            &[("1.0.0", &digest_31)],
        ),
    ]);
    index.releases[0].datasets[0].withdrawn =
        Some("roles table truncated at 65535 rows by a bad build".to_string());

    let index_file = tmp.path().join("index.json");
    std::fs::write(&index_file, serde_json::to_vec_pretty(&index)?)?;

    // 1. ods pull 2026-08-28 --index ...
    let out_named = common::ods_cmd()
        .current_dir(tmp.path())
        .args(["pull", "2026-08-28", "--index", index_file.to_str().unwrap()])
        .output()?;

    assert_eq!(out_named.status.code(), Some(1));
    let stderr_named = String::from_utf8_lossy(&out_named.stderr);
    assert!(stderr_named.contains("2026-08-28") && stderr_named.contains("cached"));
    assert!(stderr_named.contains("current → releases/2026-08-28"));
    assert!(stderr_named.contains("✖ 2026-08-28 (1.0.0) was withdrawn: roles table truncated at 65535 rows by a bad build"));
    assert!(stderr_named.contains("Pull a valid release: ods pull"));

    // 2. bare ods pull --index ...
    let out_bare = common::ods_cmd()
        .current_dir(tmp.path())
        .args(["pull", "--index", index_file.to_str().unwrap()])
        .output()?;

    assert_eq!(out_bare.status.code(), Some(0));
    let stderr_bare = String::from_utf8_lossy(&out_bare.stderr);
    assert!(stderr_bare.contains("! 2026-08-28 (1.0.0) was withdrawn: roles table truncated at 65535 rows by a bad build"));
    assert!(stderr_bare.contains("Pulling 2026-07-31 instead"));
    assert!(stderr_bare.contains("2026-07-31") && stderr_bare.contains("cached"));
    assert!(stderr_bare.contains("current → releases/2026-07-31"));

    Ok(())
}

struct StandardFixture {
    pub manifest_digest: String,
    pub manifest_bytes: Vec<u8>,
    pub files: Vec<(String, String, Vec<u8>)>, // (filename, digest, bytes)
}

fn make_standard_6_file_fixture(tmp: &Path, date: &str, version: &str) -> Result<StandardFixture> {
    let fix_dir = tmp.join(format!("fix_{}_{}", date, version));
    fs::create_dir_all(&fix_dir)?;

    let prov = ods::provenance::OdsProvenance {
        trud_release_date: Some(date.to_string()),
        trud_release_sha256: Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string()),
        trud_release_sha256_verified: Some(ods::provenance::TrudVerificationSource::TrudApi),
        ..Default::default()
    };
    let prov_bytes = serde_json::to_vec_pretty(&prov)?;

    let dp = serde_json::json!({
        "name": "ods",
        "version": version,
        "resources": []
    });
    let dp_bytes = serde_json::to_vec_pretty(&dp)?;

    let orgs_bytes = b"sample parquet orgs 123".to_vec();
    let rel_bytes = b"sample parquet rel 123".to_vec();
    let roles_bytes = b"sample parquet roles 123".to_vec();
    let succ_bytes = b"sample parquet succ 123".to_vec();

    fs::write(fix_dir.join(ods::provenance::PROVENANCE_FILENAME), &prov_bytes)?;
    fs::write(fix_dir.join(ods::datapackage::DATAPACKAGE_FILENAME), &dp_bytes)?;
    fs::write(fix_dir.join("orgs.parquet"), &orgs_bytes)?;
    fs::write(fix_dir.join("relationships.parquet"), &rel_bytes)?;
    fs::write(fix_dir.join("roles.parquet"), &roles_bytes)?;
    fs::write(fix_dir.join("successions.parquet"), &succ_bytes)?;

    let (manifest, manifest_bytes) = ods::commands::make_oci::build_manifest_from_dir(&fix_dir, &prov, version)?;
    let manifest_digest = manifest.digest()?;

    let mut files = Vec::new();
    for layer in &manifest.layers {
        let title = layer
            .annotations
            .as_ref()
            .and_then(|a| a.get("org.opencontainers.image.title"))
            .cloned()
            .unwrap();
        let bytes = match title.as_str() {
            ods::provenance::PROVENANCE_FILENAME => prov_bytes.clone(),
            ods::datapackage::DATAPACKAGE_FILENAME => dp_bytes.clone(),
            "orgs.parquet" => orgs_bytes.clone(),
            "relationships.parquet" => rel_bytes.clone(),
            "roles.parquet" => roles_bytes.clone(),
            "successions.parquet" => succ_bytes.clone(),
            other => panic!("Unexpected layer: {}", other),
        };
        files.push((title, layer.digest.clone(), bytes));
    }

    Ok(StandardFixture {
        manifest_digest,
        manifest_bytes,
        files,
    })
}

#[test]
fn test_pull_multi_mirror_combines_verified_layers_from_different_mirrors() -> Result<()> {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");

    let fix = make_standard_6_file_fixture(tmp.path(), "2026-07-31", "1.0.1")?;

    let m1_base = "https://mirror1.example.com/v2/ods-data";
    let m2_base = "https://mirror2.example.com/v2/ods-data";

    let mut remote_index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("1.0.1", &fix.manifest_digest)],
    )]);
    remote_index.mirrors = vec![
        MirrorEntry { url: m1_base.to_string() },
        MirrorEntry { url: m2_base.to_string() },
    ];

    let mut responses = BTreeMap::new();
    responses.insert(format!("{}/manifests/{}", m1_base, fix.manifest_digest), fix.manifest_bytes.clone());
    responses.insert(format!("{}/manifests/{}", m2_base, fix.manifest_digest), fix.manifest_bytes.clone());

    for (name, digest, bytes) in &fix.files {
        let m1_url = format!("{}/blobs/{}", m1_base, digest);
        let m2_url = format!("{}/blobs/{}", m2_base, digest);

        if name == "successions.parquet" {
            // Mirror 1 serves bad bytes, Mirror 2 serves good bytes
            responses.insert(m1_url, b"corrupted successions on mirror 1".to_vec());
            responses.insert(m2_url, bytes.clone());
        } else if name == "roles.parquet" {
            // Mirror 1 serves good bytes, Mirror 2 serves bad bytes
            responses.insert(m1_url, bytes.clone());
            responses.insert(m2_url, b"corrupted roles on mirror 2".to_vec());
        } else {
            // Both mirrors serve good bytes
            responses.insert(m1_url, bytes.clone());
            responses.insert(m2_url, bytes.clone());
        }
    }

    let fetcher = TestOciFetcher {
        remote_index: Some(remote_index.clone()),
        responses,
        ..Default::default()
    };

    let mut stderr_buf = Vec::new();
    let res = run_with_fetcher_and_writer(
        Args {
            release_date: Some("2026-07-31".to_string()),
            verbose: true,
            ..Default::default()
        },
        &workspace,
        &fetcher,
        &mut stderr_buf,
    );

    assert!(res.is_ok(), "multi-mirror pull combining good layers must succeed");

    let rel_dir = workspace.join("releases").join("2026-07-31");
    assert!(rel_dir.exists(), "release dir must exist");

    let outcome = ods::workspace::verify_release_dir(&rel_dir, &remote_index);
    assert!(outcome.is_verified(), "installed release must be verified");

    let (active_date, _) = Workspace::open(Some(&workspace))?.active_release()?;
    assert_eq!(active_date, "2026-07-31", "current must be pinned to 2026-07-31");

    let stderr = String::from_utf8(stderr_buf)?;
    assert!(
        stderr.contains("2026-07-31") && stderr.contains("from 2 mirrors"),
        "block must name both mirrors that served layers, got:\n{}",
        stderr
    );
    assert!(stderr.contains("current → releases/2026-07-31"));

    // `--verbose` names which host served each layer once more than one mirror was needed.
    assert!(
        stderr.contains("roles.parquet") && stderr.contains("mirror1.example.com"),
        "verbose output must name the mirror that served roles.parquet, got:\n{}",
        stderr
    );
    assert!(
        stderr.contains("successions.parquet") && stderr.contains("mirror2.example.com"),
        "verbose output must name the mirror that served successions.parquet, got:\n{}",
        stderr
    );
    let roles_idx = stderr.find("roles.parquet").expect("roles.parquet line");
    let roles_host_idx = stderr[roles_idx..].find("mirror1.example.com").expect("roles.parquet's host");
    let succ_idx = stderr.find("successions.parquet").expect("successions.parquet line");
    let succ_host_idx = stderr[succ_idx..].find("mirror2.example.com").expect("successions.parquet's host");
    assert!(
        roles_host_idx < 80 && succ_host_idx < 80,
        "each filename's host must be on the same line, got:\n{}",
        stderr
    );

    Ok(())
}

#[test]
fn test_pull_bad_layer_installs_as_bad_sha_and_leaves_current_untouched() -> Result<()> {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");

    // Establish pre-existing active release 2026-08-28
    let ws = Workspace::open_or_create(Some(&workspace))?;
    let rel_28 = workspace.join("releases").join("2026-08-28");
    fs::create_dir_all(&rel_28)?;
    fs::write(rel_28.join("dummy.txt"), b"active release 28")?;
    ws.set_active("2026-08-28")?;

    let fix = make_standard_6_file_fixture(tmp.path(), "2026-07-31", "0.1.0")?;
    let m1_base = "https://ods.fyi/v2/ods-data";

    let mut remote_index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("0.1.0", &fix.manifest_digest)],
    )]);
    remote_index.mirrors = vec![MirrorEntry { url: m1_base.to_string() }];

    let bad_bytes = b"bad successions bytes from mirror 1".to_vec();
    let mut bad_layer_digest = String::new();

    let mut responses = BTreeMap::new();
    responses.insert(format!("{}/manifests/{}", m1_base, fix.manifest_digest), fix.manifest_bytes.clone());

    for (name, digest, bytes) in &fix.files {
        let url = format!("{}/blobs/{}", m1_base, digest);
        if name == "successions.parquet" {
            bad_layer_digest = digest.clone();
            responses.insert(url, bad_bytes.clone());
        } else {
            responses.insert(url, bytes.clone());
        }
    }

    let fetcher = TestOciFetcher {
        remote_index: Some(remote_index),
        responses,
        ..Default::default()
    };

    let mut stderr_buf = Vec::new();
    let res = run_with_fetcher_and_writer(
        Args {
            release_date: Some("2026-07-31".to_string()),
            ..Default::default()
        },
        &workspace,
        &fetcher,
        &mut stderr_buf,
    );

    assert!(res.is_err());
    let err = res.unwrap_err();
    assert!(
        err.chain().any(|c| c.downcast_ref::<AlreadyReported>().is_some()),
        "must exit via AlreadyReported"
    );

    let rel_dir = workspace.join("releases").join("2026-07-31");
    assert!(rel_dir.exists(), "releases/2026-07-31 must exist");

    // Assert 5 verified files exist under their standard names
    assert!(rel_dir.join(ods::provenance::PROVENANCE_FILENAME).exists());
    assert!(rel_dir.join(ods::datapackage::DATAPACKAGE_FILENAME).exists());
    assert!(rel_dir.join("orgs.parquet").exists());
    assert!(rel_dir.join("relationships.parquet").exists());
    assert!(rel_dir.join("roles.parquet").exists());

    // successions.parquet does NOT exist, but successions.parquet.bad-sha DOES exist
    assert!(!rel_dir.join("successions.parquet").exists());
    let bad_sha_path = rel_dir.join("successions.parquet.bad-sha");
    assert!(bad_sha_path.exists());
    assert_eq!(fs::read(&bad_sha_path)?, bad_bytes);

    // Assert current symlink is untouched
    let (active_date, _) = Workspace::open(Some(&workspace))?.active_release()?;
    assert_eq!(active_date, "2026-08-28", "current must remain 2026-08-28");

    // Assert stderr output
    let stderr = String::from_utf8(stderr_buf)?;
    assert!(stderr.contains("✖ 2026-07-31 (0.1.0) arrived incomplete: 1 of 6 files failed verification on every mirror"));
    assert!(stderr.contains("successions.parquet → releases/2026-07-31/successions.parquet.bad-sha"));
    assert!(stderr.contains(&format!("expected  {}", bad_layer_digest)));
    let computed_bad_sha = format!("sha256:{:x}", sha2::Sha256::digest(&bad_bytes));
    assert!(stderr.contains(&format!("ods.fyi   {}", computed_bad_sha)));
    assert!(stderr.contains("The other 5 files are verified, in releases/2026-07-31/"));
    assert!(stderr.contains("current is still releases/2026-08-28"));
    assert!(stderr.contains("Retry: ods pull 2026-07-31"));
    assert!(stderr.contains("Use it anyway: ods use 2026-07-31"));
    assert!(stderr.contains("Report it: https://github.com/olizilla/ods/issues"));

    Ok(())
}

#[test]
fn test_pull_layer_404_on_all_mirrors_not_downloaded() -> Result<()> {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");

    let fix = make_standard_6_file_fixture(tmp.path(), "2026-07-31", "0.1.0")?;
    let m1_base = "https://ods.fyi/v2/ods-data";
    let m2_base = "https://ghcr.io/v2/olizilla/ods-data";

    let mut remote_index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("0.1.0", &fix.manifest_digest)],
    )]);
    remote_index.mirrors = vec![
        MirrorEntry { url: m1_base.to_string() },
        MirrorEntry { url: m2_base.to_string() },
    ];

    let mut responses = BTreeMap::new();
    let mut errors = BTreeMap::new();

    responses.insert(format!("{}/manifests/{}", m1_base, fix.manifest_digest), fix.manifest_bytes.clone());

    for (name, digest, bytes) in &fix.files {
        let u1 = format!("{}/blobs/{}", m1_base, digest);
        let u2 = format!("{}/blobs/{}", m2_base, digest);
        if name == "roles.parquet" {
            errors.insert(u1, "HTTP 404".to_string());
            errors.insert(u2, "HTTP 404".to_string());
        } else {
            responses.insert(u1, bytes.clone());
            responses.insert(u2, bytes.clone());
        }
    }

    let fetcher = TestOciFetcher {
        remote_index: Some(remote_index),
        responses,
        errors,
    };

    let mut stderr_buf = Vec::new();
    let res = run_with_fetcher_and_writer(
        Args {
            release_date: Some("2026-07-31".to_string()),
            ..Default::default()
        },
        &workspace,
        &fetcher,
        &mut stderr_buf,
    );

    assert!(res.is_err());
    let err = res.unwrap_err();
    assert!(err.chain().any(|c| c.downcast_ref::<AlreadyReported>().is_some()));

    let rel_dir = workspace.join("releases").join("2026-07-31");
    assert!(rel_dir.exists());

    assert!(!rel_dir.join("roles.parquet").exists());
    assert!(!rel_dir.join("roles.parquet.bad-sha").exists());

    let stderr = String::from_utf8(stderr_buf)?;
    assert!(stderr.contains("roles.parquet  not downloaded"));
    assert!(stderr.contains("ods.fyi   HTTP 404"));
    assert!(stderr.contains("ghcr.io   HTTP 404"));
    assert!(stderr.contains("current is not set"));

    Ok(())
}

#[test]
fn test_pull_force_over_verified_release_with_bad_layer_keeps_verified_copy() -> Result<()> {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");

    let fix = make_standard_6_file_fixture(tmp.path(), "2026-07-31", "0.1.0")?;
    let m1_base = "https://ods.fyi/v2/ods-data";

    let mut remote_index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("0.1.0", &fix.manifest_digest)],
    )]);
    remote_index.mirrors = vec![MirrorEntry { url: m1_base.to_string() }];

    // Pre-install verified release 2026-07-31
    let rel_dir = workspace.join("releases").join("2026-07-31");
    fs::create_dir_all(&rel_dir)?;
    let mut orig_hashes = BTreeMap::new();
    for (name, _, bytes) in &fix.files {
        let p = rel_dir.join(name);
        fs::write(&p, bytes)?;
        let h = format!("{:x}", sha2::Sha256::digest(bytes));
        orig_hashes.insert(name.clone(), h);
    }

    // Upstream now serves bad successions.parquet
    let mut responses = BTreeMap::new();
    responses.insert(format!("{}/manifests/{}", m1_base, fix.manifest_digest), fix.manifest_bytes.clone());

    for (name, digest, bytes) in &fix.files {
        let url = format!("{}/blobs/{}", m1_base, digest);
        if name == "successions.parquet" {
            responses.insert(url, b"bad successions bytes".to_vec());
        } else {
            responses.insert(url, bytes.clone());
        }
    }

    let fetcher = TestOciFetcher {
        remote_index: Some(remote_index),
        responses,
        ..Default::default()
    };

    let mut stderr_buf = Vec::new();
    let res = run_with_fetcher_and_writer(
        Args {
            release_date: Some("2026-07-31".to_string()),
            force: true,
            ..Default::default()
        },
        &workspace,
        &fetcher,
        &mut stderr_buf,
    );

    assert!(res.is_err());
    let err = res.unwrap_err();
    assert!(err.chain().any(|c| c.downcast_ref::<AlreadyReported>().is_some()));

    // Assert existing directory files and hashes are completely unchanged
    for (name, orig_h) in &orig_hashes {
        let content = fs::read(rel_dir.join(name))?;
        let current_h = format!("{:x}", sha2::Sha256::digest(&content));
        assert_eq!(&current_h, orig_h, "File {} hash must remain unchanged", name);
    }
    assert!(!rel_dir.join("successions.parquet.bad-sha").exists());

    // Assert no staging directory remains in scratch/
    let scratch_dir = workspace.join("scratch");
    if scratch_dir.exists() {
        let entries: Vec<_> = fs::read_dir(&scratch_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("pull_"))
            .collect();
        assert!(entries.is_empty(), "scratch staging dir must be removed");
    }

    let stderr = String::from_utf8(stderr_buf)?;
    assert!(stderr.contains("✖ 2026-07-31 (0.1.0) arrived incomplete: 1 of 6 files failed verification on every mirror"));
    assert!(stderr.contains("successions.parquet\n    expected  "));
    assert!(stderr.contains("Kept the verified copy already in releases/2026-07-31/"));
    assert!(stderr.contains("Report it: https://github.com/olizilla/ods/issues"));

    Ok(())
}

#[test]
fn test_pull_recovers_from_partial_install_when_upstream_fixed() -> Result<()> {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");

    let fix = make_standard_6_file_fixture(tmp.path(), "2026-07-31", "0.1.0")?;
    let m1_base = "https://ods.fyi/v2/ods-data";

    let mut remote_index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("0.1.0", &fix.manifest_digest)],
    )]);
    remote_index.mirrors = vec![MirrorEntry { url: m1_base.to_string() }];

    let mut responses = BTreeMap::new();
    responses.insert(format!("{}/manifests/{}", m1_base, fix.manifest_digest), fix.manifest_bytes.clone());

    let mut succ_url = String::new();
    let mut succ_good_bytes = Vec::new();
    for (name, digest, bytes) in &fix.files {
        let url = format!("{}/blobs/{}", m1_base, digest);
        if name == "successions.parquet" {
            succ_url = url.clone();
            succ_good_bytes = bytes.clone();
            responses.insert(url, b"bad successions bytes".to_vec());
        } else {
            responses.insert(url, bytes.clone());
        }
    }

    let mut fetcher = TestOciFetcher {
        remote_index: Some(remote_index.clone()),
        responses,
        ..Default::default()
    };

    // 1st pull: fails verification, installs incomplete with .bad-sha
    let res1 = run_with_fetcher(
        Args {
            release_date: Some("2026-07-31".to_string()),
            ..Default::default()
        },
        &workspace,
        &fetcher,
    );
    assert!(res1.is_err());
    let rel_dir = workspace.join("releases").join("2026-07-31");
    assert!(rel_dir.join("successions.parquet.bad-sha").exists());
    assert!(!rel_dir.join("successions.parquet").exists());

    // Fix upstream
    fetcher.responses.insert(succ_url, succ_good_bytes.clone());

    // 2nd pull: recovers and installs fully verified release
    let res2 = run_with_fetcher(
        Args {
            release_date: Some("2026-07-31".to_string()),
            ..Default::default()
        },
        &workspace,
        &fetcher,
    );
    assert!(res2.is_ok(), "Second pull after upstream fix must succeed");

    assert!(!rel_dir.join("successions.parquet.bad-sha").exists(), ".bad-sha must be gone");
    assert!(rel_dir.join("successions.parquet").exists());
    assert_eq!(fs::read(rel_dir.join("successions.parquet"))?, succ_good_bytes);

    let outcome = ods::workspace::verify_release_dir(&rel_dir, &remote_index);
    assert!(outcome.is_verified(), "recovered release must be verified");

    let (active_date, _) = Workspace::open(Some(&workspace))?.active_release()?;
    assert_eq!(active_date, "2026-07-31");

    Ok(())
}

#[test]
fn test_pull_withdrawn_release_with_bad_layer_prints_both_and_exits_1() -> Result<()> {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");

    // Establish pre-existing active release 2026-08-28
    let ws = Workspace::open_or_create(Some(&workspace))?;
    let rel_28 = workspace.join("releases").join("2026-08-28");
    fs::create_dir_all(&rel_28)?;
    fs::write(rel_28.join("dummy.txt"), b"active release 28")?;
    ws.set_active("2026-08-28")?;

    let fix = make_standard_6_file_fixture(tmp.path(), "2026-07-31", "0.1.0")?;
    let m1_base = "https://ods.fyi/v2/ods-data";

    let mut remote_index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("0.1.0", &fix.manifest_digest)],
    )]);
    remote_index.mirrors = vec![MirrorEntry { url: m1_base.to_string() }];
    remote_index.releases[0].datasets[0].withdrawn =
        Some("orgs.parquet lost the Welsh sites".to_string());

    let mut responses = BTreeMap::new();
    responses.insert(format!("{}/manifests/{}", m1_base, fix.manifest_digest), fix.manifest_bytes.clone());

    for (name, digest, bytes) in &fix.files {
        let url = format!("{}/blobs/{}", m1_base, digest);
        if name == "successions.parquet" {
            responses.insert(url, b"bad successions bytes".to_vec());
        } else {
            responses.insert(url, bytes.clone());
        }
    }

    let fetcher = TestOciFetcher {
        remote_index: Some(remote_index),
        responses,
        ..Default::default()
    };

    let mut stderr_buf = Vec::new();
    let res = run_with_fetcher_and_writer(
        Args {
            release_date: Some("2026-07-31".to_string()),
            ..Default::default()
        },
        &workspace,
        &fetcher,
        &mut stderr_buf,
    );

    assert!(res.is_err());
    let err = res.unwrap_err();
    assert!(err.chain().any(|c| c.downcast_ref::<AlreadyReported>().is_some()));

    let stderr = String::from_utf8(stderr_buf)?;

    // Assert both blocks printed, with incomplete block first, then withdrawal line
    let incomplete_idx = stderr.find("arrived incomplete: 1 of 6 files failed verification").expect("incomplete block");
    let withdrawn_idx = stderr.find("was withdrawn: orgs.parquet lost the Welsh sites").expect("withdrawal block");
    assert!(incomplete_idx < withdrawn_idx, "incomplete block must print before withdrawal line");

    assert!(stderr.contains("current is still releases/2026-08-28"));
    assert!(stderr.contains("Pull a valid release: ods pull"));

    let (active_date, _) = Workspace::open(Some(&workspace))?.active_release()?;
    assert_eq!(active_date, "2026-08-28", "current must remain 2026-08-28");

    Ok(())
}

#[test]
fn test_pull_bad_layer_real_http_integration() -> Result<()> {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;

    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");

    // Establish pre-existing active release 2026-08-28
    let ws = Workspace::open_or_create(Some(&workspace))?;
    let rel_28 = workspace.join("releases").join("2026-08-28");
    fs::create_dir_all(&rel_28)?;
    fs::write(rel_28.join("dummy.txt"), b"active release 28")?;
    ws.set_active("2026-08-28")?;

    let fix = make_standard_6_file_fixture(tmp.path(), "2026-07-31", "0.1.0")?;

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let base_url = format!("http://127.0.0.1:{}", port);

    let m1_base = format!("{}/mirror1/v2/ods-data", base_url);
    let m2_base = format!("{}/mirror2/v2/ods-data", base_url);

    let mut routes: std::collections::HashMap<String, Vec<u8>> = std::collections::HashMap::new();

    // Manifests on both mirrors
    routes.insert(format!("/mirror1/v2/ods-data/manifests/{}", fix.manifest_digest), fix.manifest_bytes.clone());
    routes.insert(format!("/mirror2/v2/ods-data/manifests/{}", fix.manifest_digest), fix.manifest_bytes.clone());

    let bad_bytes = b"corrupted bytes from real http server".to_vec();
    for (name, digest, bytes) in &fix.files {
        let p1 = format!("/mirror1/v2/ods-data/blobs/{}", digest);
        let p2 = format!("/mirror2/v2/ods-data/blobs/{}", digest);
        if name == "successions.parquet" {
            routes.insert(p1, bad_bytes.clone());
            routes.insert(p2, bad_bytes.clone());
        } else {
            routes.insert(p1, bytes.clone());
            routes.insert(p2, bytes.clone());
        }
    }

    let mut remote_index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("0.1.0", &fix.manifest_digest)],
    )]);
    remote_index.mirrors = vec![
        MirrorEntry { url: m1_base },
        MirrorEntry { url: m2_base },
    ];

    let index_file = tmp.path().join("index.json");
    fs::write(&index_file, serde_json::to_vec_pretty(&remote_index)?)?;

    let routes = std::sync::Arc::new(routes);
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        loop {
            if rx.try_recv().is_ok() {
                break;
            }
            if let Ok((mut stream, _)) = listener.accept() {
                let _ = stream.set_nonblocking(false);
                let routes = routes.clone();
                thread::spawn(move || {
                    let mut buf = [0u8; 4096];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    if n > 0 {
                        let line_end = buf[..n].iter().position(|&b| b == b'\r' || b == b'\n').unwrap_or(n);
                        if let Ok(line_str) = std::str::from_utf8(&buf[..line_end]) {
                            let mut parts = line_str.split_whitespace();
                            let _method = parts.next();
                            if let Some(full_path) = parts.next() {
                                let path = full_path.split('?').next().unwrap_or(full_path);
                                if let Some(body) = routes.get(path) {
                                    let header = format!(
                                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                        body.len()
                                    );
                                    let _ = stream.write_all(header.as_bytes());
                                    let _ = stream.write_all(body);
                                    let _ = stream.flush();
                                    return;
                                }
                            }
                        }
                        let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                        let _ = stream.write_all(resp.as_bytes());
                        let _ = stream.flush();
                    }
                });
            }
            thread::sleep(std::time::Duration::from_millis(1));
        }
    });

    let out = common::ods_cmd()
        .current_dir(tmp.path())
        .args(["pull", "2026-07-31", "--index", index_file.to_str().unwrap()])
        .output()?;

    let _ = tx.send(());

    assert_eq!(out.status.code(), Some(1), "must exit 1");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("arrived incomplete: 1 of 6 files failed verification on every mirror"));
    assert!(stderr.contains("successions.parquet → releases/2026-07-31/successions.parquet.bad-sha"));

    let rel_dir = workspace.join("releases").join("2026-07-31");
    assert!(rel_dir.exists());
    assert!(rel_dir.join("successions.parquet.bad-sha").exists());
    assert_eq!(fs::read(rel_dir.join("successions.parquet.bad-sha"))?, bad_bytes);

    let (active_date, _) = Workspace::open(Some(&workspace))?.active_release()?;
    assert_eq!(active_date, "2026-08-28", "current must remain 2026-08-28");

    Ok(())
}

#[test]
fn test_pull_all_records_arrived_incomplete_in_ledger() -> Result<()> {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");

    let fix = make_standard_6_file_fixture(tmp.path(), "2026-07-31", "0.1.0")?;

    let remote_index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("0.1.0", &fix.manifest_digest)],
    )]);

    let mut responses = BTreeMap::new();
    let manifest_url = format!("https://ods.fyi/v2/ods-data/manifests/{}", fix.manifest_digest);
    responses.insert(manifest_url, fix.manifest_bytes.clone());

    for (name, digest, bytes) in &fix.files {
        let url = format!("https://ods.fyi/v2/ods-data/blobs/{}", digest);
        if name == "successions.parquet" {
            responses.insert(url, b"corrupted bytes".to_vec());
        } else {
            responses.insert(url, bytes.clone());
        }
    }

    let fetcher = TestOciFetcher {
        remote_index: Some(remote_index),
        responses,
        ..Default::default()
    };

    let mut stderr_buf = Vec::new();
    let res = run_with_fetcher_and_writer(
        Args {
            all: true,
            ..Default::default()
        },
        &workspace,
        &fetcher,
        &mut stderr_buf,
    );

    assert!(res.is_err());
    let err = res.unwrap_err();
    assert!(
        err.chain().any(|c| c.downcast_ref::<ods::commands::pull::AlreadyReported>().is_some()),
        "a batch with a failure must return AlreadyReported, since the ledger is already printed: {:#}",
        err
    );

    let stderr = String::from_utf8_lossy(&stderr_buf);
    assert!(stderr.contains("arrived incomplete: 1 of 6 files failed verification"));
    assert!(
        stderr.contains("pulled        0 releases · 1 failed · 0B"),
        "ledger must report the failure in the pulled row, got:\n{}",
        stderr
    );

    Ok(())
}

/// `make_standard_6_file_fixture`'s file bytes are fixed strings, so three releases built
/// from it share the same digest for `successions.parquet` — corrupting one corrupts all
/// three. This variant folds `date` into every file's bytes so three releases in the same
/// test never collide on a blob digest.
fn make_unique_6_file_fixture(tmp: &Path, date: &str, version: &str) -> Result<StandardFixture> {
    let fix_dir = tmp.join(format!("uniq_{}_{}", date, version));
    fs::create_dir_all(&fix_dir)?;

    let prov = ods::provenance::OdsProvenance {
        trud_release_date: Some(date.to_string()),
        trud_release_sha256: Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string()),
        trud_release_sha256_verified: Some(ods::provenance::TrudVerificationSource::TrudApi),
        ..Default::default()
    };
    let prov_bytes = serde_json::to_vec_pretty(&prov)?;

    let dp = serde_json::json!({ "name": "ods", "version": version, "resources": [] });
    let dp_bytes = serde_json::to_vec_pretty(&dp)?;

    let orgs_bytes = format!("sample parquet orgs 123 {}", date).into_bytes();
    let rel_bytes = format!("sample parquet rel 123 {}", date).into_bytes();
    let roles_bytes = format!("sample parquet roles 123 {}", date).into_bytes();
    let succ_bytes = format!("sample parquet succ 123 {}", date).into_bytes();

    fs::write(fix_dir.join(ods::provenance::PROVENANCE_FILENAME), &prov_bytes)?;
    fs::write(fix_dir.join(ods::datapackage::DATAPACKAGE_FILENAME), &dp_bytes)?;
    fs::write(fix_dir.join("orgs.parquet"), &orgs_bytes)?;
    fs::write(fix_dir.join("relationships.parquet"), &rel_bytes)?;
    fs::write(fix_dir.join("roles.parquet"), &roles_bytes)?;
    fs::write(fix_dir.join("successions.parquet"), &succ_bytes)?;

    let (manifest, manifest_bytes) = ods::commands::make_oci::build_manifest_from_dir(&fix_dir, &prov, version)?;
    let manifest_digest = manifest.digest()?;

    let mut files = Vec::new();
    for layer in &manifest.layers {
        let title = layer
            .annotations
            .as_ref()
            .and_then(|a| a.get("org.opencontainers.image.title"))
            .cloned()
            .unwrap();
        let bytes = match title.as_str() {
            ods::provenance::PROVENANCE_FILENAME => prov_bytes.clone(),
            ods::datapackage::DATAPACKAGE_FILENAME => dp_bytes.clone(),
            "orgs.parquet" => orgs_bytes.clone(),
            "relationships.parquet" => rel_bytes.clone(),
            "roles.parquet" => roles_bytes.clone(),
            "successions.parquet" => succ_bytes.clone(),
            other => panic!("Unexpected layer: {}", other),
        };
        files.push((title, layer.digest.clone(), bytes));
    }

    Ok(StandardFixture { manifest_digest, manifest_bytes, files })
}

#[test]
fn test_pull_all_continues_past_an_incomplete_release_and_exits_1() -> Result<()> {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");

    let fix1 = make_unique_6_file_fixture(tmp.path(), "2026-06-26", "0.1.0")?;
    let fix2 = make_unique_6_file_fixture(tmp.path(), "2026-07-31", "0.1.0")?;
    let fix3 = make_unique_6_file_fixture(tmp.path(), "2026-08-28", "0.1.0")?;

    let remote_index = make_v1_index(&[
        (
            "2026-08-28",
            "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
            37983173,
            &[("0.1.0", &fix3.manifest_digest)],
        ),
        (
            "2026-07-31",
            "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
            37983173,
            &[("0.1.0", &fix2.manifest_digest)],
        ),
        (
            "2026-06-26",
            "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
            37983173,
            &[("0.1.0", &fix1.manifest_digest)],
        ),
    ]);

    let mut responses = BTreeMap::new();
    for fix in [&fix1, &fix3] {
        responses.insert(
            format!("https://ods.fyi/v2/ods-data/manifests/{}", fix.manifest_digest),
            fix.manifest_bytes.clone(),
        );
        for (_, digest, bytes) in &fix.files {
            responses.insert(format!("https://ods.fyi/v2/ods-data/blobs/{}", digest), bytes.clone());
        }
    }
    // 2026-07-31 (the middle release) arrives incomplete: one layer is corrupted.
    responses.insert(
        format!("https://ods.fyi/v2/ods-data/manifests/{}", fix2.manifest_digest),
        fix2.manifest_bytes.clone(),
    );
    for (name, digest, bytes) in &fix2.files {
        let url = format!("https://ods.fyi/v2/ods-data/blobs/{}", digest);
        if name == "successions.parquet" {
            responses.insert(url, b"corrupted bytes".to_vec());
        } else {
            responses.insert(url, bytes.clone());
        }
    }

    let fetcher = TestOciFetcher {
        remote_index: Some(remote_index),
        responses,
        ..Default::default()
    };

    let mut stderr_buf = Vec::new();
    let res = run_with_fetcher_and_writer(
        Args {
            all: true,
            ..Default::default()
        },
        &workspace,
        &fetcher,
        &mut stderr_buf,
    );

    assert!(res.is_err());
    let err = res.unwrap_err();
    assert!(
        err.chain().any(|c| c.downcast_ref::<ods::commands::pull::AlreadyReported>().is_some()),
        "must return AlreadyReported: {:#}",
        err
    );

    // Every release was attempted: the batch didn't stop at 2026-07-31's failure — the newer
    // 2026-08-28 was still pulled.
    assert!(workspace.join("releases").join("2026-06-26").join("orgs.parquet").exists());
    assert!(workspace.join("releases").join("2026-08-28").join("orgs.parquet").exists());

    // The pin lands on the newest release that verified in full, past the failure.
    let (active_date, _) = Workspace::open(Some(&workspace))?.active_release()?;
    assert_eq!(active_date, "2026-08-28", "pin must land on the newest fully-verified release");

    let stderr = String::from_utf8_lossy(&stderr_buf);
    let pulled_idx = stderr.find("pulled        2 releases").expect("pulled row");
    let linked_idx = stderr.find("linked        current → releases/2026-08-28").expect("linked row");
    let incomplete_idx = stderr.find("arrived incomplete: 1 of 6 files failed verification").expect("incomplete block");
    assert!(pulled_idx < linked_idx, "pulled must print before linked");
    assert!(linked_idx < incomplete_idx, "the pin must move, and linked print, before the failure block");
    assert!(
        stderr.contains("current is still releases/2026-08-28"),
        "the incomplete block must name the pin as it stands once the batch has moved it, got:\n{}",
        stderr
    );

    Ok(())
}



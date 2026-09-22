mod common;

use anyhow::Result;
use common::make_v1_index;
use ods::index::OdsReleaseIndex;

#[test]
fn test_baked_index_parses_and_validates() -> Result<()> {
    let index = OdsReleaseIndex::baked()?;
    assert_eq!(index.schema, ods::index::RELEASES_SCHEMA_V1_URL);
    assert_eq!(
        index.trud_signing_key_fingerprint,
        "71ED5964BAE53E83556320A42BE59DADEE84BEB0"
    );
    assert_eq!(index.mirrors.len(), 2);
    Ok(())
}

#[test]
fn test_baked_index_matches_its_schema() -> Result<()> {
    let schema_str = include_str!("../worker/schema/releases.v1.json");
    let schema_json: serde_json::Value = serde_json::from_str(schema_str)?;
    let validator = jsonschema::validator_for(&schema_json)
        .map_err(|e| anyhow::anyhow!("Invalid schema: {}", e))?;

    // 1. Validate baked data/releases.json
    let baked_str = include_str!("../data/releases.json");
    let baked_json: serde_json::Value = serde_json::from_str(baked_str)?;
    let errors: Vec<_> = validator.iter_errors(&baked_json).collect();
    assert!(errors.is_empty(), "data/releases.json schema errors: {:?}", errors);

    // 2. Validate index built by Task 7's test builder
    let built_index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("1.0.0", "sha256:0f2a000000000000000000000000000000000000000000000000000000000000")],
    )]);
    let built_json: serde_json::Value = serde_json::to_value(&built_index)?;
    let built_errors: Vec<_> = validator.iter_errors(&built_json).collect();
    assert!(built_errors.is_empty(), "Built index schema errors: {:?}", built_errors);

    Ok(())
}

#[test]
fn test_built_provenance_matches_its_schema() -> Result<()> {
    let schema_str = include_str!("../worker/schema/provenance.v1.json");
    let schema_json: serde_json::Value = serde_json::from_str(schema_str)?;
    let validator = jsonschema::validator_for(&schema_json)
        .map_err(|e| anyhow::anyhow!("Invalid schema: {}", e))?;

    // 1. Validate provenance of a release built by setup_synthetic_repo_and_release()
    let (_tmp, rel_dir) = common::setup_synthetic_repo_and_release();
    let prov_str = std::fs::read_to_string(rel_dir.join("_provenance.json"))?;
    let prov_json: serde_json::Value = serde_json::from_str(&prov_str)?;
    let errors: Vec<_> = validator.iter_errors(&prov_json).collect();
    assert!(errors.is_empty(), "Synthetic release provenance schema errors: {:?}", errors);

    // 2. Validate provenance written by ods trud pull against the mock zip
    let tmp_pull = tempfile::TempDir::new()?;
    let fixture_zip = tmp_pull.path().join("hscorgrefdataxml_data_7.0.0_20260731000001.zip");
    let xml_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<un:OrganisationManifest xmlns:un="http://refdata.hscic.gov.uk/org/v2-0-0">
  <un:ManifestHeader>
    <un:Version value="2-0-0" />
    <un:RecordCount value="305541" />
  </un:ManifestHeader>
</un:OrganisationManifest>"#;
    let inner_bytes = common::create_inner_zip("HSCOrgRefData_Full_20260731.xml", xml_content.as_bytes());
    common::create_nested_trud_zip(&fixture_zip, &[("fullfile.zip", &inner_bytes)]);

    let pull_out = tmp_pull.path().join("releases").join("2026-07-31");
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_ods"))
        .current_dir(tmp_pull.path())
        .arg("trud")
        .arg("pull")
        .arg("--local-archive")
        .arg(&fixture_zip)
        .arg("-o")
        .arg(&pull_out)
        .status()?;
    assert!(status.success(), "trud pull must succeed");
    let pull_prov_str = std::fs::read_to_string(pull_out.join("_provenance.json"))?;
    let pull_prov_json: serde_json::Value = serde_json::from_str(&pull_prov_str)?;
    let pull_errors: Vec<_> = validator.iter_errors(&pull_prov_json).collect();
    assert!(pull_errors.is_empty(), "trud pull provenance schema errors: {:?}", pull_errors);

    // 3. Verify that adding dropped keys causes validation to fail
    let dropped_keys = ["dataset_version", "trud_release_name", "trud_release_file"];
    for key in dropped_keys {
        let mut mutated = prov_json.clone();
        mutated[key] = serde_json::json!("should_fail");
        let errs: Vec<_> = validator.iter_errors(&mutated).collect();
        assert!(!errs.is_empty(), "Schema validation must fail when dropped key '{}' is added back", key);
    }

    Ok(())
}

#[test]
fn test_index_rejects_duplicate_version_in_release() {
    let json = r#"{
      "$schema": "https://ods.fyi/schema/releases.v1.json",
      "trud_signing_key_fingerprint": "71ED5964BAE53E83556320A42BE59DADEE84BEB0",
      "mirrors": [],
      "releases": [
        {
          "trud_release_date": "2026-07-31",
          "trud_release_sha256": "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
          "trud_release_filesize_bytes": 37983173,
          "datasets": [
            {
              "dataset_version": "1.0.0",
              "manifest_digest": "sha256:0f2a000000000000000000000000000000000000000000000000000000000000"
            },
            {
              "dataset_version": "1.0.0",
              "manifest_digest": "sha256:7c4a000000000000000000000000000000000000000000000000000000000000"
            }
          ]
        }
      ]
    }"#;

    let index: OdsReleaseIndex = serde_json::from_str(json).unwrap();
    let result = index.validate();
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("Duplicate dataset_version '1.0.0'"));
}

#[test]
fn test_index_rejects_uppercase_manifest_digest() {
    let json = r#"{
      "$schema": "https://ods.fyi/schema/releases.v1.json",
      "trud_signing_key_fingerprint": "71ED5964BAE53E83556320A42BE59DADEE84BEB0",
      "mirrors": [],
      "releases": [
        {
          "trud_release_date": "2026-07-31",
          "trud_release_sha256": "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
          "trud_release_filesize_bytes": 37983173,
          "datasets": [
            {
              "dataset_version": "1.0.0",
              "manifest_digest": "sha256:0F2A000000000000000000000000000000000000000000000000000000000000"
            }
          ]
        }
      ]
    }"#;

    let index: OdsReleaseIndex = serde_json::from_str(json).unwrap();
    let result = index.validate();
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("64 lower-case hex"));
}

#[test]
fn test_merge_allows_new_releases() -> Result<()> {
    let baked = make_v1_index(&[(
        "2019-11-29",
        "3B7F91A200000000000000000000000000000000000000000000000000000000",
        1000,
        &[("1.0.0", "sha256:c1a8000000000000000000000000000000000000000000000000000000000000")],
    )]);

    let fetched = make_v1_index(&[
        (
            "2026-07-31",
            "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
            37983173,
            &[("1.0.1", "sha256:0f2a000000000000000000000000000000000000000000000000000000000000")],
        ),
        (
            "2019-11-29",
            "3B7F91A200000000000000000000000000000000000000000000000000000000",
            1000,
            &[("1.0.0", "sha256:c1a8000000000000000000000000000000000000000000000000000000000000")],
        ),
    ]);

    let merged = baked.merge(&fetched)?;
    assert_eq!(merged.releases.len(), 2);
    Ok(())
}

#[test]
fn test_merge_rejects_contradiction_on_baked_release() {
    let baked = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("1.0.0", "sha256:0f2a000000000000000000000000000000000000000000000000000000000000")],
    )]);

    let fetched = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("1.0.0", "sha256:ffff000000000000000000000000000000000000000000000000000000000000")],
    )]);

    let result = baked.merge(&fetched);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("Security error: fetched index contradicts baked dataset"));
}

#[test]
fn test_resolve_prefers_highest_non_withdrawn_semver() -> Result<()> {
    let mut index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[
            ("1.0.0", "sha256:7c4a000000000000000000000000000000000000000000000000000000000000"),
            ("1.0.1", "sha256:0f2a000000000000000000000000000000000000000000000000000000000000"),
        ],
    )]);
    index.releases[0].datasets[0].withdrawn = Some("roles table truncated".to_string());

    let res = index.resolve(Some("2026-07-31"))?;
    assert_eq!(res.release.trud_release_date, "2026-07-31");
    assert_eq!(res.dataset.dataset_version, "1.0.1");
    Ok(())
}

#[test]
fn test_resolve_delivers_withdrawn_when_all_versions_for_date_withdrawn() -> Result<()> {
    let mut index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("1.0.0", "sha256:7c4a000000000000000000000000000000000000000000000000000000000000")],
    )]);
    index.releases[0].datasets[0].withdrawn =
        Some("roles table truncated at 65535 rows by a bad build".to_string());

    let res = index.resolve(Some("2026-07-31"))?;
    assert_eq!(res.release.trud_release_date, "2026-07-31");
    assert_eq!(res.dataset.dataset_version, "1.0.0");
    assert!(res.dataset.is_withdrawn());
    assert_eq!(
        res.dataset.withdrawn.as_deref(),
        Some("roles table truncated at 65535 rows by a bad build")
    );
    Ok(())
}

#[test]
fn test_merge_propagates_withdrawal_to_baked_release() -> Result<()> {
    let baked = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("1.0.0", "sha256:0f2a000000000000000000000000000000000000000000000000000000000000")],
    )]);

    let mut fetched = baked.clone();
    fetched.releases[0].datasets[0].withdrawn =
        Some("roles table truncated at 65535 rows by a bad build".to_string());

    let merged = baked.merge(&fetched)?;
    assert_eq!(merged.releases.len(), 1);
    assert_eq!(
        merged.releases[0].datasets[0].withdrawn.as_deref(),
        Some("roles table truncated at 65535 rows by a bad build")
    );
    Ok(())
}

#[test]
fn test_resolve_none_skips_withdrawn_newest_date() -> Result<()> {
    let mut index = make_v1_index(&[
        (
            "2026-07-31",
            "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
            37983173,
            &[("1.0.0", "sha256:2222000000000000000000000000000000000000000000000000000000000000")],
        ),
        (
            "2026-05-29",
            "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
            37983173,
            &[("1.0.0", "sha256:1111000000000000000000000000000000000000000000000000000000000000")],
        ),
    ]);
    index.releases[0].datasets[0].withdrawn = Some("critical corruption in roles".to_string());

    let res = index.resolve(None)?;
    assert_eq!(res.release.trud_release_date, "2026-05-29");
    assert_eq!(res.dataset.dataset_version, "1.0.0");
    assert_eq!(res.skipped.len(), 1);
    assert_eq!(res.skipped[0].0.trud_release_date, "2026-07-31");
    assert_eq!(res.skipped[0].1.dataset_version, "1.0.0");
    Ok(())
}

#[test]
fn test_resolve_none_fails_when_every_release_is_withdrawn() {
    let mut index = make_v1_index(&[
        (
            "2026-07-31",
            "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
            37983173,
            &[("1.0.0", "sha256:2222000000000000000000000000000000000000000000000000000000000000")],
        ),
    ]);
    index.releases[0].datasets[0].withdrawn = Some("critical corruption in roles".to_string());

    let result = index.resolve(None);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("Every release in the index is withdrawn"));
    assert!(err.contains("See them: ods pull --list"));
}

#[test]
fn test_mirror_urls_and_host() {
    let mirror = ods::index::MirrorEntry {
        url: "https://ods.fyi/v2/ods-data".to_string(),
    };
    assert_eq!(
        mirror.blob_url("sha256:0f2a000000000000000000000000000000000000000000000000000000000000"),
        "https://ods.fyi/v2/ods-data/blobs/sha256:0f2a000000000000000000000000000000000000000000000000000000000000"
    );
    assert_eq!(
        mirror.manifest_url("2026-07-31_1.0.1"),
        "https://ods.fyi/v2/ods-data/manifests/2026-07-31_1.0.1"
    );
    assert_eq!(mirror.host(), "ods.fyi");

    let ghcr = ods::index::MirrorEntry {
        url: "https://ghcr.io/v2/olizilla/ods-data".to_string(),
    };
    assert_eq!(ghcr.host(), "ghcr.io");
}

#[test]
fn test_workspace_release_index_save_and_load() -> Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let baked = OdsReleaseIndex::baked()?;

    assert!(ods::index::OdsReleaseIndex::load_from_workspace(tmp.path())?.is_none());
    ods::index::OdsReleaseIndex::save_to_workspace_bytes(
        ods::index::BAKED_RELEASES_JSON_BYTES,
        tmp.path(),
    )?;

    let loaded = ods::index::OdsReleaseIndex::load_from_workspace(tmp.path())?.unwrap();
    assert_eq!(loaded.schema, baked.schema);
    assert_eq!(
        loaded.trud_signing_key_fingerprint,
        baked.trud_signing_key_fingerprint
    );
    Ok(())
}

#[test]
fn test_resolve_none_fails_when_all_releases_have_empty_datasets() {
    let mut index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("1.0.0", "sha256:2222000000000000000000000000000000000000000000000000000000000000")],
    )]);
    index.releases[0].datasets.clear();

    let result = index.resolve(None);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert_eq!(err, "Release index contains no releases");
}

#[test]
fn test_resolve_none_fails_when_releases_is_empty() {
    let index = make_v1_index(&[]);
    let result = index.resolve(None);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert_eq!(err, "Release index contains no releases");
}

#[test]
fn test_select_dataset_helper() {
    let mut index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[
            ("1.0.0", "sha256:1111000000000000000000000000000000000000000000000000000000000000"),
            ("1.0.1", "sha256:2222000000000000000000000000000000000000000000000000000000000000"),
            ("1.0.2", "sha256:3333000000000000000000000000000000000000000000000000000000000000"),
        ],
    )]);

    // With all active: returns highest active (1.0.2, false)
    let (ds, is_withdrawn) = ods::index::select_dataset(&index.releases[0]).unwrap();
    assert_eq!(ds.dataset_version, "1.0.2");
    assert!(!is_withdrawn);

    // With 1.0.2 withdrawn: returns highest active (1.0.1, false)
    index.releases[0].datasets[2].withdrawn = Some("bug in 1.0.2".to_string());
    let (ds, is_withdrawn) = ods::index::select_dataset(&index.releases[0]).unwrap();
    assert_eq!(ds.dataset_version, "1.0.1");
    assert!(!is_withdrawn);

    // With all withdrawn: returns highest withdrawn (1.0.2, true)
    index.releases[0].datasets[0].withdrawn = Some("bug in 1.0.0".to_string());
    index.releases[0].datasets[1].withdrawn = Some("bug in 1.0.1".to_string());
    let (ds, is_withdrawn) = ods::index::select_dataset(&index.releases[0]).unwrap();
    assert_eq!(ds.dataset_version, "1.0.2");
    assert!(is_withdrawn);

    // With empty datasets: returns None
    index.releases[0].datasets.clear();
    assert!(ods::index::select_dataset(&index.releases[0]).is_none());
}


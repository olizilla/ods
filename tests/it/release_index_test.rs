use crate::common;

use anyhow::Result;
use common::make_v1_index;
use ods::index::OdsReleaseIndex;

#[test]
fn test_baked_index_parses_and_validates() -> Result<()> {
    // The compiled data/releases.json itself: `baked()` reads the fixture in a test process.
    let index = OdsReleaseIndex::from_slice(ods::index::BAKED_RELEASES_JSON_BYTES)?;
    index.validate()?;
    assert_eq!(index.schema, ods::index::RELEASES_SCHEMA_V1_URL);
    assert_eq!(index.name, "ods-data");
    assert_eq!(index.source.title, "NHS Organisation Data Service XML Data");
    assert_eq!(
        index.source.signing_key_fingerprints,
        Some(vec!["71ED5964BAE53E83556320A42BE59DADEE84BEB0".to_string()])
    );
    assert_eq!(index.mirrors.len(), 2);
    Ok(())
}

#[test]
fn test_baked_index_matches_its_schema() -> Result<()> {
    let schema_str = include_str!("../../worker/schema/releases.v1.json");
    let schema_json: serde_json::Value = serde_json::from_str(schema_str)?;
    let validator = jsonschema::validator_for(&schema_json)
        .map_err(|e| anyhow::anyhow!("Invalid schema: {}", e))?;

    // 1. Validate baked data/releases.json
    let baked_str = include_str!("../../data/releases.json");
    let baked_json: serde_json::Value = serde_json::from_str(baked_str)?;
    let errors: Vec<_> = validator.iter_errors(&baked_json).collect();
    println!("data/releases.json validated against worker/schema/releases.v1.json: {} errors", errors.len());
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

    // 3. A source hash in upper case, as TRUD writes it, fails: the index writes sha256: + lower
    let mut upper = built_json.clone();
    let hash = upper["releases"][0]["source"]["hash"].as_str().unwrap().to_string();
    upper["releases"][0]["source"]["hash"] = serde_json::json!(format!("sha256:{}", hash["sha256:".len()..].to_uppercase()));
    let upper_errors: Vec<_> = validator.iter_errors(&upper).collect();
    assert!(!upper_errors.is_empty(), "Schema validation must fail on an upper-case source hash");

    // 4. A dataset version without its source version fails
    let mut bare = built_json.clone();
    bare["releases"][0]["datasets"][0]["version"] = serde_json::json!("1.0.0");
    assert!(validator.iter_errors(&bare).next().is_some(), "Schema validation must fail on a bare dataset version");

    Ok(())
}

// docs/release-index.md's example is an index both the schema and ods accept.
#[test]
fn test_the_documented_example_is_a_valid_index() -> Result<()> {
    let doc = include_str!("../../docs/release-index.md");
    let after = doc.split("## Example").nth(1).expect("an Example section");
    let example = after.split("```json").nth(1).and_then(|b| b.split("```").next()).expect("a json block");

    let schema_json: serde_json::Value = serde_json::from_str(include_str!("../../worker/schema/releases.v1.json"))?;
    let validator = jsonschema::validator_for(&schema_json).map_err(|e| anyhow::anyhow!("Invalid schema: {}", e))?;
    let example_json: serde_json::Value = serde_json::from_str(example)?;
    let errors: Vec<_> = validator.iter_errors(&example_json).collect();
    assert!(errors.is_empty(), "docs/release-index.md example schema errors: {:?}", errors);

    OdsReleaseIndex::from_slice(example.as_bytes())?.validate()?;
    Ok(())
}

#[test]
fn test_built_trud_archive_package_matches_its_schema() -> Result<()> {
    let schema_str = include_str!("../../worker/schema/ods-datapackage.v1.json");
    let schema_json: serde_json::Value = serde_json::from_str(schema_str)?;
    let validator = jsonschema::validator_for(&schema_json)
        .map_err(|e| anyhow::anyhow!("Invalid schema: {}", e))?;

    // 1. Validate the TRUD archive package of a release built by setup_synthetic_repo_and_release()
    let (_tmp, rel_dir) = common::setup_synthetic_repo_and_release();
    let prov_str = std::fs::read_to_string(ods::provenance::trud_archive_package_path(&rel_dir))?;
    let prov_json: serde_json::Value = serde_json::from_str(&prov_str)?;
    let errors: Vec<_> = validator.iter_errors(&prov_json).collect();
    assert!(errors.is_empty(), "Synthetic release provenance schema errors: {:?}", errors);

    // 2. Validate the TRUD archive package written by ods trud pull against the mock zip
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
    let index_file = common::write_index_for_zip(tmp_pull.path(), "2026-07-31", &fixture_zip);
    let status = common::ods_cmd()
        .current_dir(tmp_pull.path())
        .arg("trud")
        .arg("pull")
        .arg("--local-archive")
        .arg(&fixture_zip)
        .arg("--index")
        .arg(&index_file)
        .arg("-o")
        .arg(&pull_out)
        .status()?;
    assert!(status.success(), "trud pull must succeed");
    let pull_prov_str = std::fs::read_to_string(ods::provenance::trud_archive_package_path(&pull_out))?;
    let pull_prov_json: serde_json::Value = serde_json::from_str(&pull_prov_str)?;
    let pull_errors: Vec<_> = validator.iter_errors(&pull_prov_json).collect();
    assert!(pull_errors.is_empty(), "trud pull provenance schema errors: {:?}", pull_errors);

    // 3. A record whose archive size isn't a number fails
    let mut mutated = pull_prov_json.clone();
    mutated["resources"][0]["bytes"] = serde_json::json!("big");
    let errs: Vec<_> = validator.iter_errors(&mutated).collect();
    assert!(!errs.is_empty(), "Schema validation must fail on a resource's bytes of \"big\"");

    Ok(())
}

#[test]
fn test_index_rejects_duplicate_version_in_release() {
    let json = r#"{
      "$schema": "https://ods.fyi/schema/releases.v1.json",
      "name": "ods-data",
      "source": {
        "title": "NHS Organisation Data Service XML Data",
        "path": "https://isd.digital.nhs.uk/trud/users/guest/filters/0/categories/5/items/341/releases",
        "signing_key_fingerprints": [
          "71ED5964BAE53E83556320A42BE59DADEE84BEB0"
        ]
      },
      "mirrors": [],
      "releases": [
        {
          "source": {
            "version": "2026-07-31",
            "hash": "sha256:8151248ddc290f3affdabae22d88e0bbd118947d948ab7bdd37e74088cfba933",
            "bytes": 37983173
          },
          "datasets": [
            {
              "version": "2026-07-31_1.0.0",
              "manifest_digest": "sha256:0f2a000000000000000000000000000000000000000000000000000000000000",
              "bytes": 29700000,
              "tool_version": "0.1.0",
              "tool_git_sha": "0123456789abcdef0123456789abcdef01234567"
            },
            {
              "version": "2026-07-31_1.0.0",
              "manifest_digest": "sha256:7c4a000000000000000000000000000000000000000000000000000000000000",
              "bytes": 29700000,
              "tool_version": "0.1.0",
              "tool_git_sha": "0123456789abcdef0123456789abcdef01234567"
            }
          ]
        }
      ]
    }"#;

    let index: OdsReleaseIndex = serde_json::from_str(json).unwrap();
    let result = index.validate();
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("Duplicate dataset version '2026-07-31_1.0.0'"));
}

#[test]
fn test_index_rejects_uppercase_manifest_digest() {
    let json = r#"{
      "$schema": "https://ods.fyi/schema/releases.v1.json",
      "name": "ods-data",
      "source": {
        "title": "NHS Organisation Data Service XML Data",
        "path": "https://isd.digital.nhs.uk/trud/users/guest/filters/0/categories/5/items/341/releases",
        "signing_key_fingerprints": [
          "71ED5964BAE53E83556320A42BE59DADEE84BEB0"
        ]
      },
      "mirrors": [],
      "releases": [
        {
          "source": {
            "version": "2026-07-31",
            "hash": "sha256:8151248ddc290f3affdabae22d88e0bbd118947d948ab7bdd37e74088cfba933",
            "bytes": 37983173
          },
          "datasets": [
            {
              "version": "2026-07-31_1.0.0",
              "manifest_digest": "sha256:0F2A000000000000000000000000000000000000000000000000000000000000",
              "bytes": 29700000,
              "tool_version": "0.1.0",
              "tool_git_sha": "0123456789abcdef0123456789abcdef01234567"
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
    assert_eq!(res.release.source.version, "2026-07-31");
    assert_eq!(res.dataset.dataset_version(), "1.0.1");
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
    assert_eq!(res.release.source.version, "2026-07-31");
    assert_eq!(res.dataset.dataset_version(), "1.0.0");
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
    assert_eq!(res.release.source.version, "2026-05-29");
    assert_eq!(res.dataset.dataset_version(), "1.0.0");
    assert_eq!(res.skipped.len(), 1);
    assert_eq!(res.skipped[0].0.source.version, "2026-07-31");
    assert_eq!(res.skipped[0].1.dataset_version(), "1.0.0");
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
    let baked = common::baked_fixture();

    assert!(ods::index::OdsReleaseIndex::load_from_workspace(tmp.path())?.is_none());
    ods::index::OdsReleaseIndex::save_to_workspace_bytes(
        &common::baked_fixture_bytes(),
        tmp.path(),
    )?;

    let loaded = ods::index::OdsReleaseIndex::load_from_workspace(tmp.path())?.unwrap();
    assert_eq!(loaded.schema, baked.schema);
    assert_eq!(
        loaded.source.signing_key_fingerprints,
        baked.source.signing_key_fingerprints
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
    assert_eq!(ds.dataset_version(), "1.0.2");
    assert!(!is_withdrawn);

    // With 1.0.2 withdrawn: returns highest active (1.0.1, false)
    index.releases[0].datasets[2].withdrawn = Some("bug in 1.0.2".to_string());
    let (ds, is_withdrawn) = ods::index::select_dataset(&index.releases[0]).unwrap();
    assert_eq!(ds.dataset_version(), "1.0.1");
    assert!(!is_withdrawn);

    // With all withdrawn: returns highest withdrawn (1.0.2, true)
    index.releases[0].datasets[0].withdrawn = Some("bug in 1.0.0".to_string());
    index.releases[0].datasets[1].withdrawn = Some("bug in 1.0.1".to_string());
    let (ds, is_withdrawn) = ods::index::select_dataset(&index.releases[0]).unwrap();
    assert_eq!(ds.dataset_version(), "1.0.2");
    assert!(is_withdrawn);

    // With empty datasets: returns None
    index.releases[0].datasets.clear();
    assert!(ods::index::select_dataset(&index.releases[0]).is_none());
}

use ods::commands::pull::{resolve_index, AlreadyReported, IndexOrigin, OciBlobFetcher};
use tempfile::TempDir;

struct MockPrecedenceFetcher {
    raw_response: Option<(Vec<u8>, String)>,
}

impl OciBlobFetcher for MockPrecedenceFetcher {
    fn fetch_bytes(&self, _url: &str) -> Result<Vec<u8>> {
        unimplemented!()
    }
    fn fetch_release_index_raw(&self) -> Result<Option<(Vec<u8>, String)>> {
        Ok(self.raw_response.clone())
    }
}

#[test]
fn test_precedence_index_flag_wins_over_all() -> Result<()> {
    let tmp = TempDir::new()?;
    let workspace = tmp.path().join("ods_data");
    std::fs::create_dir_all(&workspace)?;

    // Cache index in workspace
    let cache_index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("1.0.0", "sha256:0000000000000000000000000000000000000000000000000000000000000001")],
    )]);
    std::fs::write(workspace.join("_releases.json"), cache_index.to_json_pretty()?)?;

    // Remote index served by fetcher
    let remote_index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("1.0.0", "sha256:0000000000000000000000000000000000000000000000000000000000000002")],
    )]);
    let fetcher = MockPrecedenceFetcher {
        raw_response: Some((remote_index.to_json_pretty()?.into_bytes(), "https://ods.fyi/v2/releases.json".to_string())),
    };

    // Flag index file
    let flag_index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("1.0.0", "sha256:0000000000000000000000000000000000000000000000000000000000000003")],
    )]);
    let flag_path = tmp.path().join("flag_releases.json");
    std::fs::write(&flag_path, flag_index.to_json_pretty()?)?;

    let (index, origin) = resolve_index(
        &workspace,
        Some(flag_path.to_str().unwrap()),
        true,
        false,
        &fetcher,
    )?;

    match origin {
        IndexOrigin::Flag(f) => assert_eq!(f, flag_path.to_str().unwrap()),
        _ => panic!("Expected IndexOrigin::Flag, got {:?}", origin),
    }
    assert_eq!(index.releases[0].datasets[0].manifest_digest, "sha256:0000000000000000000000000000000000000000000000000000000000000003");
    Ok(())
}

#[test]
fn test_precedence_remote_wins_over_cache_and_baked() -> Result<()> {
    let tmp = TempDir::new()?;
    let workspace = tmp.path().join("ods_data");
    std::fs::create_dir_all(&workspace)?;

    // Cache index in workspace
    let cache_index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("1.0.0", "sha256:0000000000000000000000000000000000000000000000000000000000000001")],
    )]);
    std::fs::write(workspace.join("_releases.json"), cache_index.to_json_pretty()?)?;

    // Remote index served by fetcher
    let remote_index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("1.0.0", "sha256:0000000000000000000000000000000000000000000000000000000000000002")],
    )]);
    let fetcher = MockPrecedenceFetcher {
        raw_response: Some((remote_index.to_json_pretty()?.into_bytes(), "https://ods.fyi/v2/releases.json".to_string())),
    };

    let (index, origin) = resolve_index(
        &workspace,
        None,
        true,
        false,
        &fetcher,
    )?;

    match origin {
        IndexOrigin::Fetched(url) => assert_eq!(url, "https://ods.fyi/v2/releases.json"),
        _ => panic!("Expected IndexOrigin::Fetched, got {:?}", origin),
    }
    assert_eq!(index.releases[0].datasets[0].manifest_digest, "sha256:0000000000000000000000000000000000000000000000000000000000000002");
    Ok(())
}

#[test]
fn test_precedence_workspace_cache_wins_over_baked() -> Result<()> {
    let tmp = TempDir::new()?;
    let workspace = tmp.path().join("ods_data");
    std::fs::create_dir_all(&workspace)?;

    let cache_index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("1.0.0", "sha256:0000000000000000000000000000000000000000000000000000000000000001")],
    )]);
    let cache_path = workspace.join("_releases.json");
    std::fs::write(&cache_path, cache_index.to_json_pretty()?)?;

    let fetcher = MockPrecedenceFetcher { raw_response: None };

    let (index, origin) = resolve_index(
        &workspace,
        None,
        false,
        false,
        &fetcher,
    )?;

    match origin {
        IndexOrigin::WorkspaceCache(p) => assert_eq!(p, cache_path),
        _ => panic!("Expected IndexOrigin::WorkspaceCache, got {:?}", origin),
    }
    assert_eq!(index.releases[0].datasets[0].manifest_digest, "sha256:0000000000000000000000000000000000000000000000000000000000000001");
    Ok(())
}

// A workspace cache in the old format is refused by name, pointing at `ods pull`, which
// replaces it with the index built into ods.
#[test]
fn test_an_old_format_workspace_cache_is_refused_and_ods_pull_replaces_it() -> Result<()> {
    let tmp = TempDir::new()?;
    let workspace = tmp.path().join("ods_data");
    std::fs::create_dir_all(&workspace)?;
    let cache_path = workspace.join("_releases.json");
    std::fs::write(&cache_path, common::OLD_FORMAT_INDEX)?;

    let fetcher = MockPrecedenceFetcher { raw_response: None };
    let err = resolve_index(&workspace, None, false, false, &fetcher).unwrap_err();
    assert!(err.downcast_ref::<AlreadyReported>().is_some(), "{err}");

    let list = common::ods_cmd()
        .current_dir(tmp.path())
        .env("ODS_RELEASE_INDEX_URL", "http://127.0.0.1:9/unreachable.json")
        .args(["pull", "--list"])
        .output()?;
    let stderr = String::from_utf8_lossy(&list.stderr);
    println!("{stderr}");
    assert_eq!(list.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("✖ Cannot read ods_data/_releases.json: it's a release index in the old format\n  Refresh it: ods pull"), "{stderr}");

    let pull = common::ods_cmd()
        .current_dir(tmp.path())
        .env("ODS_RELEASE_INDEX_URL", "http://127.0.0.1:9/unreachable.json")
        .arg("pull")
        .output()?;
    println!("{}", String::from_utf8_lossy(&pull.stderr));
    assert_eq!(std::fs::read(&cache_path)?, common::baked_fixture_bytes(), "ods pull replaces the old cache");
    Ok(())
}

#[test]
fn test_precedence_baked_is_fallback() -> Result<()> {
    common::use_baked_fixture();
    let tmp = TempDir::new()?;
    let workspace = tmp.path().join("ods_data");
    std::fs::create_dir_all(&workspace)?;

    let fetcher = MockPrecedenceFetcher { raw_response: None };

    let (index, origin) = resolve_index(
        &workspace,
        None,
        false,
        false,
        &fetcher,
    )?;

    match origin {
        IndexOrigin::BuiltIn => {}
        _ => panic!("Expected IndexOrigin::BuiltIn, got {:?}", origin),
    }
    let baked = common::baked_fixture();
    assert_eq!(index.releases.len(), baked.releases.len());
    Ok(())
}

#[test]
fn test_invalid_index_flag_fails_and_names_file() -> Result<()> {
    let tmp = TempDir::new()?;
    let workspace = tmp.path().join("ods_data");
    std::fs::create_dir_all(&workspace)?;

    let invalid_path = tmp.path().join("broken_index.json");
    std::fs::write(&invalid_path, "{ \"not_valid_json\": }")?;

    let fetcher = MockPrecedenceFetcher { raw_response: None };

    let res = resolve_index(
        &workspace,
        Some(invalid_path.to_str().unwrap()),
        false,
        false,
        &fetcher,
    );

    assert!(res.is_err());
    let err = res.unwrap_err();
    assert!(
        err.downcast_ref::<AlreadyReported>().is_some() || err.to_string().contains("broken_index.json"),
        "Must return error after reporting invalid index"
    );
    Ok(())
}

#[test]
fn test_invalid_workspace_cache_warns_and_falls_back_to_baked() -> Result<()> {
    common::use_baked_fixture();
    let tmp = TempDir::new()?;
    let workspace = tmp.path().join("ods_data");
    std::fs::create_dir_all(&workspace)?;

    let invalid_cache = workspace.join("_releases.json");
    std::fs::write(&invalid_cache, "{ not json")?;

    let fetcher = MockPrecedenceFetcher { raw_response: None };

    let (index, origin) = resolve_index(
        &workspace,
        None,
        false,
        false,
        &fetcher,
    )?;

    match origin {
        IndexOrigin::BuiltIn => {}
        _ => panic!("Expected fallback to IndexOrigin::BuiltIn, got {:?}", origin),
    }
    let baked = common::baked_fixture();
    assert_eq!(index.releases.len(), baked.releases.len());
    Ok(())
}

#[test]
fn test_invalid_remote_fetch_warns_and_falls_back_to_cache() -> Result<()> {
    let tmp = TempDir::new()?;
    let workspace = tmp.path().join("ods_data");
    std::fs::create_dir_all(&workspace)?;

    let cache_index = make_v1_index(&[(
        "2026-07-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37983173,
        &[("1.0.0", "sha256:0000000000000000000000000000000000000000000000000000000000000001")],
    )]);
    let cache_path = workspace.join("_releases.json");
    std::fs::write(&cache_path, cache_index.to_json_pretty()?)?;

    let fetcher = MockPrecedenceFetcher {
        raw_response: Some((b"not valid json".to_vec(), "https://ods.fyi/v2/releases.json".to_string())),
    };

    let (index, origin) = resolve_index(
        &workspace,
        None,
        true,
        false,
        &fetcher,
    )?;

    match origin {
        IndexOrigin::WorkspaceCache(p) => assert_eq!(p, cache_path),
        _ => panic!("Expected fallback to IndexOrigin::WorkspaceCache, got {:?}", origin),
    }
    assert_eq!(index.releases[0].datasets[0].manifest_digest, "sha256:0000000000000000000000000000000000000000000000000000000000000001");
    Ok(())
}

#[test]
fn test_index_with_multiple_fingerprints_roundtrips_in_order() -> Result<()> {
    let json = r#"{
  "$schema": "https://ods.fyi/schema/releases.v1.json",
  "name": "ods-data",
  "source": {
    "title": "NHS Organisation Data Service XML Data",
    "path": "https://isd.digital.nhs.uk/trud/users/guest/filters/0/categories/5/items/341/releases",
    "signing_key_fingerprints": [
      "71ED5964BAE53E83556320A42BE59DADEE84BEB0",
      "0123456789ABCDEF0123456789ABCDEF01234567"
    ]
  },
  "mirrors": [],
  "releases": []
}"#;

    let index: OdsReleaseIndex = serde_json::from_str(json)?;
    index.validate()?;
    assert_eq!(
        index.source.signing_key_fingerprints,
        Some(vec![
            "71ED5964BAE53E83556320A42BE59DADEE84BEB0".to_string(),
            "0123456789ABCDEF0123456789ABCDEF01234567".to_string(),
        ])
    );

    let roundtrip = serde_json::to_string_pretty(&index)?;
    assert_eq!(roundtrip, json);
    Ok(())
}

#[test]
fn test_index_rejects_empty_fingerprints() {
    let json = r#"{
  "$schema": "https://ods.fyi/schema/releases.v1.json",
  "name": "ods-data",
  "source": {
    "title": "NHS Organisation Data Service XML Data",
    "path": "https://isd.digital.nhs.uk/trud/users/guest/filters/0/categories/5/items/341/releases",
    "signing_key_fingerprints": []
  },
  "mirrors": [],
  "releases": []
}"#;

    let index: OdsReleaseIndex = serde_json::from_str(json).unwrap();
    let err = index.validate().unwrap_err().to_string();
    assert!(
        err.contains("Invalid source.signing_key_fingerprints: expected at least one entry"),
        "Unexpected error: {}",
        err
    );
}

#[test]
fn test_index_rejects_duplicate_fingerprint() {
    let json = r#"{
  "$schema": "https://ods.fyi/schema/releases.v1.json",
  "name": "ods-data",
  "source": {
    "title": "NHS Organisation Data Service XML Data",
    "path": "https://isd.digital.nhs.uk/trud/users/guest/filters/0/categories/5/items/341/releases",
    "signing_key_fingerprints": [
      "71ED5964BAE53E83556320A42BE59DADEE84BEB0",
      "71ED5964BAE53E83556320A42BE59DADEE84BEB0"
    ]
  },
  "mirrors": [],
  "releases": []
}"#;

    let index: OdsReleaseIndex = serde_json::from_str(json).unwrap();
    let err = index.validate().unwrap_err().to_string();
    assert!(
        err.contains("Duplicate source.signing_key_fingerprints in index: '71ED5964BAE53E83556320A42BE59DADEE84BEB0'"),
        "Unexpected error: {}",
        err
    );
}

#[test]
fn test_index_rejects_invalid_fingerprint_length_or_charset() {
    // 39-character entry
    let json_39 = r#"{
  "$schema": "https://ods.fyi/schema/releases.v1.json",
  "name": "ods-data",
  "source": {
    "title": "NHS Organisation Data Service XML Data",
    "path": "https://isd.digital.nhs.uk/trud/users/guest/filters/0/categories/5/items/341/releases",
    "signing_key_fingerprints": [
      "71ED5964BAE53E83556320A42BE59DADEE84BEB"
    ]
  },
  "mirrors": [],
  "releases": []
}"#;

    let index_39: OdsReleaseIndex = serde_json::from_str(json_39).unwrap();
    let err_39 = index_39.validate().unwrap_err().to_string();
    assert!(
        err_39.contains("Invalid source.signing_key_fingerprints: expected 40 upper-case hex characters, got '71ED5964BAE53E83556320A42BE59DADEE84BEB'"),
        "Unexpected error: {}",
        err_39
    );

    // lowercase entry
    let json_lower = r#"{
  "$schema": "https://ods.fyi/schema/releases.v1.json",
  "name": "ods-data",
  "source": {
    "title": "NHS Organisation Data Service XML Data",
    "path": "https://isd.digital.nhs.uk/trud/users/guest/filters/0/categories/5/items/341/releases",
    "signing_key_fingerprints": [
      "71ed5964bae53e83556320a42be59dadee84beb0"
    ]
  },
  "mirrors": [],
  "releases": []
}"#;

    let index_lower: OdsReleaseIndex = serde_json::from_str(json_lower).unwrap();
    let err_lower = index_lower.validate().unwrap_err().to_string();
    assert!(
        err_lower.contains("Invalid source.signing_key_fingerprints: expected 40 upper-case hex characters, got '71ed5964bae53e83556320a42be59dadee84beb0'"),
        "Unexpected error: {}",
        err_lower
    );
}

#[test]
fn test_schema_rejects_an_old_format_index() -> Result<()> {
    let schema_str = include_str!("../../worker/schema/releases.v1.json");
    let schema_json: serde_json::Value = serde_json::from_str(schema_str)?;
    let validator = jsonschema::validator_for(&schema_json)
        .map_err(|e| anyhow::anyhow!("Invalid schema: {}", e))?;

    let old_json: serde_json::Value = serde_json::from_slice(common::OLD_FORMAT_INDEX)?;
    let errors: Vec<_> = validator.iter_errors(&old_json).collect();
    println!("Schema validation rejected the old format with {} errors:", errors.len());
    for err in &errors {
        println!("  - {}", err);
    }
    assert!(!errors.is_empty(), "Schema must reject an index in the old format");
    Ok(())
}

#[test]
fn test_merge_fingerprints_takes_candidate_when_non_empty() -> Result<()> {
    let base = common::baked_fixture();
    let mut candidate = common::baked_fixture();
    candidate.source.signing_key_fingerprints = Some(vec![
        "71ED5964BAE53E83556320A42BE59DADEE84BEB0".to_string(),
        "0123456789ABCDEF0123456789ABCDEF01234567".to_string(),
    ]);

    let merged = base.merge(&candidate)?;
    assert_eq!(
        merged.source.signing_key_fingerprints,
        Some(vec![
            "71ED5964BAE53E83556320A42BE59DADEE84BEB0".to_string(),
            "0123456789ABCDEF0123456789ABCDEF01234567".to_string(),
        ])
    );
    Ok(())
}


// A release the built-in index records only for its known source issue is refused as having no
// dataset yet, naming the issue and how to build it; a date with no row is refused as unknown.
// The built-in index is this test's own: the fixture's header and one row, for 2019-05-31.
#[test]
fn test_pull_of_a_release_with_no_dataset_says_so() -> Result<()> {
    let mut baked = common::baked_fixture();
    baked.releases.push(ods::index::Release {
        source: ods::index::SourceRelease {
            version: "2019-05-31".to_string(),
            hash: format!("sha256:{}", "2".repeat(64)),
            bytes: 39422486,
            issues: vec!["two-full-files-2019-05".to_string()],
        },
        datasets: Vec::new(),
    });
    baked.validate()?;
    let baked_dir = TempDir::new()?;
    let baked_file = baked_dir.path().join("baked.json");
    std::fs::write(&baked_file, baked.to_json_pretty()?)?;

    let pull = |date: &str| -> Result<std::process::Output> {
        let tmp = TempDir::new()?;
        Ok(common::ods_cmd()
            .current_dir(tmp.path())
            .env("ODS_TEST_BAKED_INDEX", &baked_file)
            .env("ODS_RELEASE_INDEX_URL", "http://127.0.0.1:9/unreachable.json")
            .args(["pull", date])
            .output()?)
    };

    let recorded = pull("2019-05-31")?;
    let stderr = String::from_utf8_lossy(&recorded.stderr);
    println!("{stderr}");
    assert_eq!(recorded.status.code(), Some(1), "{stderr}");
    assert_eq!(
        stderr.trim_end(),
        "✖ 2019-05-31 is in the release index, but no dataset has been published for it yet\n  \
         * known source issue: two-full-files-2019-05  https://github.com/olizilla/ods/blob/main/docs/source-issues/two-full-files-2019-05.md\n  \
         Build it yourself: ods trud pull 2019-05-31 && ods make"
    );

    let unknown = pull("2019-06-27")?;
    let stderr = String::from_utf8_lossy(&unknown.stderr);
    println!("{stderr}");
    assert_eq!(unknown.status.code(), Some(1), "{stderr}");
    assert_eq!(stderr.trim_end(), "✖ Release date '2019-06-27' is not known to the release index");
    Ok(())
}

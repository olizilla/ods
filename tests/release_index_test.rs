use anyhow::Result;
use ods::index::{OdsReleaseIndex, ReleaseIndexEntry};

#[test]
fn test_baked_index_parses_and_validates() -> Result<()> {
    let index = OdsReleaseIndex::baked()?;
    assert_eq!(index.index_version, 2);
    assert_eq!(index.type_tag, "ods_release_index");
    Ok(())
}

#[test]
fn test_index_rejects_duplicate_date_version_pair() {
    let json = r#"{
      "_type": "ods_release_index",
      "index_version": 2,
      "mirrors": [],
      "releases": [
        {
          "trud_release_date": "2026-07-31",
          "dataset_version": "1.0.0",
          "tag": "2026-07-31_1.0.0",
          "manifest_digest": "sha256:0f2a000000000000000000000000000000000000000000000000000000000000",
          "trud_release_sha256": "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
          "tool_version": "0.4.3"
        },
        {
          "trud_release_date": "2026-07-31",
          "dataset_version": "1.0.0",
          "tag": "2026-07-31_1.0.0",
          "manifest_digest": "sha256:7c4a000000000000000000000000000000000000000000000000000000000000",
          "trud_release_sha256": "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
          "tool_version": "0.4.3"
        }
      ]
    }"#;

    let index: OdsReleaseIndex = serde_json::from_str(json).unwrap();
    let result = index.validate();
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("Duplicate (date, version) pair"));
}

#[test]
fn test_index_rejects_uppercase_manifest_digest() {
    let json = r#"{
      "_type": "ods_release_index",
      "index_version": 2,
      "mirrors": [],
      "releases": [
        {
          "trud_release_date": "2026-07-31",
          "dataset_version": "1.0.0",
          "tag": "2026-07-31_1.0.0",
          "manifest_digest": "sha256:0F2A000000000000000000000000000000000000000000000000000000000000",
          "trud_release_sha256": "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
          "tool_version": "0.4.3"
        }
      ]
    }"#;

    let index: OdsReleaseIndex = serde_json::from_str(json).unwrap();
    let result = index.validate();
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("must be lowercase hex"));
}

#[test]
fn test_merge_allows_new_releases() -> Result<()> {
    let baked = OdsReleaseIndex {
        type_tag: "ods_release_index".to_string(),
        index_version: 2,
        concept_doi: None,
        mirrors: vec![],
        releases: vec![ReleaseIndexEntry {
            trud_release_date: "2019-11-29".to_string(),
            dataset_version: "1.0.0".to_string(),
            tag: "2019-11-29_1.0.0".to_string(),
            manifest_digest: "sha256:c1a8000000000000000000000000000000000000000000000000000000000000".to_string(),
            trud_release_sha256: "3B7F91A2".to_string(),
            tool_version: "0.4.2".to_string(),
            dataset_doi: None,
            withdrawn: None,
        }],
    };

    let fetched = OdsReleaseIndex {
        type_tag: "ods_release_index".to_string(),
        index_version: 2,
        concept_doi: None,
        mirrors: vec![],
        releases: vec![
            ReleaseIndexEntry {
                trud_release_date: "2019-11-29".to_string(),
                dataset_version: "1.0.0".to_string(),
                tag: "2019-11-29_1.0.0".to_string(),
                manifest_digest: "sha256:c1a8000000000000000000000000000000000000000000000000000000000000".to_string(),
                trud_release_sha256: "3B7F91A2".to_string(),
                tool_version: "0.4.2".to_string(),
                dataset_doi: None,
                withdrawn: None,
            },
            ReleaseIndexEntry {
                trud_release_date: "2026-07-31".to_string(),
                dataset_version: "1.0.1".to_string(),
                tag: "2026-07-31_1.0.1".to_string(),
                manifest_digest: "sha256:0f2a000000000000000000000000000000000000000000000000000000000000".to_string(),
                trud_release_sha256: "8151248D".to_string(),
                tool_version: "0.4.3".to_string(),
                dataset_doi: None,
                withdrawn: None,
            },
        ],
    };

    let merged = baked.merge(&fetched)?;
    assert_eq!(merged.releases.len(), 2);
    Ok(())
}

#[test]
fn test_merge_rejects_contradiction_on_baked_release() {
    let baked = OdsReleaseIndex {
        type_tag: "ods_release_index".to_string(),
        index_version: 2,
        concept_doi: None,
        mirrors: vec![],
        releases: vec![ReleaseIndexEntry {
            trud_release_date: "2026-07-31".to_string(),
            dataset_version: "1.0.0".to_string(),
            tag: "2026-07-31_1.0.0".to_string(),
            manifest_digest: "sha256:0f2a000000000000000000000000000000000000000000000000000000000000".to_string(),
            trud_release_sha256: "8151248D".to_string(),
            tool_version: "0.4.3".to_string(),
            dataset_doi: None,
            withdrawn: None,
        }],
    };

    let fetched = OdsReleaseIndex {
        type_tag: "ods_release_index".to_string(),
        index_version: 2,
        concept_doi: None,
        mirrors: vec![],
        releases: vec![ReleaseIndexEntry {
            trud_release_date: "2026-07-31".to_string(),
            dataset_version: "1.0.0".to_string(),
            tag: "2026-07-31_1.0.0".to_string(),
            manifest_digest: "sha256:ffff000000000000000000000000000000000000000000000000000000000000".to_string(),
            trud_release_sha256: "8151248D".to_string(),
            tool_version: "0.4.3".to_string(),
            dataset_doi: None,
            withdrawn: None,
        }],
    };

    let result = baked.merge(&fetched);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("Security error: fetched index contradicts baked release"));
}

#[test]
fn test_resolve_prefers_highest_non_withdrawn_semver() -> Result<()> {
    let index = OdsReleaseIndex {
        type_tag: "ods_release_index".to_string(),
        index_version: 2,
        concept_doi: None,
        mirrors: vec![],
        releases: vec![
            ReleaseIndexEntry {
                trud_release_date: "2026-07-31".to_string(),
                dataset_version: "1.0.0".to_string(),
                tag: "2026-07-31_1.0.0".to_string(),
                manifest_digest: "sha256:7c4a000000000000000000000000000000000000000000000000000000000000".to_string(),
                trud_release_sha256: "8151248D".to_string(),
                tool_version: "0.4.2".to_string(),
                dataset_doi: None,
                withdrawn: Some("roles table truncated".to_string()),
            },
            ReleaseIndexEntry {
                trud_release_date: "2026-07-31".to_string(),
                dataset_version: "1.0.1".to_string(),
                tag: "2026-07-31_1.0.1".to_string(),
                manifest_digest: "sha256:0f2a000000000000000000000000000000000000000000000000000000000000".to_string(),
                trud_release_sha256: "8151248D".to_string(),
                tool_version: "0.4.3".to_string(),
                dataset_doi: None,
                withdrawn: None,
            },
        ],
    };

    let resolved = index.resolve(Some("2026-07-31"))?;
    assert_eq!(resolved.dataset_version, "1.0.1");
    Ok(())
}

#[test]
fn test_resolve_refuses_when_all_versions_for_date_withdrawn() {
    let index = OdsReleaseIndex {
        type_tag: "ods_release_index".to_string(),
        index_version: 2,
        concept_doi: None,
        mirrors: vec![],
        releases: vec![ReleaseIndexEntry {
            trud_release_date: "2026-07-31".to_string(),
            dataset_version: "1.0.0".to_string(),
            tag: "2026-07-31_1.0.0".to_string(),
            manifest_digest: "sha256:7c4a000000000000000000000000000000000000000000000000000000000000".to_string(),
            trud_release_sha256: "8151248D".to_string(),
            tool_version: "0.4.2".to_string(),
            dataset_doi: None,
            withdrawn: Some("roles table truncated at 65535 rows by a bad build".to_string()),
        }],
    };

    let result = index.resolve(Some("2026-07-31"));
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("2026-07-31 has no valid release"));
    assert!(err.contains("1.0.0 was withdrawn: roles table truncated at 65535 rows by a bad build"));
}

#[test]
fn test_merge_propagates_withdrawal_to_baked_release() -> Result<()> {
    let baked = OdsReleaseIndex {
        type_tag: "ods_release_index".to_string(),
        index_version: 2,
        concept_doi: None,
        mirrors: vec![],
        releases: vec![ReleaseIndexEntry {
            trud_release_date: "2026-07-31".to_string(),
            dataset_version: "1.0.0".to_string(),
            tag: "2026-07-31_1.0.0".to_string(),
            manifest_digest: "sha256:0f2a000000000000000000000000000000000000000000000000000000000000".to_string(),
            trud_release_sha256: "8151248D".to_string(),
            tool_version: "0.4.3".to_string(),
            dataset_doi: None,
            withdrawn: None,
        }],
    };

    let fetched = OdsReleaseIndex {
        type_tag: "ods_release_index".to_string(),
        index_version: 2,
        concept_doi: None,
        mirrors: vec![],
        releases: vec![ReleaseIndexEntry {
            trud_release_date: "2026-07-31".to_string(),
            dataset_version: "1.0.0".to_string(),
            tag: "2026-07-31_1.0.0".to_string(),
            manifest_digest: "sha256:0f2a000000000000000000000000000000000000000000000000000000000000".to_string(),
            trud_release_sha256: "8151248D".to_string(),
            tool_version: "0.4.3".to_string(),
            dataset_doi: None,
            withdrawn: Some("roles table truncated at 65535 rows by a bad build".to_string()),
        }],
    };

    let merged = baked.merge(&fetched)?;
    assert_eq!(merged.releases.len(), 1);
    assert_eq!(
        merged.releases[0].withdrawn.as_deref(),
        Some("roles table truncated at 65535 rows by a bad build")
    );
    Ok(())
}

#[test]
fn test_resolve_none_refuses_when_newest_date_is_withdrawn() {
    let index = OdsReleaseIndex {
        type_tag: "ods_release_index".to_string(),
        index_version: 2,
        concept_doi: None,
        mirrors: vec![],
        releases: vec![
            ReleaseIndexEntry {
                trud_release_date: "2026-05-29".to_string(),
                dataset_version: "1.0.0".to_string(),
                tag: "2026-05-29_1.0.0".to_string(),
                manifest_digest: "sha256:1111000000000000000000000000000000000000000000000000000000000000".to_string(),
                trud_release_sha256: "AAAA".to_string(),
                tool_version: "0.4.2".to_string(),
                dataset_doi: None,
                withdrawn: None,
            },
            ReleaseIndexEntry {
                trud_release_date: "2026-07-31".to_string(),
                dataset_version: "1.0.0".to_string(),
                tag: "2026-07-31_1.0.0".to_string(),
                manifest_digest: "sha256:2222000000000000000000000000000000000000000000000000000000000000".to_string(),
                trud_release_sha256: "BBBB".to_string(),
                tool_version: "0.4.3".to_string(),
                dataset_doi: None,
                withdrawn: Some("critical corruption in roles".to_string()),
            },
        ],
    };

    let result = index.resolve(None);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("2026-07-31 has no valid release"));
    assert!(err.contains("critical corruption in roles"));
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
    ods::index::OdsReleaseIndex::save_to_workspace_bytes(ods::index::BAKED_RELEASES_JSON_BYTES, tmp.path())?;

    let loaded = ods::index::OdsReleaseIndex::load_from_workspace(tmp.path())?.unwrap();
    assert_eq!(loaded.index_version, baked.index_version);
    assert_eq!(loaded.type_tag, "ods_release_index");
    Ok(())
}



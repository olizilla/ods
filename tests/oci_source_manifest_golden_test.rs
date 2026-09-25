//! R10: a source bundle's packing rules are frozen with the first push. This pins the exact
//! manifest bytes for a fixture bundle, so a change to a media type, an annotation, the ordering
//! or the config would show up here instead of as a second digest for an archive already pushed.

mod common;

use ods::oci::source;
use std::fs;
use tempfile::TempDir;

const GOLDEN: &str = "tests/fixtures/oci_source_manifest_golden.json";

#[test]
fn test_source_manifest_matches_the_golden_bytes() {
    let tmp = TempDir::new().unwrap();
    let release = common::create_source_release(tmp.path(), "2026-07-31");

    let bundle = source::pack(&release).expect("the fixture bundle packs");

    if std::env::var("UPDATE_EXPECT").is_ok() {
        fs::write(GOLDEN, &bundle.manifest_bytes).unwrap();
    }
    let golden = fs::read(GOLDEN).expect("tests/fixtures/oci_source_manifest_golden.json must exist");
    assert_eq!(
        String::from_utf8_lossy(&bundle.manifest_bytes),
        String::from_utf8_lossy(&golden),
        "the source manifest changed: that would give a second digest for an archive already pushed"
    );

    // The rules the bytes stand for, stated
    let manifest: serde_json::Value = serde_json::from_slice(&golden).unwrap();
    assert_eq!(manifest["artifactType"], "application/vnd.fyi.ods.source.v1");
    assert_eq!(manifest["config"]["mediaType"], "application/vnd.oci.empty.v1+json");
    assert_eq!(manifest["config"]["size"], 2);
    let annotations: Vec<&str> = manifest["annotations"].as_object().unwrap().keys().map(|k| k.as_str()).collect();
    assert_eq!(
        annotations,
        vec![
            "fyi.ods.attribution",
            "fyi.ods.trud-release-date",
            "fyi.ods.trud-release-sha256",
            "org.opencontainers.image.licenses",
        ],
        "only NHS's facts and terms are annotated: no clock value, nothing about the tool"
    );
    let titles: Vec<&str> = manifest["layers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["annotations"]["org.opencontainers.image.title"].as_str().unwrap())
        .collect();
    let mut sorted = titles.clone();
    sorted.sort();
    assert_eq!(titles, sorted, "layers are sorted by title");
    assert_eq!(titles.len(), 4);
}

// The digest is a function of the four files, wherever they sit.
#[test]
fn test_the_same_release_packs_to_the_same_digest_from_two_directories() {
    let (first, second) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let a = source::pack(&common::create_source_release(first.path(), "2026-07-31")).unwrap();
    let b = source::pack(&common::create_source_release(&second.path().join("elsewhere"), "2026-07-31")).unwrap();

    println!("{}\n{}", a.manifest_digest, b.manifest_digest);
    assert_eq!(a.manifest_digest, b.manifest_digest);
    assert_eq!(a.manifest_bytes, b.manifest_bytes);
}

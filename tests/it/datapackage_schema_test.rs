use crate::common;

use anyhow::Result;

fn validator() -> jsonschema::Validator {
    let schema_str = include_str!("../../worker/schema/datapackage.v1.json");
    let schema_json: serde_json::Value = serde_json::from_str(schema_str).unwrap();
    jsonschema::validator_for(&schema_json).expect("schema itself must be valid draft-07")
}

/// The `datapackage.json` view of a release whose four Parquet files carry the 2026-09-25
/// provenance: the files `ods make` writes, with stub tables.
fn sample_view() -> serde_json::Value {
    let tmp = tempfile::tempdir().unwrap();
    for f in ["orgs", "roles", "relationships", "successions"] {
        common::write_fixture_parquet(
            &tmp.path().join(format!("{f}.parquet")),
            "2026-09-25",
            "CA0FEE7512F593ADA1FA9B95BF1372B41911167DA463A98FECF33ADFD86697E5",
            ods::datapackage::DATASET_VERSION,
            f,
        );
    }
    ods::datapackage::generate_view(tmp.path(), None).unwrap()
}

#[test]
fn test_built_datapackage_matches_its_schema() -> Result<()> {
    let validator = validator();
    let pkg = sample_view();
    let errors: Vec<_> = validator.iter_errors(&pkg).collect();
    println!("built datapackage.json validated against worker/schema/datapackage.v1.json: {} errors", errors.len());
    assert!(errors.is_empty(), "datapackage.json schema errors: {:?}", errors);
    Ok(())
}

#[test]
fn test_datapackage_schema_rejects_wrong_bytes_type() -> Result<()> {
    let validator = validator();
    let mut pkg = sample_view();
    pkg["sources"][0]["bytes"] = serde_json::json!("big");
    let errors: Vec<_> = validator.iter_errors(&pkg).collect();
    println!(
        "descriptor with sources[0].bytes = \"big\": {} errors: {:?}",
        errors.len(),
        errors.iter().map(|e| e.to_string()).collect::<Vec<_>>()
    );
    assert!(!errors.is_empty(), "expected the wrong bytes type to be rejected");
    Ok(())
}

#[test]
fn test_pull_record_matches_the_same_schema() -> Result<()> {
    let validator = validator();
    let tmp = tempfile::tempdir().unwrap();
    let release = common::create_source_release(tmp.path(), "2026-09-25");
    let record: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(ods::provenance::pull_record_path(&release))?)?;
    assert_eq!(record["resources"].as_array().map(|r| r.len()), Some(4), "archive, checksum, signature, key");
    let errors: Vec<_> = validator.iter_errors(&record).collect();
    println!("trud/datapackage.json validated against worker/schema/datapackage.v1.json: {} errors", errors.len());
    assert!(errors.is_empty(), "trud/datapackage.json schema errors: {:?}", errors);
    Ok(())
}

use crate::common;

use anyhow::Result;

fn validator() -> jsonschema::Validator {
    let schema_str = include_str!("../../worker/schema/ods-datapackage.v1.json");
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
    println!("built datapackage.json validated against worker/schema/ods-datapackage.v1.json: {} errors", errors.len());
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
    println!("trud/datapackage.json validated against worker/schema/ods-datapackage.v1.json: {} errors", errors.len());
    assert!(errors.is_empty(), "trud/datapackage.json schema errors: {:?}", errors);
    Ok(())
}

/// The view of files without provenance (a `--force` build) claims nothing about their source or
/// terms: only `$schema`, a `description` and the `resources`, which still validates, since
/// Data Package v2 requires only `resources`.
#[test]
fn test_view_without_provenance_names_only_the_files_and_validates() -> Result<()> {
    let tmp = tempfile::tempdir().unwrap();
    for f in ["orgs", "roles", "relationships", "successions"] {
        ods::commands::parquet::write_stub_parquet(&tmp.path().join(format!("{f}.parquet")), None, f)?;
    }
    let view = ods::datapackage::generate_view(tmp.path(), None)?;
    let keys: Vec<&str> = view.as_object().unwrap().keys().map(|k| k.as_str()).collect();
    assert_eq!(keys, ["$schema", "description", "resources"], "{view}");
    assert_eq!(view["resources"].as_array().map(|r| r.len()), Some(4));
    let errors: Vec<_> = validator().iter_errors(&view).map(|e| e.to_string()).collect();
    assert!(errors.is_empty(), "{errors:?}");
    Ok(())
}

/// The four list columns are Table Schema `list` fields with `itemType: string`, which our profile
/// accepts ahead of Data Package 2.0.1, while a field type nobody defined still fails.
#[test]
fn test_list_fields_validate_and_a_nonsense_type_fails() -> Result<()> {
    let validator = validator();
    let pkg = sample_view();
    let orgs = pkg["resources"].as_array().unwrap().iter().find(|r| r["name"] == "orgs").unwrap();
    let lists: Vec<(String, String)> = orgs["schema"]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["type"] == "list")
        .map(|f| (f["name"].as_str().unwrap().to_string(), f["itemType"].as_str().unwrap().to_string()))
        .collect();
    let string = || "string".to_string();
    assert_eq!(
        lists,
        [
            ("role_codes".to_string(), string()),
            ("role_names".to_string(), string()),
            ("predecessor_codes".to_string(), string()),
            ("successor_codes".to_string(), string()),
        ]
    );
    assert_eq!(validator.iter_errors(&pkg).count(), 0);

    let mut nonsense = pkg.clone();
    let fields = nonsense["resources"][0]["schema"]["fields"].as_array_mut().unwrap();
    let role_codes = fields.iter_mut().find(|f| f["name"] == "role_codes").unwrap();
    role_codes["type"] = serde_json::json!("nonsense");
    let errors: Vec<String> = validator.iter_errors(&nonsense).map(|e| e.to_string()).collect();
    println!("role_codes of type \"nonsense\": {} errors: {:?}", errors.len(), errors);
    assert!(!errors.is_empty(), "a field type nobody defined must fail");

    // An `itemType` outside the List Field's enum fails too.
    let mut bad_item = pkg.clone();
    let fields = bad_item["resources"][0]["schema"]["fields"].as_array_mut().unwrap();
    fields.iter_mut().find(|f| f["name"] == "role_codes").unwrap()["itemType"] = serde_json::json!("object");
    assert!(validator.iter_errors(&bad_item).count() > 0, "itemType object isn't a List Field item type");
    Ok(())
}

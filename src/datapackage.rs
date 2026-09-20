use arrow::datatypes::{DataType, Field, Schema};
use serde_json::{json, Value};

pub const DATAPACKAGE_FILENAME: &str = "datapackage.json";
pub const DATAPACKAGE_JSON: &str = include_str!("../data/datapackage.json");

pub fn dataset_version() -> &'static str {
    "0.1.0"
}

fn arrow_type_to_table_schema_type(dt: &DataType) -> &'static str {
    match dt {
        DataType::Utf8 => "string",
        DataType::Date32 => "date",
        DataType::Boolean => "boolean",
        DataType::List(_) => "array",
        _ => "string",
    }
}

fn field_to_json(field: &Field) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("name".to_string(), json!(field.name()));
    obj.insert("type".to_string(), json!(arrow_type_to_table_schema_type(field.data_type())));

    if !field.is_nullable() {
        obj.insert("constraints".to_string(), json!({ "required": true }));
    }

    Value::Object(obj)
}

fn schema_to_table_schema(
    schema: &Schema,
    primary_key: &str,
    foreign_keys: Vec<Value>,
) -> Value {
    let fields: Vec<Value> = schema.fields().iter().map(|f| field_to_json(f)).collect();
    let mut schema_obj = serde_json::Map::new();
    schema_obj.insert("fields".to_string(), json!(fields));
    schema_obj.insert("primaryKey".to_string(), json!([primary_key]));
    if !foreign_keys.is_empty() {
        schema_obj.insert("foreignKeys".to_string(), json!(foreign_keys));
    }
    Value::Object(schema_obj)
}

pub fn generate_datapackage() -> Value {
    let orgs_schema = crate::commands::parquet::orgs_schema();
    let roles_schema = crate::commands::parquet::roles_schema();
    let relationships_schema = crate::commands::parquet::relationships_schema();
    let successions_schema = crate::commands::parquet::successions_schema();

    let roles_fk = vec![json!({
        "fields": ["ods_code"],
        "reference": {
            "resource": "orgs_all",
            "fields": ["ods_code"]
        }
    })];

    let relationships_fk = vec![
        json!({
            "fields": ["source_code"],
            "reference": {
                "resource": "orgs_all",
                "fields": ["ods_code"]
            }
        }),
        json!({
            "fields": ["target_code"],
            "reference": {
                "resource": "orgs_all",
                "fields": ["ods_code"]
            }
        }),
    ];

    let successions_fk = vec![
        json!({
            "fields": ["predecessor_code"],
            "reference": {
                "resource": "orgs_all",
                "fields": ["ods_code"]
            }
        }),
        json!({
            "fields": ["successor_code"],
            "reference": {
                "resource": "orgs_all",
                "fields": ["ods_code"]
            }
        }),
    ];

    let resources = vec![
        json!({
            "name": "orgs",
            "type": "table",
            "path": "orgs.parquet",
            "format": "parquet",
            "mediatype": "application/vnd.apache.parquet",
            "schema": schema_to_table_schema(&orgs_schema, "ods_code", vec![])
        }),
        json!({
            "name": "orgs_all",
            "type": "table",
            "path": "orgs_all.parquet",
            "format": "parquet",
            "mediatype": "application/vnd.apache.parquet",
            "schema": schema_to_table_schema(&orgs_schema, "ods_code", vec![])
        }),
        json!({
            "name": "roles",
            "type": "table",
            "path": "roles.parquet",
            "format": "parquet",
            "mediatype": "application/vnd.apache.parquet",
            "schema": schema_to_table_schema(&roles_schema, "role_id", roles_fk)
        }),
        json!({
            "name": "relationships",
            "type": "table",
            "path": "relationships.parquet",
            "format": "parquet",
            "mediatype": "application/vnd.apache.parquet",
            "schema": schema_to_table_schema(&relationships_schema, "rel_id", relationships_fk)
        }),
        json!({
            "name": "successions",
            "type": "table",
            "path": "successions.parquet",
            "format": "parquet",
            "mediatype": "application/vnd.apache.parquet",
            "schema": schema_to_table_schema(&successions_schema, "succession_id", successions_fk)
        }),
    ];

    json!({
        "$schema": "https://datapackage.org/profiles/2.0/datapackage.json",
        "name": "ods-fyi",
        "title": "ods: NHS Organisation Data as verifiable Parquet files",
        "description": "All the organisations and sites in the NHS Organisation Data Service, as queryable & verifiable Parquet files. Deterministic projections of NHS England's ODS XML release on NHS TRUD, published by ods.fyi.",
        "version": dataset_version(),
        "licenses": [
            {
                "name": "OGL-UK-3.0",
                "path": "https://www.nationalarchives.gov.uk/doc/open-government-licence/version/3/",
                "title": "Open Government Licence v3.0",
                "attribution": "Contains information from NHS England, licensed under the current version of the Open Government Licence."
            }
        ],
        "sources": [
            {
                "title": "NHS Organisation Data Service XML Data, NHS England",
                "path": "https://isd.digital.nhs.uk/trud"
            }
        ],
        "resources": resources
    })
}

/// Generates the enriched per-release `datapackage.json` containing the schema contract
/// plus this release's facts: `id` (DOI if set), `version`, and per-resource `bytes`
/// and `hash` (SHA-256).
pub fn generate_release_datapackage(
    output_dir: &std::path::Path,
    dataset_doi: Option<&str>,
    dataset_version_opt: Option<&str>,
) -> Value {
    let mut pkg: Value = serde_json::from_str(DATAPACKAGE_JSON).unwrap_or_else(|_| generate_datapackage());

    if let Some(doi) = dataset_doi {
        pkg["id"] = json!(doi);
    } else if let Some(obj) = pkg.as_object_mut() {
        obj.remove("id");
    }

    let ver = dataset_version_opt
        .map(|s| s.to_string())
        .unwrap_or_else(|| dataset_version().to_string());
    pkg["version"] = json!(ver);

    if let Some(resources) = pkg["resources"].as_array_mut() {
        for res in resources.iter_mut() {
            if let Some(path_str) = res["path"].as_str() {
                let file_path = output_dir.join(path_str);
                if file_path.exists() {
                    if let Ok(meta) = std::fs::metadata(&file_path) {
                        res["bytes"] = json!(meta.len());
                    }
                    if let Ok(hash) = crate::provenance::compute_file_sha256(&file_path) {
                        res["hash"] = json!(format!("sha256:{}", hash.to_lowercase()));
                    }
                }
            }
        }
    }

    pkg
}

/// Reads the `version` field from `datapackage.json` in a release directory.
/// Returns `None` if the file is absent, unreadable, or lacks a string `version`.
pub fn read_dataset_version_from_dir(release_dir: &std::path::Path) -> Option<String> {
    let dp_path = release_dir.join("datapackage.json");
    if dp_path.exists() {
        if let Ok(bytes) = std::fs::read(&dp_path) {
            if let Ok(val) = serde_json::from_slice::<Value>(&bytes) {
                if let Some(v) = val.get("version").and_then(|v| v.as_str()) {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_dataset_version_from_descriptor() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(read_dataset_version_from_dir(tmp.path()), None);

        let dp_path = tmp.path().join("datapackage.json");
        std::fs::write(&dp_path, r#"{"version": "1.2.3"}"#).unwrap();
        assert_eq!(read_dataset_version_from_dir(tmp.path()), Some("1.2.3".to_string()));

        std::fs::write(&dp_path, r#"{"name": "test"}"#).unwrap();
        assert_eq!(read_dataset_version_from_dir(tmp.path()), None);
    }
}


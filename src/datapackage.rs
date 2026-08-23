use arrow::datatypes::{DataType, Field, Schema};
use serde_json::{json, Value};

pub const DATAPACKAGE_JSON: &str = include_str!("../data/datapackage.json");

pub fn schema_version() -> &'static str {
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
    schema_obj.insert("primaryKey".to_string(), json!(primary_key));
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
        "fields": "ods_code",
        "reference": {
            "resource": "orgs_all",
            "fields": "ods_code"
        }
    })];

    let relationships_fk = vec![
        json!({
            "fields": "source_code",
            "reference": {
                "resource": "orgs_all",
                "fields": "ods_code"
            }
        }),
        json!({
            "fields": "target_code",
            "reference": {
                "resource": "orgs_all",
                "fields": "ods_code"
            }
        }),
    ];

    let successions_fk = vec![
        json!({
            "fields": "predecessor_code",
            "reference": {
                "resource": "orgs_all",
                "fields": "ods_code"
            }
        }),
        json!({
            "fields": "successor_code",
            "reference": {
                "resource": "orgs_all",
                "fields": "ods_code"
            }
        }),
    ];

    let resources = vec![
        json!({
            "name": "orgs",
            "path": "orgs.parquet",
            "format": "parquet",
            "mediatype": "application/vnd.apache.parquet",
            "schema": schema_to_table_schema(&orgs_schema, "ods_code", vec![])
        }),
        json!({
            "name": "orgs_all",
            "path": "orgs_all.parquet",
            "format": "parquet",
            "mediatype": "application/vnd.apache.parquet",
            "schema": schema_to_table_schema(&orgs_schema, "ods_code", vec![])
        }),
        json!({
            "name": "roles",
            "path": "roles.parquet",
            "format": "parquet",
            "mediatype": "application/vnd.apache.parquet",
            "schema": schema_to_table_schema(&roles_schema, "role_id", roles_fk)
        }),
        json!({
            "name": "relationships",
            "path": "relationships.parquet",
            "format": "parquet",
            "mediatype": "application/vnd.apache.parquet",
            "schema": schema_to_table_schema(&relationships_schema, "rel_id", relationships_fk)
        }),
        json!({
            "name": "successions",
            "path": "successions.parquet",
            "format": "parquet",
            "mediatype": "application/vnd.apache.parquet",
            "schema": schema_to_table_schema(&successions_schema, "succession_id", successions_fk)
        }),
    ];

    json!({
        "name": "nhs-ods-parquet",
        "title": "NHS Organisation Data Service (ODS) Parquet Dataset",
        "description": "Columnar Parquet dataset compiled from the official NHS TRUD Organisation Data Service (ODS) release",
        "version": schema_version(),
        "licenses": [
            {
                "name": "OGL-UK-3.0",
                "path": "https://www.nationalarchives.gov.uk/doc/open-government-licence/version/3/",
                "title": "Open Government Licence v3.0",
                "attribution": "Contains Open Data and public sector information licensed under the Open Government Licence v3.0 from NHS England."
            }
        ],
        "resources": resources
    })
}

/// Generates the enriched per-release `datapackage.json` containing the schema contract
/// plus this release's facts: `id` (DOI if set), `sources` (TRUD release metadata),
/// and per-resource `bytes` and `hash` (SHA-256).
pub fn generate_release_datapackage(
    output_dir: &std::path::Path,
    provenance: Option<&crate::provenance::OdsProvenance>,
) -> Value {
    let mut pkg: Value = serde_json::from_str(DATAPACKAGE_JSON).unwrap_or_else(|_| generate_datapackage());

    if let Some(prov) = provenance {
        if let Some(ref doi) = prov.dataset_doi {
            pkg["id"] = json!(doi);
        }

        let mut sources = Vec::new();
        let source_title = prov
            .trud_release_name
            .clone()
            .unwrap_or_else(|| "NHS TRUD ODS XML Organisation Data".to_string());
        let mut source_obj = serde_json::Map::new();
        source_obj.insert("title".to_string(), json!(source_title));
        if let Some(ref date) = prov.trud_release_date {
            source_obj.insert("version".to_string(), json!(date));
        }
        sources.push(Value::Object(source_obj));
        pkg["sources"] = Value::Array(sources);
    }

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


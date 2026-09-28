use arrow::datatypes::{DataType, Field, Schema};
use serde_json::{json, Value};

pub const DATAPACKAGE_FILENAME: &str = "datapackage.json";

/// The dataset version. This is the single source of truth for the dataset version.
/// `ods make` embeds it in every Parquet file, inside `version` (`<source release>_<dataset
/// version>`), so bumping it changes every Parquet file and every manifest digest built
/// afterwards.
pub const DATASET_VERSION: &str = "0.1.0";

/// Our Data Package v2 extension profile: extends the published profile with
/// `licenses[].attribution` and the Source properties `hash`, `bytes` and `_cache`.
pub const DATAPACKAGE_SCHEMA_V1_URL: &str = "https://ods.fyi/schema/datapackage.v1.json";

/// The dataset family's stable name. With `version` it is unique per release, and equals the
/// OCI `repository:tag`.
pub const NAME: &str = "ods-data";
const TITLE: &str = "ods: NHS Organisation Data as verifiable Parquet files";
const DESCRIPTION: &str = "All the organisations and sites in the NHS Organisation Data Service, as queryable & verifiable Parquet files. Deterministic projections of NHS England's ODS XML release on NHS TRUD, published by ods.fyi.";

/// The source's title in the schema contract: the pull record's `title` for the TRUD release.
const TRUD_SOURCE_TITLE: &str = crate::provenance::SOURCE_TITLE;

/// The base of a hash-stable URL for our own copy of the source zip: the `nhs-ods-xml` archive
/// image's blobs, keyed by the layer digest, which is the zip's own SHA-256 (`src/oci/source.rs`).
const NHS_ODS_XML_CACHE_BASE: &str = "https://ghcr.io/v2/olizilla/nhs-ods-xml/blobs/sha256:";

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

/// The four tables' resource entries: path, format, mediatype and Table Schema. Never `bytes`
/// or `hash`: those are release facts, filled in from the files.
fn resources_value() -> Vec<Value> {
    let orgs_schema = crate::commands::parquet::orgs_schema();
    let roles_schema = crate::commands::parquet::roles_schema();
    let relationships_schema = crate::commands::parquet::relationships_schema();
    let successions_schema = crate::commands::parquet::successions_schema();

    let roles_fk = vec![json!({
        "fields": ["ods_code"],
        "reference": {
            "resource": "orgs",
            "fields": ["ods_code"]
        }
    })];

    let relationships_fk = vec![
        json!({
            "fields": ["source_code"],
            "reference": {
                "resource": "orgs",
                "fields": ["ods_code"]
            }
        }),
        json!({
            "fields": ["target_code"],
            "reference": {
                "resource": "orgs",
                "fields": ["ods_code"]
            }
        }),
    ];

    let successions_fk = vec![
        json!({
            "fields": ["predecessor_code"],
            "reference": {
                "resource": "orgs",
                "fields": ["ods_code"]
            }
        }),
        json!({
            "fields": ["successor_code"],
            "reference": {
                "resource": "orgs",
                "fields": ["ods_code"]
            }
        }),
    ];

    vec![
        json!({
            "name": "orgs",
            "type": "table",
            "path": "orgs.parquet",
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
    ]
}

/// The schema contract only: table names, columns and types, with no per-release facts (no
/// `id`, no source provenance, no resource `bytes`/`hash`). This is what `data/datapackage.json`
/// pins (`docs/tests.md` D2/T2) — it was never a real per-release descriptor, just the fixed
/// shape every generated view shares, so it stays a plain function rather than becoming "a view
/// of a fixture release".
pub fn generate_datapackage() -> Value {
    json!({
        "$schema": DATAPACKAGE_SCHEMA_V1_URL,
        "name": NAME,
        "title": TITLE,
        "description": DESCRIPTION,
        "version": DATASET_VERSION,
        "licenses": [crate::terms::license()],
        "contributors": crate::terms::contributors(),
        "sources": [{
            "title": TRUD_SOURCE_TITLE,
            "path": crate::terms::LANDING_PAGE
        }],
        "resources": resources_value()
    })
}

/// The Data Package view of a release: the embedded object its files carry, plus `$schema`,
/// `id`, `title` and `description`, the source's `_cache`, and one resource per file with its
/// `bytes` and `hash` and its Table Schema from `ods`'s compiled schemas. Written beside the
/// files by `ods make`, `ods pull` and `ods make datapackage`; never packed, and `ods` never
/// reads a value from it. The same files and index give the same bytes.
///
/// `id` is the index row's `dataset_doi` when the row names this exact manifest and has one,
/// else the release's `oci` purl from the rebuilt manifest: the view isn't part of any digest,
/// so it may name it. A build without provenance has no manifest, so no `id`.
pub fn generate_view(
    release_dir: &std::path::Path,
    index: Option<&crate::index::OdsReleaseIndex>,
) -> anyhow::Result<Value> {
    use crate::provenance::ReleaseRecord;

    type Files = Vec<(String, u64, String)>;
    let record = crate::provenance::read_release(release_dir)?;
    let (embedded, id, files): (crate::provenance::Embedded, Option<String>, Files) = match record {
        ReleaseRecord::Provenanced(facts) => {
            let (manifest, _) = crate::oci::dataset::build(release_dir, &facts)?;
            let digest = manifest.digest()?;
            let doi = index.and_then(|index| {
                index
                    .releases
                    .iter()
                    .find(|r| r.trud_release_date == facts.release_date)
                    .and_then(|r| r.datasets.iter().find(|d| d.dataset_version == facts.dataset_version))
                    .filter(|d| d.manifest_digest == digest)
                    .and_then(|d| d.dataset_doi.clone())
            });
            let id = doi.unwrap_or_else(|| oci_purl(&digest));
            let files = manifest
                .layers
                .iter()
                .filter_map(|l| {
                    let title = l.annotations.as_ref()?.get(crate::oci::ANNOTATION_TITLE)?;
                    Some((title.clone(), l.size, l.digest.clone()))
                })
                .collect();
            (facts.embedded.clone(), Some(id), files)
        }
        ReleaseRecord::NoProvenance(embedded) => {
            let mut files = Vec::new();
            for name in crate::provenance::release_parquet_files(release_dir)? {
                let path = release_dir.join(&name);
                let bytes = std::fs::metadata(&path)?.len();
                let hash = crate::provenance::prefixed_sha256(&crate::provenance::compute_file_sha256(&path)?);
                files.push((name, bytes, hash));
            }
            (*embedded, None, files)
        }
        other => anyhow::bail!(
            "{}",
            crate::provenance::format_record_refusal(release_dir, &other).unwrap_or_default()
        ),
    };

    let mut obj = serde_json::Map::new();
    obj.insert("$schema".to_string(), json!(DATAPACKAGE_SCHEMA_V1_URL));
    if let Some(id) = id {
        obj.insert("id".to_string(), json!(id));
    }
    obj.insert("name".to_string(), json!(embedded.name));
    if let Some(ref version) = embedded.version {
        obj.insert("version".to_string(), json!(version));
    }
    obj.insert("title".to_string(), json!(TITLE));
    obj.insert("description".to_string(), json!(DESCRIPTION));
    obj.insert("licenses".to_string(), serde_json::to_value(&embedded.licenses)?);
    obj.insert("contributors".to_string(), serde_json::to_value(&embedded.contributors)?);
    if let Some(ref sources) = embedded.sources {
        let mut values = Vec::with_capacity(sources.len());
        for source in sources {
            let mut value = serde_json::to_value(source)?;
            value["_cache"] = json!([format!("{}{}", NHS_ODS_XML_CACHE_BASE, source.hash.trim_start_matches("sha256:"))]);
            values.push(value);
        }
        obj.insert("sources".to_string(), json!(values));
    }

    let mut resources = Vec::new();
    for mut res in resources_value() {
        let path = res["path"].as_str().unwrap_or_default().to_string();
        let Some((_, bytes, hash)) = files.iter().find(|(name, _, _)| *name == path) else {
            continue;
        };
        res["bytes"] = json!(bytes);
        res["hash"] = json!(hash);
        resources.push(res);
    }
    obj.insert("resources".to_string(), json!(resources));

    Ok(Value::Object(obj))
}

/// The view as written to disk: pretty JSON with a trailing newline.
pub fn view_json(
    release_dir: &std::path::Path,
    index: Option<&crate::index::OdsReleaseIndex>,
) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(&generate_view(release_dir, index)?)? + "\n")
}

/// Writes the view to `release_dir/datapackage.json`.
pub fn write_view(
    release_dir: &std::path::Path,
    index: Option<&crate::index::OdsReleaseIndex>,
) -> anyhow::Result<()> {
    let path = release_dir.join(DATAPACKAGE_FILENAME);
    let json = view_json(release_dir, index)?;
    std::fs::write(&path, json).map_err(|e| anyhow::anyhow!("writing {}: {}", path.display(), e))
}

/// The release's package URL: its manifest digest in the `ods-data` repository on ods.fyi.
pub fn oci_purl(manifest_digest: &str) -> String {
    format!(
        "pkg:oci/{}@sha256%3A{}?repository_url=ods.fyi%2F{}",
        NAME,
        manifest_digest.trim_start_matches("sha256:"),
        NAME
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_datapackage_has_no_per_release_facts() {
        let pkg = generate_datapackage();
        assert!(pkg.get("id").is_none());
        assert!(pkg["sources"][0].get("hash").is_none());
        assert!(pkg["resources"][0].get("bytes").is_none());
    }
}

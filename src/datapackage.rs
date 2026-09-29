use arrow::datatypes::{DataType, Field, Schema};
use serde_json::{json, Value};

pub const DATAPACKAGE_FILENAME: &str = "datapackage.json";

/// The dataset version. This is the single source of truth for the dataset version.
/// `ods make` embeds it in every Parquet file, inside `version` (`<source release>_<dataset
/// version>`), so bumping it changes every Parquet file and every manifest digest built
/// afterwards.
pub const DATASET_VERSION: &str = "0.2.0";

/// Our Data Package v2 extension profile: extends the published profile with
/// `licenses[].attribution` and the Source properties `hash`, `bytes` and `_cache`.
pub const ODS_DATAPACKAGE_SCHEMA_URL: &str = "https://ods.fyi/schema/ods-datapackage.v1.json";

/// The dataset family's stable name. With `version` it is unique per release, and equals the
/// OCI `repository:tag`.
pub const NAME: &str = "ods-data";
const TITLE: &str = "ods: NHS Organisation Data as verifiable Parquet files";
/// The dataset in one sentence: the first of `DESCRIPTION`, and the start of a dataset manifest's
/// `org.opencontainers.image.description`.
pub const SUMMARY: &str = "All the organisations and sites in the NHS Organisation Data Service, as queryable & verifiable Parquet files.";
const DESCRIPTION: &str = "All the organisations and sites in the NHS Organisation Data Service, as queryable & verifiable Parquet files. Deterministic projections of NHS England's ODS XML release on NHS TRUD, published by ods.fyi.";

/// Where people go to read about the dataset: the view's `homepage`.
pub const HOMEPAGE: &str = "https://ods.fyi";

/// A view of files that carry no provenance says only what it can: which files they are.
const NO_PROVENANCE_DESCRIPTION: &str = "Parquet files that carry no provenance: built from an archive ods couldn't match to a TRUD release, or by an older ods. Their source and terms are unknown.";

/// The source's title in the schema contract: the TRUD archive package's `title` for the TRUD release.
const TRUD_SOURCE_TITLE: &str = crate::provenance::SOURCE_TITLE;

/// Where our own packaging of the TRUD archive lives: the `nhs-ods-xml` repository on ghcr.io
/// (`src/oci/source.rs`). The repository is private, so it names the packaging and isn't a link
/// anyone can fetch. The last path segment is `SOURCE_NAME`.
const SOURCE_REGISTRY: &str = "ghcr.io";
const SOURCE_REPOSITORY: &str = "olizilla/nhs-ods-xml";

/// A hash-stable URL for our own copy of the source zip: the archive image's blob keyed by the
/// layer digest, which is the zip's own SHA-256.
fn source_cache_url(sha256: &str) -> String {
    format!(
        "https://{}/v2/{}/blobs/sha256:{}",
        SOURCE_REGISTRY,
        SOURCE_REPOSITORY,
        sha256.trim_start_matches("sha256:")
    )
}

/// The package URL of the TRUD archive's OCI bundle for a source release, by tag: the bundle's
/// digest needs NHS's checksum, signature and key, which a pulled release doesn't have, while
/// the tag is always the source version. Qualifiers in purl's sorted order, `/` encoded.
pub fn source_purl(source_version: &str) -> String {
    format!(
        "pkg:oci/{}?repository_url={}%2F{}&tag={}",
        crate::provenance::SOURCE_NAME,
        SOURCE_REGISTRY,
        SOURCE_REPOSITORY.replace('/', "%2F"),
        source_version
    )
}

fn arrow_type_to_table_schema_type(dt: &DataType) -> &'static str {
    match dt {
        DataType::Utf8 => "string",
        DataType::Date32 => "date",
        DataType::Boolean => "boolean",
        // Table Schema's List Field, ahead of Data Package 2.0.1: our profile adds it to the
        // published field types (worker/schema/ods-datapackage.v1.json, docs/datapackage.md).
        DataType::List(_) => "list",
        _ => "string",
    }
}

fn field_to_json(field: &Field) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("name".to_string(), json!(field.name()));
    obj.insert("type".to_string(), json!(arrow_type_to_table_schema_type(field.data_type())));
    if let DataType::List(item) = field.data_type() {
        obj.insert("itemType".to_string(), json!(arrow_type_to_table_schema_type(item.data_type())));
    }

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
/// `purl`, no source provenance, no resource `bytes`/`hash`). This is what `data/datapackage.json`
/// pins (`docs/tests.md` D2/T2) — it was never a real per-release descriptor, just the fixed
/// shape every generated view shares, so it stays a plain function rather than becoming "a view
/// of a fixture release".
pub fn generate_datapackage() -> Value {
    json!({
        "$schema": ODS_DATAPACKAGE_SCHEMA_URL,
        "name": NAME,
        "title": TITLE,
        "description": DESCRIPTION,
        "homepage": HOMEPAGE,
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
/// `purl`, `title`, `description` and `homepage`, each source's `purl` and `_cache`, and one
/// resource per file with its `bytes` and `hash` and its Table Schema from `ods`'s compiled
/// schemas. Written beside the files by `ods make`, `ods pull` and `ods make datapackage`; never
/// packed, and `ods` never reads a value from it. The same files give the same bytes.
///
/// `purl` is the release's `oci` purl from the manifest rebuilt from the files, on every route:
/// the view isn't part of any digest, so it may name it. The view writes no `id` and no DOI; a
/// DOI is a post-publish fact that lives in the release index.
///
/// Files without provenance (no `datapackage` key) get `$schema`, a `description` saying their
/// source and terms are unknown, and `resources`: the facts about the files, and nothing else.
pub fn generate_view(release_dir: &std::path::Path) -> anyhow::Result<Value> {
    use crate::provenance::ReleaseRecord;

    type Files = Vec<(String, u64, String)>;
    let record = crate::provenance::read_release(release_dir)?;
    let (embedded, purl, files): (Option<crate::provenance::Embedded>, Option<String>, Files) = match record {
        ReleaseRecord::Provenanced(facts) => {
            let (manifest, _) = crate::oci::dataset::build(release_dir, &facts)?;
            let purl = oci_purl(&manifest.digest()?);
            let files = manifest
                .layers
                .iter()
                .filter_map(|l| {
                    let title = l.annotations.as_ref()?.get(crate::oci::ANNOTATION_TITLE)?;
                    Some((title.clone(), l.size, l.digest.clone()))
                })
                .collect();
            (Some(facts.embedded.clone()), Some(purl), files)
        }
        ReleaseRecord::NoProvenance => {
            let mut files = Vec::new();
            for name in crate::provenance::release_parquet_files(release_dir)? {
                let path = release_dir.join(&name);
                let bytes = std::fs::metadata(&path)?.len();
                let hash = crate::provenance::prefixed_sha256(&crate::provenance::compute_file_sha256(&path)?);
                files.push((name, bytes, hash));
            }
            (None, None, files)
        }
        other => anyhow::bail!(
            "{}",
            crate::provenance::format_record_refusal(release_dir, &other).unwrap_or_default()
        ),
    };

    let mut obj = serde_json::Map::new();
    obj.insert("$schema".to_string(), json!(ODS_DATAPACKAGE_SCHEMA_URL));
    if let Some(purl) = purl {
        obj.insert("purl".to_string(), json!(purl));
    }
    match embedded {
        Some(embedded) => {
            obj.insert("name".to_string(), json!(embedded.name));
            obj.insert("version".to_string(), json!(embedded.version));
            obj.insert("title".to_string(), json!(TITLE));
            obj.insert("description".to_string(), json!(DESCRIPTION));
            obj.insert("homepage".to_string(), json!(HOMEPAGE));
            obj.insert("licenses".to_string(), serde_json::to_value(&embedded.licenses)?);
            obj.insert("contributors".to_string(), serde_json::to_value(&embedded.contributors)?);
            let mut values = Vec::with_capacity(embedded.sources.len());
            for source in &embedded.sources {
                // `purl` leads the source's keys: the embedded object's own keys follow it.
                let mut value = serde_json::Map::new();
                value.insert("purl".to_string(), json!(source_purl(&source.version)));
                let Value::Object(own) = serde_json::to_value(source)? else {
                    anyhow::bail!("a source serialises as a JSON object");
                };
                value.extend(own);
                value.insert(
                    "_cache".to_string(),
                    json!([source_cache_url(&source.hash)]),
                );
                values.push(Value::Object(value));
            }
            obj.insert("sources".to_string(), json!(values));
        }
        None => {
            obj.insert("description".to_string(), json!(NO_PROVENANCE_DESCRIPTION));
        }
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
pub fn view_json(release_dir: &std::path::Path) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(&generate_view(release_dir)?)? + "\n")
}

/// Writes the view to `release_dir/datapackage.json`.
pub fn write_view(release_dir: &std::path::Path) -> anyhow::Result<()> {
    let path = release_dir.join(DATAPACKAGE_FILENAME);
    let json = view_json(release_dir)?;
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
    fn test_description_starts_with_the_summary() {
        assert!(DESCRIPTION.starts_with(SUMMARY));
    }

    #[test]
    fn test_generate_datapackage_has_no_per_release_facts() {
        let pkg = generate_datapackage();
        assert!(pkg.get("id").is_none());
        assert!(pkg.get("purl").is_none());
        assert!(pkg["sources"][0].get("purl").is_none());
        assert!(pkg["sources"][0].get("hash").is_none());
        assert!(pkg["resources"][0].get("bytes").is_none());
    }

    #[test]
    fn test_source_purl_names_the_archive_bundle_by_tag() {
        assert_eq!(
            source_purl("2026-09-25"),
            "pkg:oci/nhs-ods-xml?repository_url=ghcr.io%2Folizilla%2Fnhs-ods-xml&tag=2026-09-25"
        );
    }

    #[test]
    fn test_the_source_purl_and_cache_url_name_the_same_repository() {
        assert!(SOURCE_REPOSITORY.ends_with(&format!("/{}", crate::provenance::SOURCE_NAME)));
        assert_eq!(
            source_cache_url("sha256:abc"),
            "https://ghcr.io/v2/olizilla/nhs-ods-xml/blobs/sha256:abc"
        );
        assert!(source_purl("2026-09-25").contains("repository_url=ghcr.io%2Folizilla%2Fnhs-ods-xml&"));
    }
}

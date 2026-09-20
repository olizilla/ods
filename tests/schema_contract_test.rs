use anyhow::Result;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::Path;

fn extract_schema_subtree(pkg: &Value) -> Value {
    let resources = pkg["resources"].as_array().expect("resources array");
    let mapped_resources: Vec<Value> = resources
        .iter()
        .map(|r| {
            serde_json::json!({
                "name": r["name"],
                "type": r["type"],
                "path": r["path"],
                "format": r["format"],
                "mediatype": r["mediatype"],
                "schema": r["schema"]
            })
        })
        .collect();
    serde_json::json!(mapped_resources)
}

#[test]
fn test_datapackage_matches_committed_snapshot() -> Result<()> {
    let generated = ods::datapackage::generate_datapackage();
    let data_pkg_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/datapackage.json");

    if std::env::var("UPDATE_SCHEMA").is_ok() {
        let formatted = serde_json::to_string_pretty(&generated)? + "\n";
        fs::write(&data_pkg_path, formatted)?;
        eprintln!("✓ Updated data/datapackage.json from Arrow schemas");
        return Ok(());
    }

    let committed_str = fs::read_to_string(&data_pkg_path)
        .expect("data/datapackage.json must exist in repo");
    let committed: Value = serde_json::from_str(&committed_str)?;

    let gen_subtree = extract_schema_subtree(&generated);
    let com_subtree = extract_schema_subtree(&committed);

    if gen_subtree != com_subtree {
        let gen_pretty = serde_json::to_string_pretty(&gen_subtree)?;
        let com_pretty = serde_json::to_string_pretty(&com_subtree)?;
        panic!(
            "Schema differs from committed data/datapackage.json!\n\nGenerated:\n{}\n\nCommitted:\n{}\n\nRun `UPDATE_SCHEMA=1 cargo test` to regenerate, review the diff, and decide whether `version` needs a minor or major bump.",
            gen_pretty, com_pretty
        );
    }

    assert_eq!(generated["$schema"], committed["$schema"]);
    assert_eq!(generated["version"], committed["version"], "schema version in code must match committed package version");
    assert_eq!(generated["name"], committed["name"]);
    assert_eq!(generated["licenses"], committed["licenses"]);

    Ok(())
}

fn extract_backtick_columns(cell: &str) -> Vec<String> {
    let mut cols = Vec::new();
    let mut rest = cell;
    while let Some(start) = rest.find('`') {
        let after_start = &rest[start + 1..];
        if let Some(end) = after_start.find('`') {
            let col = &after_start[..end];
            if col.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') && !col.is_empty() {
                cols.push(col.to_string());
            }
            rest = &after_start[end + 1..];
        } else {
            break;
        }
    }
    cols
}

fn parse_docs_parquet_md_columns(content: &str) -> HashMap<String, BTreeSet<String>> {
    let mut result: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut current_table: Option<String> = None;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("## `orgs.parquet`") {
            current_table = Some("orgs".to_string());
            result.entry("orgs".to_string()).or_default();
            result.entry("orgs_all".to_string()).or_default();
            continue;
        } else if trimmed.starts_with("## `roles.parquet`") {
            current_table = Some("roles".to_string());
            result.entry("roles".to_string()).or_default();
            continue;
        } else if trimmed.starts_with("## `relationships.parquet`") {
            current_table = Some("relationships".to_string());
            result.entry("relationships".to_string()).or_default();
            continue;
        } else if trimmed.starts_with("## `successions.parquet`") {
            current_table = Some("successions".to_string());
            result.entry("successions".to_string()).or_default();
            continue;
        } else if trimmed.starts_with("## ") {
            current_table = None;
            continue;
        }

        if let Some(ref table) = current_table {
            if trimmed.starts_with('|') && trimmed.ends_with('|') {
                // Table row
                let cells: Vec<&str> = trimmed.split('|').map(str::trim).collect();
                if cells.len() >= 3 {
                    let first_cell = cells[1];
                    for col in extract_backtick_columns(first_cell) {
                        if table == "orgs" {
                            result.get_mut("orgs").unwrap().insert(col.clone());
                            result.get_mut("orgs_all").unwrap().insert(col);
                        } else {
                            result.get_mut(table).unwrap().insert(col);
                        }
                    }
                }
            }
        }
    }

    result
}

fn get_code_columns_map() -> HashMap<String, BTreeSet<String>> {
    let pkg = ods::datapackage::generate_datapackage();
    let mut map = HashMap::new();
    for res in pkg["resources"].as_array().unwrap() {
        let name = res["name"].as_str().unwrap().to_string();
        let mut cols = BTreeSet::new();
        for f in res["schema"]["fields"].as_array().unwrap() {
            cols.insert(f["name"].as_str().unwrap().to_string());
        }
        map.insert(name, cols);
    }
    map
}

fn cross_check_columns(
    docs_map: &HashMap<String, BTreeSet<String>>,
    code_map: &HashMap<String, BTreeSet<String>>,
) -> Result<(), String> {
    let mut errors = Vec::new();

    for (table, code_cols) in code_map {
        let docs_cols = match docs_map.get(table) {
            Some(cols) => cols,
            None => {
                errors.push(format!("Table '{}' is present in code schemas but missing in docs/parquet.md", table));
                continue;
            }
        };

        // 1. Column in code but not docs
        for col in code_cols {
            if !docs_cols.contains(col) {
                errors.push(format!("Column '{}.{}' exists in code schema but is missing from docs/parquet.md", table, col));
            }
        }

        // 2. Column in docs but not code
        for col in docs_cols {
            if !code_cols.contains(col) {
                errors.push(format!("Column '{}.{}' is documented in docs/parquet.md but does not exist in code schema", table, col));
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

#[test]
fn test_docs_parquet_md_cross_check() -> Result<()> {
    let docs_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/parquet.md");
    let content = fs::read_to_string(&docs_path)?;
    let docs_map = parse_docs_parquet_md_columns(&content);
    let code_map = get_code_columns_map();

    if let Err(msg) = cross_check_columns(&docs_map, &code_map) {
        panic!("Docs cross-check failed!\n{}", msg);
    }

    Ok(())
}


use anyhow::{bail, Context, Result};
use arrow::array::{Array, Date32Array, StringArray};
use clap::{Parser, ValueEnum};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Table,
    Markdown,
    Csv,
    Json,
    Tsv,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortBy {
    Code,
    Name,
    Postcode,
}

#[derive(Parser, Debug, Clone)]
pub struct Args {
    /// Search query matching organisation name (case-insensitive substring)
    pub query: Option<String>,

    /// Filter by exact ODS code (repeatable and comma-separated, e.g. A82608,RJZ)
    #[arg(long, value_delimiter = ',', num_args = 1..)]
    pub code: Vec<String>,

    /// Filter by location: country -> county -> town -> postcode (minimum 3 characters)
    #[arg(long = "in")]
    pub location: Option<String>,

    /// Filter by role code (e.g. RO76) or curated role name (repeatable and comma-separated). Codes never change; use codes for durable queries and scripts.
    #[arg(short, long, value_delimiter = ',', num_args = 1..)]
    pub role: Vec<String>,

    /// GP practices — RO76, RO227, RO315
    #[arg(long, help_heading = "Role Shortcuts")]
    pub gp: bool,

    /// Dental practices — RO110, RO65
    #[arg(long, help_heading = "Role Shortcuts")]
    pub dentist: bool,

    /// Query the complete historical database (orgs_all.parquet) including inactive/closed entities
    #[arg(long, short)]
    pub all: bool,

    /// Show full role set in stored order without +N de-emphasis
    #[arg(long, short)]
    pub verbose: bool,

    /// Explicit sort order (overrides default relevance ranking)
    #[arg(long, short, value_enum)]
    pub sort: Option<SortBy>,

    /// Output format
    #[arg(long, short, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,

    /// Input directory containing Parquet files (defaults to active release)
    #[arg(long, short)]
    pub input: Option<PathBuf>,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            query: None,
            code: Vec::new(),
            location: None,
            role: Vec::new(),
            gp: false,
            dentist: false,
            all: false,
            verbose: false,
            sort: None,
            format: OutputFormat::Table,
            input: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LocationLevel {
    Country = 1,
    County = 2,
    Town = 3,
    Postcode = 4,
}

impl LocationLevel {
    pub fn name(&self) -> &'static str {
        match self {
            LocationLevel::Country => "country",
            LocationLevel::County => "county",
            LocationLevel::Town => "town",
            LocationLevel::Postcode => "postcode",
        }
    }
}

pub fn normalize_for_matching(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut last_was_space = false;

    for c in s.chars() {
        if c == '\'' || c == '’' || c == '‘' {
            continue;
        } else if c.is_alphanumeric() {
            for lc in c.to_lowercase() {
                result.push(lc);
            }
            last_was_space = false;
        } else {
            if !last_was_space && !result.is_empty() {
                result.push(' ');
                last_was_space = true;
            }
        }
    }

    if result.ends_with(' ') {
        result.pop();
    }
    result
}

pub fn normalize_postcode(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

pub fn compute_name_rank(norm_name: &str, norm_query: &str) -> Option<u8> {
    if norm_query.is_empty() {
        return Some(4);
    }
    if norm_name == norm_query {
        Some(1)
    } else if norm_name.starts_with(norm_query) {
        Some(2)
    } else if norm_name.split_whitespace().any(|w| w.starts_with(norm_query)) || norm_name.contains(&format!(" {norm_query}")) {
        Some(3)
    } else if norm_name.contains(norm_query) {
        Some(4)
    } else {
        None
    }
}

/// Canonical in-memory representation of an organisation row from `orgs.parquet` / `orgs_all.parquet`.
///
/// The field declaration order defines the serialized JSON key order for flattened output
/// records (`ods find --format json` and `ods info --format json`).
/// Verified against `orgs_schema()` in `tests/find_output_record_test.rs`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct OrgRow {
    pub ods_code: String,
    pub name: String,
    pub record_class: String,
    pub role_codes: Vec<String>,
    pub role_names: Vec<String>,
    pub primary_role_code: String,
    pub address: Option<String>,
    pub town: Option<String>,
    pub county: Option<String>,
    pub postcode: Option<String>,
    pub country: Option<String>,
    pub uprn: Option<String>,
    pub telephone: Option<String>,
    pub website: Option<String>,
    pub predecessor_codes: Vec<String>,
    pub successor_codes: Vec<String>,
    pub status: String,
    pub legal_start: Option<String>,
    pub legal_end: Option<String>,
    pub operational_start: Option<String>,
    pub operational_end: Option<String>,
    pub last_changed: Option<String>,
    pub trud_release_date: String,
}

#[derive(Debug, Clone, Copy)]
pub struct OrgColumnIndices {
    pub ods_code: usize,
    pub name: usize,
    pub record_class: usize,
    pub status: usize,
    pub primary_role_code: usize,
    pub role_codes: Option<usize>,
    pub role_names: Option<usize>,
    pub pred_codes: Option<usize>,
    pub succ_codes: Option<usize>,
    pub address: Option<usize>,
    pub town: Option<usize>,
    pub county: Option<usize>,
    pub postcode: Option<usize>,
    pub country: Option<usize>,
    pub uprn: Option<usize>,
    pub telephone: Option<usize>,
    pub website: Option<usize>,
    pub op_start: Option<usize>,
    pub op_end: Option<usize>,
    pub leg_start: Option<usize>,
    pub leg_end: Option<usize>,
    pub last_change: Option<usize>,
    pub trud_release_date: Option<usize>,
}

impl OrgColumnIndices {
    pub fn try_from_schema(schema: &arrow::datatypes::Schema) -> Result<Self> {
        Ok(Self {
            ods_code: schema.index_of("ods_code")?,
            name: schema.index_of("name")?,
            record_class: schema.index_of("record_class")?,
            status: schema.index_of("status")?,
            primary_role_code: schema.index_of("primary_role_code")?,
            role_codes: schema.index_of("role_codes").ok(),
            role_names: schema.index_of("role_names").ok(),
            pred_codes: schema.index_of("predecessor_codes").ok(),
            succ_codes: schema.index_of("successor_codes").ok(),
            address: schema.index_of("address").ok(),
            town: schema.index_of("town").ok(),
            county: schema.index_of("county").ok(),
            postcode: schema.index_of("postcode").ok(),
            country: schema.index_of("country").ok(),
            uprn: schema.index_of("uprn").ok(),
            telephone: schema.index_of("telephone").ok(),
            website: schema.index_of("website").ok(),
            op_start: schema.index_of("operational_start").ok(),
            op_end: schema.index_of("operational_end").ok(),
            leg_start: schema.index_of("legal_start").ok(),
            leg_end: schema.index_of("legal_end").ok(),
            last_change: schema.index_of("last_changed").ok(),
            trud_release_date: schema.index_of("trud_release_date").ok(),
        })
    }
}

pub fn extract_batch_opt_str(
    batch: &arrow::record_batch::RecordBatch,
    idx: Option<usize>,
    row: usize,
) -> Option<String> {
    let idx = idx?;
    let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
    if arr.is_valid(row) {
        Some(arr.value(row).to_string())
    } else {
        None
    }
}

pub fn extract_batch_list(
    batch: &arrow::record_batch::RecordBatch,
    idx: Option<usize>,
    row: usize,
) -> Vec<String> {
    if let Some(idx) = idx {
        if let Some(list_arr) = batch
            .column(idx)
            .as_any()
            .downcast_ref::<arrow::array::ListArray>()
        {
            if list_arr.is_valid(row) {
                let val_arr = list_arr.value(row);
                if let Some(str_arr) = val_arr.as_any().downcast_ref::<StringArray>() {
                    return (0..str_arr.len())
                        .filter(|&j| str_arr.is_valid(j))
                        .map(|j| str_arr.value(j).to_string())
                        .collect();
                }
            }
        }
    }
    Vec::new()
}

pub fn extract_batch_date(
    batch: &arrow::record_batch::RecordBatch,
    idx: Option<usize>,
    row: usize,
) -> Option<String> {
    let idx = idx?;
    let arr = batch.column(idx).as_any().downcast_ref::<Date32Array>()?;
    if arr.is_valid(row) {
        let days = arr.value(row);
        let epoch = chrono::NaiveDate::from_ymd_opt(1970, 1, 1)?;
        let date = epoch.checked_add_signed(chrono::Duration::days(days as i64))?;
        Some(date.format("%Y-%m-%d").to_string())
    } else {
        None
    }
}

pub fn extract_org_row_from_batch(
    batch: &arrow::record_batch::RecordBatch,
    row: usize,
    idx: &OrgColumnIndices,
) -> OrgRow {
    let ods_code = batch
        .column(idx.ods_code)
        .as_any()
        .downcast_ref::<StringArray>()
        .map(|a| a.value(row))
        .unwrap_or("");
    let name = batch
        .column(idx.name)
        .as_any()
        .downcast_ref::<StringArray>()
        .map(|a| a.value(row))
        .unwrap_or("");
    let record_class = batch
        .column(idx.record_class)
        .as_any()
        .downcast_ref::<StringArray>()
        .map(|a| a.value(row))
        .unwrap_or("");
    let status = batch
        .column(idx.status)
        .as_any()
        .downcast_ref::<StringArray>()
        .map(|a| a.value(row))
        .unwrap_or("");
    let primary_role = batch
        .column(idx.primary_role_code)
        .as_any()
        .downcast_ref::<StringArray>()
        .map(|a| a.value(row))
        .unwrap_or("");

    let mut role_codes = extract_batch_list(batch, idx.role_codes, row);
    if role_codes.is_empty() && !primary_role.is_empty() {
        role_codes = vec![primary_role.to_string()];
    }
    let role_names = extract_batch_list(batch, idx.role_names, row);

    OrgRow {
        ods_code: ods_code.to_string(),
        name: name.to_string(),
        record_class: record_class.to_string(),
        role_codes,
        role_names,
        primary_role_code: primary_role.to_string(),
        address: extract_batch_opt_str(batch, idx.address, row),
        town: extract_batch_opt_str(batch, idx.town, row),
        county: extract_batch_opt_str(batch, idx.county, row),
        postcode: extract_batch_opt_str(batch, idx.postcode, row),
        country: extract_batch_opt_str(batch, idx.country, row),
        uprn: extract_batch_opt_str(batch, idx.uprn, row),
        telephone: extract_batch_opt_str(batch, idx.telephone, row),
        website: extract_batch_opt_str(batch, idx.website, row),
        predecessor_codes: extract_batch_list(batch, idx.pred_codes, row),
        successor_codes: extract_batch_list(batch, idx.succ_codes, row),
        status: status.to_string(),
        legal_start: extract_batch_date(batch, idx.leg_start, row),
        legal_end: extract_batch_date(batch, idx.leg_end, row),
        operational_start: extract_batch_date(batch, idx.op_start, row),
        operational_end: extract_batch_date(batch, idx.op_end, row),
        last_changed: extract_batch_date(batch, idx.last_change, row),
        trud_release_date: extract_batch_date(batch, idx.trud_release_date, row).unwrap_or_default(),
    }
}

#[derive(Debug, Clone)]
pub struct Match {
    pub org: OrgRow,
    pub rank: u8,
    pub loc_level: Option<LocationLevel>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Related {
    pub code: String,
    pub name: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutputRecord {
    #[serde(flatten)]
    pub org: OrgRow,
    pub predecessors: Vec<Related>,
    pub successors: Vec<Related>,
}

pub fn build_output_record(
    org: &OrgRow,
    org_metadata: &HashMap<String, (String, String)>,
) -> OutputRecord {
    let predecessors = org.predecessor_codes.iter().map(|code| {
        let (name, status) = org_metadata
            .get(code)
            .cloned()
            .unwrap_or_else(|| (String::new(), "unknown".to_string()));
        Related {
            code: code.clone(),
            name,
            status,
        }
    }).collect();

    let successors = org.successor_codes.iter().map(|code| {
        let (name, status) = org_metadata
            .get(code)
            .cloned()
            .unwrap_or_else(|| (String::new(), "unknown".to_string()));
        Related {
            code: code.clone(),
            name,
            status,
        }
    }).collect();

    OutputRecord {
        org: org.clone(),
        predecessors,
        successors,
    }
}

/// Derives the CSV headers from the serialized representation of an OutputRecord.
///
/// Filters out object-array fields (`predecessors`, `successors`), returning the
/// 23 Parquet schema columns in exact declaration order.
pub fn csv_headers_from_output_record() -> Vec<String> {
    let dummy = OutputRecord {
        org: OrgRow {
            ods_code: String::new(),
            name: String::new(),
            record_class: String::new(),
            role_codes: Vec::new(),
            role_names: Vec::new(),
            primary_role_code: String::new(),
            address: None,
            town: None,
            county: None,
            postcode: None,
            country: None,
            uprn: None,
            telephone: None,
            website: None,
            predecessor_codes: Vec::new(),
            successor_codes: Vec::new(),
            status: String::new(),
            legal_start: None,
            legal_end: None,
            operational_start: None,
            operational_end: None,
            last_changed: None,
            trud_release_date: String::new(),
        },
        predecessors: vec![Related {
            code: String::new(),
            name: String::new(),
            status: String::new(),
        }],
        successors: vec![Related {
            code: String::new(),
            name: String::new(),
            status: String::new(),
        }],
    };
    let val = serde_json::to_value(&dummy).expect("serialize dummy OutputRecord");
    csv_headers_from_json(&val)
}

/// Computes CSV headers from a serialized JSON value by filtering out object-valued arrays.
pub fn csv_headers_from_json(val: &serde_json::Value) -> Vec<String> {
    val.as_object()
        .map(|obj| {
            obj.iter()
                .filter(|(k, v)| match v {
                    serde_json::Value::Object(_) => false,
                    serde_json::Value::Array(arr) => {
                        *k != "predecessors"
                            && *k != "successors"
                            && !arr.iter().any(|x| x.is_object())
                    }
                    _ => true,
                })
                .map(|(k, _v)| k.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// Formats a serialized JSON value for a single CSV cell.
///
/// | serialized value | CSV cell |
/// | string | the string (escaped) |
/// | null | empty |
/// | array of strings | joined with `; ` (escaped) |
/// | array of objects | omitted — JSON only |
pub fn format_csv_cell(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => crate::roles::escape_csv(s),
        serde_json::Value::Null => String::new(),
        serde_json::Value::Array(items) => {
            let str_items: Vec<&str> = items.iter().filter_map(|x| x.as_str()).collect();
            crate::roles::escape_csv(&str_items.join("; "))
        }
        _ => String::new(),
    }
}

#[derive(Debug, Clone)]
pub struct SuccessionEdge {
    pub target_code: String,
    pub date: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SuccessionGraph {
    pub forward: HashMap<String, Vec<SuccessionEdge>>,
    pub reverse: HashMap<String, Vec<SuccessionEdge>>,
}

pub fn load_succession_graph(parquet_dir: &Path) -> SuccessionGraph {
    let mut graph = SuccessionGraph::default();
    let path = parquet_dir.join("successions.parquet");
    if !path.exists() {
        return graph;
    }
    let Ok(file) = File::open(&path) else { return graph };
    let Ok(builder) = ParquetRecordBatchReaderBuilder::try_new(file) else { return graph };
    let Ok(reader) = builder.build() else { return graph };

    for batch in reader.flatten() {
        let schema = batch.schema();
        let (Ok(pred_idx), Ok(succ_idx)) = (
            schema.index_of("predecessor_code"),
            schema.index_of("successor_code"),
        ) else { continue };

        let date_idx = schema.index_of("legal_start").ok();

        let pred_arr = batch.column(pred_idx).as_any().downcast_ref::<StringArray>();
        let succ_arr = batch.column(succ_idx).as_any().downcast_ref::<StringArray>();
        let date_arr = date_idx.and_then(|i| batch.column(i).as_any().downcast_ref::<Date32Array>());

        let (Some(pred_arr), Some(succ_arr)) = (pred_arr, succ_arr) else { continue };

        for i in 0..batch.num_rows() {
            if pred_arr.is_null(i) || succ_arr.is_null(i) {
                continue;
            }
            let pred = pred_arr.value(i).to_string();
            let succ = succ_arr.value(i).to_string();
            let date = date_arr.and_then(|arr| {
                if arr.is_valid(i) {
                    let days = arr.value(i);
                    let epoch = chrono::NaiveDate::from_ymd_opt(1970, 1, 1)?;
                    let d = epoch.checked_add_signed(chrono::Duration::days(days as i64))?;
                    Some(d.format("%Y-%m-%d").to_string())
                } else {
                    None
                }
            });

            graph.forward.entry(pred.clone()).or_default().push(SuccessionEdge {
                target_code: succ.clone(),
                date: date.clone(),
            });
            graph.reverse.entry(succ).or_default().push(SuccessionEdge {
                target_code: pred,
                date,
            });
        }
    }
    graph
}

pub fn load_org_metadata(parquet_dir: &Path) -> HashMap<String, (String, String)> {
    let mut map = HashMap::new();
    let path = if parquet_dir.join("orgs_all.parquet").exists() {
        parquet_dir.join("orgs_all.parquet")
    } else {
        parquet_dir.join("orgs.parquet")
    };
    let Ok(file) = File::open(&path) else { return map };
    let Ok(builder) = ParquetRecordBatchReaderBuilder::try_new(file) else { return map };
    let Ok(reader) = builder.build() else { return map };

    for batch in reader.flatten() {
        let schema = batch.schema();
        let (Ok(code_idx), Ok(name_idx), Ok(status_idx)) = (
            schema.index_of("ods_code"),
            schema.index_of("name"),
            schema.index_of("status"),
        ) else { continue };

        let code_arr = batch.column(code_idx).as_any().downcast_ref::<StringArray>();
        let name_arr = batch.column(name_idx).as_any().downcast_ref::<StringArray>();
        let status_arr = batch.column(status_idx).as_any().downcast_ref::<StringArray>();

        let (Some(code_arr), Some(name_arr), Some(status_arr)) = (code_arr, name_arr, status_arr) else { continue };

        for i in 0..batch.num_rows() {
            if code_arr.is_valid(i) {
                let code = code_arr.value(i).to_string();
                let name = if name_arr.is_valid(i) { name_arr.value(i).to_string() } else { String::new() };
                let status = if status_arr.is_valid(i) { status_arr.value(i).to_string() } else { String::new() };
                map.insert(code, (name, status));
            }
        }
    }
    map
}

#[derive(Debug, Clone)]
pub struct SuccessionHop {
    pub depth: usize,
    pub date: Option<String>,
    pub code: String,
    pub name: String,
    pub status: String,
}

pub fn walk_succession_chain(
    code: &str,
    graph: &SuccessionGraph,
    org_meta: &HashMap<String, (String, String)>,
) -> Vec<SuccessionHop> {
    let mut hops = Vec::new();
    let mut visited = std::collections::HashSet::new();
    visited.insert(code.to_string());

    fn walk(
        curr: &str,
        depth: usize,
        graph: &SuccessionGraph,
        org_meta: &HashMap<String, (String, String)>,
        visited: &mut std::collections::HashSet<String>,
        hops: &mut Vec<SuccessionHop>,
    ) {
        if let Some(edges) = graph.forward.get(curr) {
            for edge in edges {
                if visited.contains(&edge.target_code) {
                    continue;
                }
                visited.insert(edge.target_code.clone());
                let (name, status) = org_meta.get(&edge.target_code)
                    .cloned()
                    .unwrap_or_else(|| (String::new(), "unknown".to_string()));
                hops.push(SuccessionHop {
                    depth,
                    date: edge.date.clone(),
                    code: edge.target_code.clone(),
                    name,
                    status,
                });
                walk(&edge.target_code, depth + 1, graph, org_meta, visited, hops);
            }
        }
    }

    walk(code, 1, graph, org_meta, &mut visited, &mut hops);
    hops
}

pub fn get_predecessors(
    code: &str,
    graph: &SuccessionGraph,
    org_meta: &HashMap<String, (String, String)>,
) -> Vec<SuccessionHop> {
    let mut hops = Vec::new();
    if let Some(edges) = graph.reverse.get(code) {
        for edge in edges {
            let (name, status) = org_meta.get(&edge.target_code)
                .cloned()
                .unwrap_or_else(|| (String::new(), "unknown".to_string()));
            hops.push(SuccessionHop {
                depth: 1,
                date: edge.date.clone(),
                code: edge.target_code.clone(),
                name,
                status,
            });
        }
    }
    hops
}

pub fn resolve_table_successor_display(
    hops: &[SuccessionHop],
) -> Option<String> {
    if hops.is_empty() {
        return None;
    }
    let active_hops: Vec<&SuccessionHop> = hops.iter()
        .filter(|h| h.status.eq_ignore_ascii_case("active"))
        .collect();

    if !active_hops.is_empty() {
        let first = active_hops[0];
        let first_name = if !first.name.is_empty() { &first.name } else { &first.code };
        if active_hops.len() > 1 {
            Some(format!("{first_name} +{}", active_hops.len() - 1))
        } else {
            Some(first_name.to_string())
        }
    } else {
        let first = &hops[0];
        let first_name = if !first.name.is_empty() { &first.name } else { &first.code };
        if hops.len() > 1 {
            Some(format!("{first_name} +{}", hops.len() - 1))
        } else {
            Some(first_name.to_string())
        }
    }
}

/// Find a single organisation by exact ODS code in a Parquet file.
pub fn find_org_by_code(path: &Path, target_code: &str) -> Result<Option<OrgRow>> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let reader = builder.build()?;

    for batch in reader {
        let batch = batch?;
        let indices = OrgColumnIndices::try_from_schema(&batch.schema())?;
        let ods_code_arr = batch
            .column(indices.ods_code)
            .as_any()
            .downcast_ref::<StringArray>()
            .context("ods_code StringArray")?;

        for i in 0..batch.num_rows() {
            if ods_code_arr.is_valid(i) && ods_code_arr.value(i).eq_ignore_ascii_case(target_code) {
                return Ok(Some(extract_org_row_from_batch(&batch, i, &indices)));
            }
        }
    }
    Ok(None)
}

/// Find an organisation across active (orgs.parquet) and historical (orgs_all.parquet).
/// Returns Some((OrgRow, is_from_orgs_all)).
pub fn find_org_in_parquet(
    parquet_dir: &Path,
    target_code: &str,
) -> Result<Option<(OrgRow, bool)>> {
    let orgs_path = parquet_dir.join("orgs.parquet");
    if orgs_path.exists() {
        if let Some(org) = find_org_by_code(&orgs_path, target_code)? {
            return Ok(Some((org, false)));
        }
    }
    let orgs_all_path = parquet_dir.join("orgs_all.parquet");
    if orgs_all_path.exists() {
        if let Some(org) = find_org_by_code(&orgs_all_path, target_code)? {
            return Ok(Some((org, true)));
        }
    }
    Ok(None)
}

struct ParsedLocation {
    norm: String,
    norm_postcode: String,
}

struct ParsedRoleFilter {
    codes: Vec<String>,
    names: Vec<String>,
}

pub fn run(args: Args) -> Result<()> {
    if args.query.is_none()
        && args.code.is_empty()
        && args.location.is_none()
        && args.role.is_empty()
        && !args.gp
        && !args.dentist
    {
        if args.format == OutputFormat::Table || args.format == OutputFormat::Markdown {
            eprintln!(
                "No search filters given. Try:\n\n  ods find sedbergh                 by name\n  ods find --in SW9                 by postcode, town, county or country\n  ods find --role RO76              by role\n\nOr browse with a fuzzy finder:\n\n  ods find --format tsv | fzf\n\nPick one and open it:\n\n  ods find --format tsv | fzf | cut -f1 | xargs ods info"
            );
            return Ok(());
        }
    }

    let resolved_input = crate::workspace::resolve_parquet_input(args.input.as_deref())?;

    run_with_writer(args, &mut std::io::stdout(), &resolved_input)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedFooterTable {
    pub table: String,
    pub overflow_hint: Option<String>,
}

/// Renders a comfy-table with a boxed footer row derived from the bottom border.
///
/// Pops the rendered bottom border (`└───┴───┘`) and produces three lines:
/// 1. merge: bottom border with `└`->`├` and `┘`->`┤`, leaving all `┴` dividers untouched.
/// 2. footer: `│ ` + left + padding + right + ` │`, measuring display width with `unicode_width`.
/// 3. bottom: bottom border with every `┴`->`─`.
///
/// If `left`, `right`, and a single-space gap do not fit `inner_width`, the right-hand hint
/// is dropped from the footer and returned as `overflow_hint` to be printed as a `*` line below.
pub fn render_with_footer(table: &comfy_table::Table, left: &str, right: &str) -> RenderedFooterTable {
    use unicode_width::UnicodeWidthStr;

    let rendered = table.to_string();
    let mut lines: Vec<&str> = rendered.lines().collect();
    if lines.is_empty() {
        return RenderedFooterTable {
            table: rendered,
            overflow_hint: None,
        };
    }

    let bottom_border = lines.pop().unwrap();
    let border_width = bottom_border.width();

    let style = table.style();
    let b_left = style.bottom_border.left;
    let b_right = style.bottom_border.right;
    let b_junction = style.bottom_border.junction;
    let b_fill = style.bottom_border.fill;

    let m_left = style.row_separator.left.unwrap_or('├');
    let m_right = style.row_separator.right.unwrap_or('┤');

    // 1. Merge line: replace bottom-left corner with m_left, bottom-right corner with m_right, junctions untouched
    let mut merge_chars: Vec<char> = bottom_border.chars().collect();
    if let Some(first) = merge_chars.first_mut() {
        if b_left == Some(*first) {
            *first = m_left;
        }
    }
    if let Some(last) = merge_chars.last_mut() {
        if b_right == Some(*last) {
            *last = m_right;
        }
    }
    let merge_line: String = merge_chars.into_iter().collect();

    // 2. Bottom line: replace junction with fill
    let bottom_line = match (b_junction, b_fill) {
        (Some(j), Some(f)) => bottom_border.replace(j, &f.to_string()),
        _ => bottom_border.to_string(),
    };

    // 3. Footer line & overflow hint
    // Box borders: "│ " (2) and " │" (2) -> 4 columns total
    let inner_width = border_width.saturating_sub(4);
    let left_width = left.width();
    let right_width = right.width();

    let (footer_line, overflow_hint) = if left_width + 1 + right_width <= inner_width {
        let padding = inner_width - left_width - right_width;
        (
            format!("│ {}{}{} │", left, " ".repeat(padding), right),
            None,
        )
    } else {
        let padding = inner_width.saturating_sub(left_width);
        (
            format!("│ {}{} │", left, " ".repeat(padding)),
            Some(right.to_string()),
        )
    };

    let mut result = lines.join("\n");
    if !result.is_empty() {
        result.push('\n');
    }
    result.push_str(&merge_line);
    result.push('\n');
    result.push_str(&footer_line);
    result.push('\n');
    result.push_str(&bottom_line);

    RenderedFooterTable {
        table: result,
        overflow_hint,
    }
}

pub fn run_with_writer(args: Args, writer: &mut dyn std::io::Write, parquet_dir: &Path) -> Result<()> {
    let file_name = if args.all { "orgs_all.parquet" } else { "orgs.parquet" };
    let release_date = crate::provenance::OdsProvenance::load_from_dir(parquet_dir)
        .and_then(|p| p.trud_release_date)
        .or_else(|| {
            crate::workspace::find_workspace_root_from(parquet_dir, None)
                .and_then(|r| crate::workspace::Workspace::open(Some(&r)).ok())
                .and_then(|ws| ws.active_release().ok().map(|(d, _)| d))
        });
    let path = parquet_dir.join(file_name);
    if !path.exists() {
        if args.input.is_some() || parquet_dir.join(crate::provenance::PROVENANCE_FILENAME).exists() {
            anyhow::bail!(
                "✖ Parquet file '{}' not found in '{}'",
                file_name,
                parquet_dir.display()
            );
        }
        if let Ok(ws) = crate::workspace::Workspace::open(None) {
            let releases = ws.releases().unwrap_or_default();
            if !releases.is_empty() {
                let n = releases.len();
                let count_str = if n == 1 { "1 release".to_string() } else { format!("{} releases", n) };
                let newest_date = &releases[0].date;
                let ws_name = ws.root().file_name().and_then(|n| n.to_str()).unwrap_or(crate::workspace::DEFAULT_WORKSPACE_DIR);
                anyhow::bail!(
                    "✖ No active release pinned\n  {} in {}/releases/, none active.\n  Pin one:  ods use {}",
                    count_str,
                    ws_name,
                    newest_date
                );
            }
        }
        anyhow::bail!(
            "✖ no ods workspace found here\n  Pass -i <trud.zip> -o <dir>, or run `ods pull` or `ods trud pull` to create a workspace."
        );
    }

    // Expand alias flags (--gp, --dentist) into role filters with OR semantics
    let mut effective_roles = args.role.clone();
    let mut alias_notices: Vec<String> = Vec::new();

    if args.gp {
        if let Some(alias) = crate::roles::role_aliases().aliases.get("gp") {
            effective_roles.extend(alias.codes.clone());
            let names: Vec<&str> = alias
                .codes
                .iter()
                .filter_map(|c| crate::roles::role_names().name(c))
                .collect();
            alias_notices.push(format!(
                "* --gp: {} — {}",
                alias.codes.join(", "),
                names.join(", ")
            ));
        }
    }

    if args.dentist {
        if let Some(alias) = crate::roles::role_aliases().aliases.get("dentist") {
            effective_roles.extend(alias.codes.clone());
            let names: Vec<&str> = alias
                .codes
                .iter()
                .filter_map(|c| crate::roles::role_names().name(c))
                .collect();
            alias_notices.push(format!(
                "* --dentist: {} — {}",
                alias.codes.join(", "),
                names.join(", ")
            ));
        }
    }

    // 1. Validate --in location filter
    let parsed_location = if let Some(ref loc) = args.location {
        let trimmed = loc.trim();
        if trimmed.chars().count() < 3 {
            bail!("✖ Location query '{}' is too short (minimum 3 characters)", trimmed);
        }
        Some(ParsedLocation {
            norm: normalize_for_matching(trimmed),
            norm_postcode: normalize_postcode(trimmed),
        })
    } else {
        None
    };

    // 2. Validate --role filter (codes or curated names)
    let parsed_roles = if !effective_roles.is_empty() {
        let vocab = crate::roles::role_names();
        let mut codes = Vec::new();
        let mut names = Vec::new();

        for r_input in &effective_roles {
            let trimmed = r_input.trim();
            if trimmed.is_empty() {
                continue;
            }
            let is_code = (trimmed.starts_with("RO") || trimmed.starts_with("ro"))
                && trimmed[2..].chars().all(|c| c.is_ascii_digit());
            if is_code {
                let code_upper = trimmed.to_uppercase();
                if !vocab.names.contains_key(&code_upper) {
                    bail!("✖ Unknown role code '{}'\n  Check known role codes with `ods role`", trimmed);
                }
                codes.push(code_upper);
            } else {
                let norm_input = normalize_for_matching(trimmed);
                let matched_entry = vocab.names.iter().find(|(_, vn)| normalize_for_matching(vn) == norm_input);
                if let Some((code, valid_name)) = matched_entry {
                    codes.push(code.clone());
                    names.push(valid_name.clone());
                } else {
                    let suggestions = crate::roles::find_role_suggestions(trimmed);
                    let mut msg = format!("✖ No role named '{}'\n", trimmed);
                    if !suggestions.is_empty() {
                        let names_str = suggestions.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(" · ");
                        let codes_str = suggestions.iter().map(|(_, c)| *c).collect::<Vec<_>>().join(",");
                        msg.push_str(&format!("  Did you mean: {}\n  Or use codes: ods find --role {}", names_str, codes_str));
                    }
                    bail!("{}", msg);
                }
            }
        }
        Some(ParsedRoleFilter { codes, names })
    } else {
        None
    };

    // 3. Normalised name query
    let norm_query = args.query.as_deref().map(normalize_for_matching).unwrap_or_default();

    let file = File::open(&path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let reader = builder.build()?;

    let successions_graph = if args.all && (args.format == OutputFormat::Table || args.format == OutputFormat::Markdown) {
        Some(load_succession_graph(parquet_dir))
    } else {
        None
    };
    let org_metadata = load_org_metadata(parquet_dir);

    let mut matches: Vec<Match> = Vec::new();

    for batch in reader {
        let batch = batch?;
        let schema = batch.schema();
        let num_rows = batch.num_rows();

        let ods_code_arr = batch.column(schema.index_of("ods_code")?)
            .as_any().downcast_ref::<StringArray>().context("ods_code StringArray")?;
        let name_arr = batch.column(schema.index_of("name")?)
            .as_any().downcast_ref::<StringArray>().context("name StringArray")?;
        let primary_role_arr = batch.column(schema.index_of("primary_role_code")?)
            .as_any().downcast_ref::<StringArray>().context("primary_role_code StringArray")?;

        let indices = OrgColumnIndices::try_from_schema(&schema)?;

        for i in 0..num_rows {
            let ods_code = ods_code_arr.value(i);

            // Filter by exact ODS code if --code specified
            if !args.code.is_empty() {
                let matches_code = args.code.iter().any(|c| c.eq_ignore_ascii_case(ods_code));
                if !matches_code {
                    continue;
                }
            }

            let name = name_arr.value(i);
            let norm_name = normalize_for_matching(name);

            // Match name against positional query
            let rank = if args.query.is_some() {
                if let Some(r) = compute_name_rank(&norm_name, &norm_query) {
                    r
                } else {
                    continue;
                }
            } else {
                4
            };

            let primary_role = primary_role_arr.value(i);

            // Extract all role codes
            let role_codes = extract_batch_list(&batch, indices.role_codes, i);
            let role_codes = if role_codes.is_empty() {
                vec![primary_role.to_string()]
            } else {
                role_codes
            };

            // Extract all role names
            let role_names = extract_batch_list(&batch, indices.role_names, i);

            // Filter by --role if specified
            if let Some(ref rf) = parsed_roles {
                let matches_code = rf.codes.iter().any(|c| role_codes.iter().any(|rc| rc.eq_ignore_ascii_case(c)));
                let matches_name = rf.names.iter().any(|n| role_names.iter().any(|rn| rn.eq_ignore_ascii_case(n)));
                if !matches_code && !matches_name {
                    continue;
                }
            }

            let postcode = extract_batch_opt_str(&batch, indices.postcode, i);
            let town = extract_batch_opt_str(&batch, indices.town, i);
            let county = extract_batch_opt_str(&batch, indices.county, i);
            let country = extract_batch_opt_str(&batch, indices.country, i);

            // Evaluate location matching level
            let loc_level: Option<LocationLevel> = if let Some(ref loc_info) = parsed_location {
                let norm_country = normalize_for_matching(country.as_deref().unwrap_or(""));
                let norm_county = normalize_for_matching(county.as_deref().unwrap_or(""));
                let norm_town = normalize_for_matching(town.as_deref().unwrap_or(""));
                let norm_post = normalize_postcode(postcode.as_deref().unwrap_or(""));

                if norm_country.starts_with(&loc_info.norm) || norm_country == loc_info.norm {
                    Some(LocationLevel::Country)
                } else if norm_county.starts_with(&loc_info.norm) || norm_county == loc_info.norm {
                    Some(LocationLevel::County)
                } else if norm_town.starts_with(&loc_info.norm) || norm_town == loc_info.norm {
                    Some(LocationLevel::Town)
                } else if !loc_info.norm_postcode.is_empty() && norm_post.starts_with(&loc_info.norm_postcode) {
                    Some(LocationLevel::Postcode)
                } else {
                    None
                }
            } else {
                None
            };

            if parsed_location.is_some() && loc_level.is_none() {
                continue;
            }

            let org = extract_org_row_from_batch(&batch, i, &indices);

            matches.push(Match {
                org,
                rank,
                loc_level,
            });
        }
    }

    // 4. Broad-to-narrow location filtering (retain only winning level)
    let matched_loc_level = if let Some(ref loc) = args.location {
        let min_level = matches.iter().filter_map(|m| m.loc_level).min();
        match min_level {
            Some(lvl) => {
                matches.retain(|m| m.loc_level == Some(lvl));
                Some(lvl)
            }
            None => {
                bail!("✖ No organisation found in '{}'\n  --in matches country, county, town or postcode.", loc.trim());
            }
        }
    } else {
        None
    };

    // 5. Sorting
    matches.sort_by(|a, b| {
        if let Some(sort_by) = args.sort {
            match sort_by {
                SortBy::Code => a.org.ods_code.cmp(&b.org.ods_code),
                SortBy::Name => a.org.name.to_lowercase().cmp(&b.org.name.to_lowercase()),
                SortBy::Postcode => {
                    let a_post = a.org.postcode.as_deref().unwrap_or("");
                    let b_post = b.org.postcode.as_deref().unwrap_or("");
                    a_post.to_lowercase().cmp(&b_post.to_lowercase())
                }
            }
        } else if args.query.is_some() {
            a.rank.cmp(&b.rank).then_with(|| a.org.ods_code.cmp(&b.org.ods_code))
        } else {
            a.org.ods_code.cmp(&b.org.ods_code)
        }
    });

    let matched_count = matches.len();

    match args.format {
        OutputFormat::Json => {
            for m in &matches {
                let record = build_output_record(&m.org, &org_metadata);
                let json = serde_json::to_value(&record)?;
                writeln!(writer, "{}", json)?;
            }
        }
        OutputFormat::Csv => {
            let headers = csv_headers_from_output_record();
            writeln!(writer, "{}", headers.join(","))?;
            for m in &matches {
                let record = build_output_record(&m.org, &org_metadata);
                let val = serde_json::to_value(&record)?;
                let row_cells: Vec<String> = headers
                    .iter()
                    .map(|h| format_csv_cell(&val[h]))
                    .collect();
                writeln!(writer, "{}", row_cells.join(","))?;
            }
        }
        OutputFormat::Tsv | OutputFormat::Table | OutputFormat::Markdown => {
            struct DisplayRow<'a> {
                ods_code: &'a str,
                full_name: String,
                postcode: &'a str,
                role_display: String,
                record_class: &'a str,
                status: &'a str,
            }

            let display_rows: Vec<DisplayRow> = matches
                .iter()
                .map(|r| {
                    let full_name = if args.all && r.org.status.eq_ignore_ascii_case("inactive") {
                        if let Some(ref graph) = successions_graph {
                            let succ_hops = walk_succession_chain(&r.org.ods_code, graph, &org_metadata);
                            if let Some(live_succ) = resolve_table_successor_display(&succ_hops) {
                                format!("{} → {}", r.org.name, live_succ)
                            } else {
                                r.org.name.clone()
                            }
                        } else {
                            r.org.name.clone()
                        }
                    } else {
                        r.org.name.clone()
                    };

                    let role_display = crate::roles::format_roles_for_display(&r.org.role_codes, &r.org.role_names, args.verbose);

                    DisplayRow {
                        ods_code: &r.org.ods_code,
                        full_name,
                        postcode: r.org.postcode.as_deref().unwrap_or(""),
                        role_display,
                        record_class: &r.org.record_class,
                        status: &r.org.status,
                    }
                })
                .collect();

            if args.format == OutputFormat::Tsv {
                for row in &display_rows {
                    if args.all {
                        writeln!(
                            writer,
                            "{}\t{}\t{}\t{}\t{}\t{}",
                            row.ods_code, row.full_name, row.postcode, row.role_display, row.record_class, row.status
                        )?;
                    } else {
                        writeln!(
                            writer,
                            "{}\t{}\t{}\t{}\t{}",
                            row.ods_code, row.full_name, row.postcode, row.role_display, row.record_class
                        )?;
                    }
                }
            } else {
                use std::io::IsTerminal;
                use comfy_table::{Table, ContentArrangement, presets};

                let mut table = Table::new();

                if args.format == OutputFormat::Markdown {
                    table.load_style(presets::ASCII_MARKDOWN);
                    table.set_content_arrangement(ContentArrangement::Disabled);
                } else {
                    table.load_style(presets::UTF8_FULL_CONDENSED);
                    table.set_truncation_indicator("…");
                    table.set_content_arrangement(ContentArrangement::Dynamic);
                    if let Ok(col_env) = std::env::var("COLUMNS").and_then(|c| c.parse::<u16>().map_err(|_| std::env::VarError::NotPresent)) {
                        table.set_width(col_env);
                    } else if std::io::stdout().is_terminal() {
                        if let Ok((cols, _)) = crossterm::terminal::size() {
                            table.set_width(cols);
                        }
                    } else {
                        table.set_width(120);
                    }
                }

                if args.all {
                    table.set_header(vec!["ODS Code", "Name", "Postcode", "Roles", "Class", "Status"]);
                } else {
                    table.set_header(vec!["ODS Code", "Name", "Postcode", "Roles", "Class"]);
                }

                for row in &display_rows {
                    if args.all {
                        table.add_row(vec![
                            row.ods_code,
                            &row.full_name,
                            row.postcode,
                            &row.role_display,
                            row.record_class,
                            row.status,
                        ]);
                    } else {
                        table.add_row(vec![
                            row.ods_code,
                            &row.full_name,
                            row.postcode,
                            &row.role_display,
                            row.record_class,
                        ]);
                    }
                }

                if args.format == OutputFormat::Markdown {
                    for notice in &alias_notices {
                        writeln!(writer, "{}", notice)?;
                    }
                    writeln!(writer, "{}", table)?;
                } else {
                    // OutputFormat::Table
                    // 1. Search line
                    let search_desc = match (&args.query, &args.location, matched_loc_level) {
                        (Some(q), Some(loc), Some(lvl)) => format!("\"{}\" in name, \"{}\" in {}", q, loc, lvl.name()),
                        (Some(q), Some(loc), None) => format!("\"{}\" in name, \"{}\" in location", q, loc),
                        (None, Some(loc), Some(lvl)) => format!("\"{}\" in {}", loc, lvl.name()),
                        (None, Some(loc), None) => format!("\"{}\" in location", loc),
                        (Some(q), None, _) => format!("\"{}\" in name", q),
                        (None, None, _) => {
                            if !args.code.is_empty() {
                                format!("\"{}\" in code", args.code.join(", "))
                            } else if !args.role.is_empty() {
                                format!("\"{}\" in role", args.role.join(", "))
                            } else if args.gp {
                                "GP practices".to_string()
                            } else if args.dentist {
                                "Dental practices".to_string()
                            } else {
                                "all".to_string()
                            }
                        }
                    };
                    writeln!(writer, "* Search: {}", search_desc)?;

                    // 2. Source line
                    let source_rel = if let Some(ref d) = release_date {
                        format!("releases/{}/{}", d, file_name)
                    } else {
                        let ws_root = crate::workspace::find_workspace_root_from(parquet_dir, None);
                        if let Some(ref root) = ws_root {
                            parquet_dir.strip_prefix(root).unwrap_or(parquet_dir).join(file_name).display().to_string()
                        } else {
                            parquet_dir.join(file_name).display().to_string()
                        }
                    };
                    writeln!(writer, "* Source: {}", source_rel)?;

                    // 3. Disagreement notice (!) directly under Source
                    if let Some(ref d) = release_date {
                        if let crate::workspace::ReleaseResolution::Disagreement { ref cwd_date } =
                            crate::workspace::check_release_resolution(d, Some(parquet_dir))
                        {
                            writeln!(
                                writer,
                                "! Run from releases/{}. Change source with: ods use {}",
                                cwd_date, cwd_date
                            )?;
                        }
                    }

                    // 4. Any alias notices
                    for notice in &alias_notices {
                        writeln!(writer, "{}", notice)?;
                    }

                    // 5. Render table with footer
                    let n = matched_count;
                    let (left, right) = if args.all {
                        let active_count = matches.iter().filter(|r| r.org.status.eq_ignore_ascii_case("active")).count();
                        let inactive_count = n.saturating_sub(active_count);
                        let left_str = if n == 1 {
                            "1 record".to_string()
                        } else {
                            format!("{} records", n)
                        };
                        let right_str = format!("{} active · {} inactive", active_count, inactive_count);
                        (left_str, right_str)
                    } else {
                        let left_str = if n == 1 {
                            "1 active record".to_string()
                        } else {
                            format!("{} active records", n)
                        };
                        let right_str = "Use --all to include inactive".to_string();
                        (left_str, right_str)
                    };

                    let rendered_footer = render_with_footer(&table, &left, &right);
                    writeln!(writer, "{}", rendered_footer.table)?;
                    if let Some(hint) = rendered_footer.overflow_hint {
                        writeln!(writer, "* {}", hint)?;
                    }
                }
            }
        }
    }

    // Print ODS code hint if query happens to be a valid ODS code in the dataset
    if args.format == OutputFormat::Table || args.format == OutputFormat::Markdown {
        if let Some(ref q) = args.query {
            let q_clean = q.trim().to_uppercase();
            if org_metadata.contains_key(&q_clean) {
                writeln!(writer, "\n* '{}' is also an ODS code — ods info {}", q_clean, q_clean)?;
            }
        }
    }

    let is_machine = args.format == OutputFormat::Json
        || args.format == OutputFormat::Csv
        || args.format == OutputFormat::Tsv;

    // Staleness nudge: check index age if human-readable output
    if !is_machine {
        let ws_root = crate::workspace::find_workspace_root_from(parquet_dir, None);
        let index_opt = ws_root
            .as_deref()
            .and_then(|r| crate::index::OdsReleaseIndex::load_from_workspace(r).ok().flatten())
            .or_else(|| crate::index::OdsReleaseIndex::baked().ok());
        if let Some(index) = index_opt {
            crate::workspace::check_and_emit_staleness_nudge(&index, false);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_load_succession_graph_nonexistent() {
        let dir = tempdir().unwrap();
        let graph = load_succession_graph(dir.path());
        assert!(graph.forward.is_empty());
        assert!(graph.reverse.is_empty());
    }

    fn setup_synthetic_parquet() -> (tempfile::TempDir, PathBuf) {
        let dir = tempdir().unwrap();
        let parquet_dir = dir.path().to_path_buf();

        // Create synthetic OdsRecord list
        let records = vec![
            crate::ods_xml::OdsRecord {
                ods_code: "A101".to_string(),
                name: "Alpha Health Centre".to_string(),
                status: "active".to_string(),
                role: "prescribing cost centre".to_string(),
                record_class: "org".to_string(),
                geo_loc: Some(crate::ods_xml::Location {
                    address_lines: vec!["1 Main Street".to_string()],
                    town: Some("London".to_string()),
                    county: Some("Greater London".to_string()),
                    postcode: Some("SW1A 1AA".to_string()),
                    country: Some("ENGLAND".to_string()),
                    uprn: Some("10001".to_string()),
                }),
                roles: vec![
                    crate::ods_xml::OdsRole {
                        id: "RO177".to_string(),
                        code: None,
                        display_name: Some("prescribing cost centre".to_string()),
                        unique_role_id: "1".to_string(),
                        primary_role: true,
                        status: "active".to_string(),
                        dates: vec![],
                    },
                    crate::ods_xml::OdsRole {
                        id: "RO76".to_string(),
                        code: None,
                        display_name: Some("gp practice".to_string()),
                        unique_role_id: "2".to_string(),
                        primary_role: false,
                        status: "active".to_string(),
                        dates: vec![],
                    },
                ],
                relationships: vec![
                    crate::ods_xml::OdsRelationship {
                        id: "RE4".to_string(),
                        display_name: Some("IS COMMISSIONED BY".to_string()),
                        unique_rel_id: "1001".to_string(),
                        status: "active".to_string(),
                        dates: vec![],
                        target: crate::ods_xml::OdsRelationshipTarget {
                            ods_code: "00A".to_string(),
                            name: Some("NHS London ICB".to_string()),
                            ..Default::default()
                        },
                    }
                ],
                ..Default::default()
            },
            crate::ods_xml::OdsRecord {
                ods_code: "B202".to_string(),
                name: "Beta Surgery".to_string(),
                status: "inactive".to_string(),
                role: "branch surgery".to_string(),
                record_class: "site".to_string(),
                geo_loc: Some(crate::ods_xml::Location {
                    address_lines: vec!["2 High Street".to_string()],
                    town: Some("Manchester".to_string()),
                    county: None,
                    postcode: Some("M1 1AA".to_string()),
                    country: Some("ENGLAND".to_string()),
                    uprn: None,
                }),
                successors: vec![
                    crate::ods_xml::OdsSuccessor {
                        unique_succ_id: "1".to_string(),
                        succ_type: "Successor".to_string(),
                        dates: vec![],
                        target: crate::ods_xml::OdsRelationshipTarget {
                            ods_code: "A101".to_string(),
                            name: Some("Alpha Health Centre".to_string()),
                            root: None,
                            assigning_authority_name: None,
                            primary_role_id: None,
                            primary_role_display_name: None,
                            primary_role_unique_role_id: None,
                        },
                    }
                ],
                ..Default::default()
            },
        ];


        let edges = crate::commands::parquet::build_succession_edges(&records);
        let (succ_closures, pred_closures) = crate::commands::parquet::compute_transitive_closures(&records, &edges);

        let mut prov = crate::provenance::OdsProvenance::default();
        prov.trud_release_date = Some("2026-07-31".to_string());

        crate::commands::parquet::export_orgs(&parquet_dir, &records, &succ_closures, &pred_closures, Some(&prov)).unwrap();
        crate::commands::parquet::export_orgs_all(&parquet_dir, &records, &succ_closures, &pred_closures, Some(&prov)).unwrap();
        crate::commands::parquet::export_roles(&parquet_dir, &records, Some(&prov)).unwrap();
        crate::commands::parquet::export_relationships(&parquet_dir, &records, Some(&prov)).unwrap();
        crate::commands::parquet::export_successions(&parquet_dir, &records, Some(&prov)).unwrap();

        (dir, parquet_dir)
    }

    #[test]
    fn test_find_with_synthetic_parquet() {
        let (_dir, parquet_dir) = setup_synthetic_parquet();

        // Test 1: find active record (Table mode)
        let mut out = Vec::new();
        run_with_writer(
            Args {
                query: Some("Alpha".to_string()),
                sort: Some(SortBy::Code),
                format: OutputFormat::Table,
                input: Some(parquet_dir.clone()),
                ..Default::default()
            },
            &mut out,
            &parquet_dir,
        ).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("A101"));
        assert!(s.contains("Alpha Health Centre"));
        assert!(s.contains("SW1A 1AA"));

        // Test 2: find secondary role filtering
        let mut out = Vec::new();
        run_with_writer(
            Args {
                query: Some("Alpha".to_string()),
                role: vec!["gp practice".to_string()],
                sort: Some(SortBy::Code),
                format: OutputFormat::Table,
                input: Some(parquet_dir.clone()),
                ..Default::default()
            },
            &mut out,
            &parquet_dir,
        ).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("A101"), "Secondary role filter should match GP practice");

        // Test 3: find --all including inactive with successor in table
        let mut out = Vec::new();
        run_with_writer(
            Args {
                query: Some("Beta".to_string()),
                all: true,
                sort: Some(SortBy::Code),
                format: OutputFormat::Table,
                input: Some(parquet_dir.clone()),
                ..Default::default()
            },
            &mut out,
            &parquet_dir,
        ).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("B202"));
        assert!(s.contains("A101") || s.contains("Alpha Health Centre")); // Successor

        // Test 4: find output is table, and ods info provides detail inspector
        let mut out = Vec::new();
        run_with_writer(
            Args {
                query: Some("Alpha".to_string()),
                code: vec!["A101".to_string()],
                sort: Some(SortBy::Code),
                format: OutputFormat::Table,
                input: Some(parquet_dir.clone()),
                ..Default::default()
            },
            &mut out,
            &parquet_dir,
        ).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("ODS Code") && s.contains("A101"));

        let mut info_out = Vec::new();
        crate::commands::info::run_with_writer(
            crate::commands::info::Args {
                ods_code: "A101".to_string(),
                format: crate::commands::info::OutputFormat::Markdown,
                input: Some(parquet_dir.clone()),
                ..Default::default()
            },
            &mut info_out,
            &parquet_dir,
        ).unwrap();
        let info_s = String::from_utf8(info_out).unwrap();
        assert!(info_s.contains("Alpha Health Centre"));
        assert!(info_s.contains("A101"));
        assert!(info_s.contains("GP Practice") && info_s.contains("RO76"), "expected curated role name\n{info_s}");

        // Test 5: CSV output
        let mut out = Vec::new();
        run_with_writer(
            Args {
                query: Some("Alpha".to_string()),
                sort: Some(SortBy::Code),
                format: OutputFormat::Csv,
                input: Some(parquet_dir.clone()),
                ..Default::default()
            },
            &mut out,
            &parquet_dir,
        ).unwrap();
        let s = String::from_utf8(out).unwrap();
        let expected_header = csv_headers_from_output_record().join(",");
        assert!(s.starts_with(&expected_header));
        assert!(s.contains("A101,Alpha Health Centre,org"));

        // Test 6: JSON output
        let mut out = Vec::new();
        run_with_writer(
            Args {
                query: Some("Alpha".to_string()),
                sort: Some(SortBy::Code),
                format: OutputFormat::Json,
                input: Some(parquet_dir.clone()),
                ..Default::default()
            },
            &mut out,
            &parquet_dir,
        ).unwrap();
        let s = String::from_utf8(out).unwrap();
        let val: serde_json::Value = serde_json::from_str(s.trim()).unwrap();
        assert_eq!(val["ods_code"], "A101");
        assert_eq!(val["name"], "Alpha Health Centre");
    }

    #[test]
    fn test_find_json_schema_matches_parquet_schema() {
        let (dir, parquet_dir) = setup_synthetic_parquet();
        let _keep_dir = dir;

        let parquet_schema = crate::commands::parquet::orgs_schema();
        let parquet_fields = parquet_schema.fields();

        // 1. Assert OrgRow serialized keys equals orgs_schema() names in order, derived at runtime
        let dummy_org = OrgRow {
            ods_code: "X".to_string(),
            name: "X".to_string(),
            record_class: "X".to_string(),
            role_codes: vec!["R1".to_string()],
            role_names: vec!["Role 1".to_string()],
            primary_role_code: "R1".to_string(),
            address: None,
            town: None,
            county: None,
            postcode: None,
            country: None,
            uprn: None,
            telephone: None,
            website: None,
            predecessor_codes: vec![],
            successor_codes: vec![],
            status: "X".to_string(),
            legal_start: None,
            legal_end: None,
            operational_start: None,
            operational_end: None,
            last_changed: None,
            trud_release_date: "X".to_string(),
        };

        let org_val = serde_json::to_value(&dummy_org).unwrap();
        let org_obj = org_val.as_object().unwrap();
        let org_keys: Vec<String> = org_obj.keys().cloned().collect();

        assert_eq!(org_keys.len(), parquet_fields.len());
        for (i, field) in parquet_fields.iter().enumerate() {
            assert_eq!(
                &org_keys[i],
                field.name(),
                "OrgRow key index {} mismatch with orgs_schema()",
                i
            );

            // Assert nullability agrees: Option<String> is nullable, non-Option is not
            let val = &org_obj[field.name()];
            if field.is_nullable() {
                assert!(
                    val.is_null(),
                    "Field '{}' is nullable in schema but not null in dummy OrgRow",
                    field.name()
                );
            } else {
                assert!(
                    !val.is_null(),
                    "Field '{}' is non-nullable in schema but is null in dummy OrgRow",
                    field.name()
                );
            }
        }

        // 2. Call run_with_writer on live parquet and verify JSON output has 25 keys
        let mut out = Vec::new();
        run_with_writer(
            Args {
                query: Some("Alpha".to_string()),
                sort: Some(SortBy::Code),
                format: OutputFormat::Json,
                input: Some(parquet_dir.clone()),
                ..Default::default()
            },
            &mut out,
            &parquet_dir,
        ).unwrap();
        let s = String::from_utf8(out).unwrap();
        let line = s.lines().next().expect("at least one JSON line");
        let val: serde_json::Value = serde_json::from_str(line).unwrap();
        let obj = val.as_object().unwrap();
        let json_keys: Vec<String> = obj.keys().cloned().collect();

        assert_eq!(json_keys.len(), 25);
        for (i, field) in parquet_fields.iter().enumerate() {
            assert_eq!(&json_keys[i], field.name());
        }
        assert_eq!(json_keys[23], "predecessors");
        assert_eq!(json_keys[24], "successors");
    }

    #[test]
    fn test_find_csv_schema_matches_json_schema() {
        let (dir, parquet_dir) = setup_synthetic_parquet();
        let _keep_dir = dir;

        let mut json_out = Vec::new();
        run_with_writer(
            Args {
                query: Some("Alpha".to_string()),
                sort: Some(SortBy::Code),
                format: OutputFormat::Json,
                input: Some(parquet_dir.clone()),
                ..Default::default()
            },
            &mut json_out,
            &parquet_dir,
        ).unwrap();
        let json_str = String::from_utf8(json_out).unwrap();
        let json_val: serde_json::Value = serde_json::from_str(json_str.lines().next().unwrap()).unwrap();
        let json_obj = json_val.as_object().unwrap();

        let mut csv_out = Vec::new();
        run_with_writer(
            Args {
                query: Some("Alpha".to_string()),
                sort: Some(SortBy::Code),
                format: OutputFormat::Csv,
                input: Some(parquet_dir.clone()),
                ..Default::default()
            },
            &mut csv_out,
            &parquet_dir,
        ).unwrap();
        let csv_str = String::from_utf8(csv_out).unwrap();
        let header_line = csv_str.lines().next().unwrap();
        let csv_headers: Vec<String> = header_line.split(',').map(|s| s.to_string()).collect();

        let expected_csv_headers: Vec<String> = json_obj
            .iter()
            .filter(|(k, _)| *k != "predecessors" && *k != "successors")
            .map(|(k, _)| k.clone())
            .collect();

        assert_eq!(
            csv_headers, expected_csv_headers,
            "CSV output headers do not match JSON key sequence minus object arrays!"
        );
        assert_eq!(csv_headers.len(), 23);
        assert_eq!(json_obj.len(), 25);
    }

    #[test]
    fn test_render_with_footer_display_width_consistency_with_accents() {
        use comfy_table::{Table, presets, ContentArrangement};
        use unicode_width::UnicodeWidthStr;

        let mut table = Table::new();
        table.load_style(presets::UTF8_FULL_CONDENSED);
        table.set_content_arrangement(ContentArrangement::Disabled);
        table.set_header(vec!["ODS Code", "Name", "Postcode"]);
        table.add_row(vec!["H01", "Hôpital Sainte-Thérèse d'Avila", "SW1A 1AA"]);

        let left = "1 active record";
        let right = "Use --all to include inactive";
        let rendered = render_with_footer(&table, left, right);

        let lines: Vec<&str> = rendered.table.lines().collect();
        assert!(!lines.is_empty());
        let expected_width = lines[0].width();

        for (idx, line) in lines.iter().enumerate() {
            assert_eq!(
                line.width(),
                expected_width,
                "Line {} does not match expected display width {}: '{}'",
                idx,
                expected_width,
                line
            );
        }
    }

    #[test]
    fn test_render_with_footer_divider_alignment() {
        use comfy_table::{Table, presets, ContentArrangement};

        let mut table = Table::new();
        table.load_style(presets::UTF8_FULL_CONDENSED);
        table.set_content_arrangement(ContentArrangement::Disabled);
        table.set_header(vec!["Col A", "Column B", "Col C", "Column D"]);
        table.add_row(vec!["123", "Example item", "Test", "More content"]);

        let original_render = table.to_string();
        let orig_bottom = original_render.lines().last().unwrap();
        let orig_divider_indices: Vec<usize> = orig_bottom
            .char_indices()
            .filter_map(|(i, c)| if c == '┴' { Some(i) } else { None })
            .collect();

        let rendered = render_with_footer(&table, "1 record", "1 active · 0 inactive");
        let lines: Vec<&str> = rendered.table.lines().collect();
        // The merge line is 3rd from the bottom (above footer and bottom line)
        let merge_line = lines[lines.len() - 3];
        let merge_divider_indices: Vec<usize> = merge_line
            .char_indices()
            .filter_map(|(i, c)| if c == '┴' { Some(i) } else { None })
            .collect();

        assert_eq!(
            orig_divider_indices, merge_divider_indices,
            "Merge line ┴ dividers do not match original bottom border!"
        );
        assert!(merge_line.starts_with('├'));
        assert!(merge_line.ends_with('┤'));

        let bottom_line = lines.last().unwrap();
        assert!(bottom_line.starts_with('└'));
        assert!(bottom_line.ends_with('┘'));
        assert!(!bottom_line.contains('┴'), "Bottom line must have ┴ replaced by ─");
    }

    #[test]
    fn test_render_with_footer_narrow_overflow() {
        use comfy_table::{Table, presets, ContentArrangement};
        use unicode_width::UnicodeWidthStr;

        let mut table = Table::new();
        table.load_style(presets::UTF8_FULL_CONDENSED);
        table.set_content_arrangement(ContentArrangement::Dynamic);
        table.set_width(40);
        table.set_header(vec!["ODS Code", "Name", "Postcode", "Roles", "Class"]);
        table.add_row(vec![
            "A82608",
            "SEDBERGH MEDICAL PRACTICE",
            "LA10 5DL",
            "GP Practice +1",
            "org",
        ]);

        let left = "6 active records";
        let right = "Use --all to include inactive";
        let rendered = render_with_footer(&table, left, right);

        assert_eq!(
            rendered.overflow_hint,
            Some("Use --all to include inactive".to_string()),
            "Hint should overflow on narrow box"
        );

        let lines: Vec<&str> = rendered.table.lines().collect();
        let expected_width = lines[0].width();
        for (idx, line) in lines.iter().enumerate() {
            assert_eq!(
                line.width(),
                expected_width,
                "Line {} display width mismatch: '{}'",
                idx,
                line
            );
        }
    }
}


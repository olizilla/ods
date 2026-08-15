use anyhow::{Context, Result};
use arrow::array::{Array, Date32Array, StringArray};
use clap::{Parser, ValueEnum};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Table,
    Csv,
    Json,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortBy {
    Code,
    Name,
    Postcode,
}

#[derive(Parser, Debug, Clone)]
pub struct Args {
    /// Query string (searches ODS code, Name, Postcode, Town, Commissioner, Parent, or Role). Omit to launch interactive TUI.
    pub query: Option<String>,

    /// Optional primary role filter
    #[arg(long, short)]
    pub role: Option<String>,

    /// Query the complete historical database (orgs_all.parquet) including inactive/closed entities
    #[arg(long, short)]
    pub all: bool,

    /// Enable detailed OKF Markdown inspector view for matching records
    #[arg(long, short)]
    pub verbose: bool,

    /// Sort order: code, name, postcode
    #[arg(long, short, value_enum, default_value_t = SortBy::Code)]
    pub sort: SortBy,

    /// Output format: table, csv, json
    #[arg(long, short, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,

    /// Input directory containing Parquet files (orgs.parquet, etc.)
    #[arg(long, short, default_value = ".")]
    pub input: PathBuf,
}

#[derive(Debug, Clone)]
struct MatchedRecord {
    ods_code: String,
    name: String,
    entity_type: String,
    status: String,
    /// The ODS primary role *code* (e.g. RO177).
    primary_role_code: String,
    /// Curated display name for `primary_role_code`, resolved from roles.parquet.
    role_name: String,
    /// Every active role code held, including the primary one.
    role_codes: Vec<String>,
    /// What this entity actually is, per data/category_rules.json.
    category: String,
    address: String,
    town: String,
    county: String,
    postcode: String,
    country: String,
    uprn: String,
    telephone: String,
    website: String,
    commissioner_name: String,
    commissioner_code: String,
    parent_name: String,
    parent_code: String,
    pcn_name: String,
    pcn_code: String,
    trust_name: String,
    trust_code: String,
    icb_name: String,
    icb_code: String,
    region_name: String,
    region_code: String,
    operational_start: Option<String>,
    operational_end: Option<String>,
    legal_start: Option<String>,
    legal_end: Option<String>,
    last_changed: Option<String>,
    publication_date: Option<String>,
    is_exact_code_match: bool,
}

struct SuccessorInfo {
    successor_code: Option<String>,
    successor: Option<String>,
    _chain: String,
}

fn load_successors_map(parquet_dir: &Path) -> HashMap<String, Vec<SuccessorInfo>> {
    let path = if parquet_dir.join("successions.parquet").exists() {
        parquet_dir.join("successions.parquet")
    } else {
        parquet_dir.join("successors.parquet")
    };
    let mut map: HashMap<String, Vec<SuccessorInfo>> = HashMap::new();
    if !path.exists() {
        return map;
    }
    let Ok(file) = File::open(&path) else { return map };
    let Ok(builder) = ParquetRecordBatchReaderBuilder::try_new(file) else { return map };
    let Ok(reader) = builder.build() else { return map };

    for batch in reader.flatten() {
        let schema = batch.schema();
        let ods_idx = schema.index_of("predecessor_code").or_else(|_| schema.index_of("ods_code")).ok();
        let succ_code_idx = schema.index_of("successor_code").ok();
        let succ_name_idx = schema.index_of("successor").ok();
        let chain_idx = schema.index_of("succession_chain").ok();

        let Some(ods_idx) = ods_idx else { continue };
        let Some(succ_code_idx) = succ_code_idx else { continue };

        let ods_arr = batch.column(ods_idx).as_any().downcast_ref::<StringArray>();
        let succ_code_arr = batch.column(succ_code_idx).as_any().downcast_ref::<StringArray>();
        let succ_name_arr = succ_name_idx.and_then(|idx| batch.column(idx).as_any().downcast_ref::<StringArray>());
        let chain_arr = chain_idx.and_then(|idx| batch.column(idx).as_any().downcast_ref::<StringArray>());

        let (Some(ods_arr), Some(succ_code_arr)) = (ods_arr, succ_code_arr) else { continue };

        for i in 0..batch.num_rows() {
            let code = ods_arr.value(i).to_string();
            let succ_code = if succ_code_arr.is_valid(i) { Some(succ_code_arr.value(i).to_string()) } else { None };
            let succ_name = succ_name_arr.and_then(|arr| if arr.is_valid(i) { Some(arr.value(i).to_string()) } else { None });
            let chain = chain_arr.map(|arr| arr.value(i).to_string()).unwrap_or_else(|| succ_code.clone().unwrap_or_default());

            map.entry(code).or_default().push(SuccessorInfo {
                successor_code: succ_code,
                successor: succ_name,
                _chain: chain,
            });
        }
    }
    map
}

/// Loads the release's role vocabulary: `role_code` -> curated display name.
///
/// The vocabulary ships alongside the data, so a workspace built by a different
/// `ods` version still renders with the names that release was published with.
/// Falls back to the compiled-in vocabulary when the file is absent.
fn load_role_vocabulary(parquet_dir: &Path) -> HashMap<String, String> {
    let mut map: HashMap<String, String> = HashMap::new();
    let path = parquet_dir.join("roles.parquet");

    if let Ok(file) = File::open(&path) {
        if let Ok(builder) = ParquetRecordBatchReaderBuilder::try_new(file) {
            if let Ok(reader) = builder.build() {
                for batch in reader.flatten() {
                    let schema = batch.schema();
                    let (Ok(code_idx), Ok(name_idx)) =
                        (schema.index_of("role_code"), schema.index_of("role_name")) else { continue };
                    let code_arr = batch.column(code_idx).as_any().downcast_ref::<StringArray>();
                    let name_arr = batch.column(name_idx).as_any().downcast_ref::<StringArray>();
                    let (Some(code_arr), Some(name_arr)) = (code_arr, name_arr) else { continue };
                    for i in 0..batch.num_rows() {
                        map.insert(code_arr.value(i).to_string(), name_arr.value(i).to_string());
                    }
                }
            }
        }
    }

    if map.is_empty() {
        let vocab = crate::roles::role_names();
        for (code, name) in &vocab.names {
            map.insert(code.clone(), name.clone());
        }
    }
    map
}

/// `RO76` -> `GP Practice (RO76)`, falling back to the bare code when the
/// vocabulary does not know it.
fn role_display(vocab: &HashMap<String, String>, code: &str) -> String {
    match vocab.get(code) {
        Some(name) => format!("{name} ({code})"),
        None => code.to_string(),
    }
}

pub fn run(args: Args) -> Result<()> {
    let resolved_input = if args.input == PathBuf::from(".") {
        if let Some(workspace_root) = crate::workspace::find_workspace_root() {
            let active_dir = workspace_root.join("current");
            if active_dir.join("orgs.parquet").exists() || active_dir.join("orgs_all.parquet").exists() {
                active_dir
            } else {
                args.input.clone()
            }
        } else {
            args.input.clone()
        }
    } else {
        args.input.clone()
    };

    if args.query.is_none() {
        let mut tui_args = args;
        tui_args.input = resolved_input;
        return crate::tui::run(tui_args);
    }

    run_with_writer(args, &mut std::io::stdout(), &resolved_input)
}

pub fn run_with_writer(args: Args, writer: &mut dyn std::io::Write, parquet_dir: &Path) -> Result<()> {
    let file_name = if args.all { "orgs_all.parquet" } else { "orgs.parquet" };
    let path = parquet_dir.join(file_name);
    if !path.exists() {
        anyhow::bail!(
            "✖ No dataset found in ods_data/current\n  Run `ods pull` to download the latest pre-built NHS ODS dataset release."
        );
    }

    let file = File::open(&path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let reader = builder.build()?;

    let query_str = args.query.as_deref().unwrap_or_default();
    let query_lower = query_str.to_lowercase();
    let query_words: Vec<String> = query_lower
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();
    let role_filter_lower = args.role.map(|r| r.to_lowercase());

    let successors_map = load_successors_map(parquet_dir);
    let role_vocab = load_role_vocabulary(parquet_dir);

    let mut matches: Vec<MatchedRecord> = Vec::new();

    for batch in reader {
        let batch = batch?;
        let schema = batch.schema();
        let num_rows = batch.num_rows();

        let ods_code_arr = batch.column(schema.index_of("ods_code")?)
            .as_any().downcast_ref::<StringArray>().context("ods_code StringArray")?;
        let name_arr = batch.column(schema.index_of("name")?)
            .as_any().downcast_ref::<StringArray>().context("name StringArray")?;
        let record_class_arr = batch.column(schema.index_of("entity_type")?)
            .as_any().downcast_ref::<StringArray>().context("entity_type StringArray")?;
        let status_arr = batch.column(schema.index_of("status")?)
            .as_any().downcast_ref::<StringArray>().context("status StringArray")?;
        let primary_role_arr = batch.column(schema.index_of("primary_role_code")?)
            .as_any().downcast_ref::<StringArray>().context("primary_role_code StringArray")?;
        let roles_arr = batch.column(schema.index_of("role_codes")?)
            .as_any().downcast_ref::<arrow::array::ListArray>().context("role_codes ListArray")?;
        let category_arr = batch.column(schema.index_of("category")?)
            .as_any().downcast_ref::<StringArray>().context("category StringArray")?;

        let address_idx = schema.index_of("address").ok();
        let town_idx = schema.index_of("town").ok();
        let county_idx = schema.index_of("county").ok();
        let postcode_idx = schema.index_of("postcode").ok();
        let country_idx = schema.index_of("country").ok();
        let uprn_idx = schema.index_of("uprn").ok();
        let telephone_idx = schema.index_of("telephone").ok();
        let website_idx = schema.index_of("website").ok();
        let commissioner_idx = schema.index_of("commissioner_name").ok();
        let commissioner_code_idx = schema.index_of("commissioner_code").ok();
        let parent_idx = schema.index_of("parent_name").ok();
        let parent_code_idx = schema.index_of("parent_code").ok();
        let pcn_idx = schema.index_of("pcn_name").ok();
        let pcn_code_idx = schema.index_of("pcn_code").ok();
        let trust_idx = schema.index_of("trust_name").ok();
        let trust_code_idx = schema.index_of("trust_code").ok();
        let icb_idx = schema.index_of("icb_name").ok();
        let icb_code_idx = schema.index_of("icb_code").ok();
        let region_idx = schema.index_of("region_name").ok();
        let region_code_idx = schema.index_of("region_code").ok();

        let op_start_idx = schema.index_of("operational_start").ok();
        let op_end_idx = schema.index_of("operational_end").ok();
        let leg_start_idx = schema.index_of("legal_start").ok();
        let leg_end_idx = schema.index_of("legal_end").ok();
        let last_change_idx = schema.index_of("last_changed").ok();
        let pub_date_idx = schema.index_of("publication_date").ok();

        let extract_date = |batch: &arrow::record_batch::RecordBatch, idx: Option<usize>, row: usize| -> Option<String> {
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
        };

        for i in 0..num_rows {
            let code = ods_code_arr.value(i);
            let name = name_arr.value(i);
            let class = record_class_arr.value(i);
            let status = status_arr.value(i);
            let primary_role = primary_role_arr.value(i);
            let role_name = role_vocab
                .get(primary_role)
                .cloned()
                .unwrap_or_else(|| primary_role.to_string());

            let role_codes: Vec<String> = if roles_arr.is_valid(i) {
                let vals = roles_arr.value(i);
                vals.as_any()
                    .downcast_ref::<StringArray>()
                    .map(|a| (0..a.len()).map(|j| a.value(j).to_string()).collect())
                    .unwrap_or_default()
            } else {
                Vec::new()
            };

            let address = address_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i)) } else { None }
            }).unwrap_or("");

            let town = town_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i)) } else { None }
            }).unwrap_or("");

            let county = county_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i)) } else { None }
            }).unwrap_or("");

            let postcode = postcode_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i)) } else { None }
            }).unwrap_or("");

            let country = country_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i)) } else { None }
            }).unwrap_or("");

            let uprn = uprn_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i)) } else { None }
            }).unwrap_or("");

            let telephone = telephone_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i)) } else { None }
            }).unwrap_or("");

            let website = website_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i)) } else { None }
            }).unwrap_or("");

            let commissioner = commissioner_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i)) } else { None }
            }).unwrap_or("");

            let commissioner_code = commissioner_code_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i)) } else { None }
            }).unwrap_or("");

            let parent = parent_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i)) } else { None }
            }).unwrap_or("");

            let parent_code = parent_code_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i)) } else { None }
            }).unwrap_or("");

            let pcn = pcn_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i)) } else { None }
            }).unwrap_or("");

            let pcn_code = pcn_code_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i)) } else { None }
            }).unwrap_or("");

            let trust = trust_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i)) } else { None }
            }).unwrap_or("");

            let trust_code = trust_code_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i)) } else { None }
            }).unwrap_or("");

            let icb = icb_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i)) } else { None }
            }).unwrap_or("");

            let icb_code = icb_code_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i)) } else { None }
            }).unwrap_or("");

            let region = region_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i)) } else { None }
            }).unwrap_or("");

            let region_code = region_code_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i)) } else { None }
            }).unwrap_or("");

            let op_start = extract_date(&batch, op_start_idx, i);
            let op_end = extract_date(&batch, op_end_idx, i);
            let leg_start = extract_date(&batch, leg_start_idx, i);
            let leg_end = extract_date(&batch, leg_end_idx, i);
            let last_change_date = extract_date(&batch, last_change_idx, i);
            let pub_date = extract_date(&batch, pub_date_idx, i);

            let is_exact_code = code.to_lowercase() == query_lower;

            // Every role the entity holds, as a display name, so a query for
            // "gp practice" reaches practices whose *primary* role is
            // "prescribing cost centre".
            let role_names_lower: Vec<String> = role_codes
                .iter()
                .map(|c| role_vocab.get(c).cloned().unwrap_or_else(|| c.clone()).to_lowercase())
                .collect();

            let matched = is_exact_code || query_words.iter().all(|word| {
                code.to_lowercase().starts_with(word)
                || name.to_lowercase().contains(word)
                || postcode.to_lowercase().replace(' ', "").contains(word)
                || town.to_lowercase().contains(word)
                || commissioner.to_lowercase().contains(word)
                || parent.to_lowercase().contains(word)
                || role_names_lower.iter().any(|r| r.contains(word))
            });

            if matched {
                // --role accepts either a curated name fragment ("gp practice")
                // or an RO code ("RO76"), matched against every role held.
                if let Some(ref role_filter) = role_filter_lower {
                    let matches_name = role_names_lower.iter().any(|r| r.contains(role_filter));
                    let matches_code = role_codes
                        .iter()
                        .any(|c| c.eq_ignore_ascii_case(role_filter));
                    if !matches_name && !matches_code {
                        continue;
                    }
                }

                matches.push(MatchedRecord {
                    ods_code: code.to_string(),
                    name: name.to_string(),
                    entity_type: class.to_string(),
                    status: status.to_string(),
                    primary_role_code: primary_role.to_string(),
                    role_name: role_name.clone(),
                    role_codes: role_codes.clone(),
                    category: category_arr.value(i).to_string(),
                    address: address.to_string(),
                    town: town.to_string(),
                    county: county.to_string(),
                    postcode: postcode.to_string(),
                    country: country.to_string(),
                    uprn: uprn.to_string(),
                    telephone: telephone.to_string(),
                    website: website.to_string(),
                    commissioner_name: commissioner.to_string(),
                    commissioner_code: commissioner_code.to_string(),
                    parent_name: parent.to_string(),
                    parent_code: parent_code.to_string(),
                    pcn_name: pcn.to_string(),
                    pcn_code: pcn_code.to_string(),
                    trust_name: trust.to_string(),
                    trust_code: trust_code.to_string(),
                    icb_name: icb.to_string(),
                    icb_code: icb_code.to_string(),
                    region_name: region.to_string(),
                    region_code: region_code.to_string(),
                    operational_start: op_start,
                    operational_end: op_end,
                    legal_start: leg_start,
                    legal_end: leg_end,
                    last_changed: last_change_date,
                    publication_date: pub_date,
                    is_exact_code_match: is_exact_code,
                });
            }
        }
    }

    // Sort exact ODS code matches to top, then apply user-requested sort
    matches.sort_by(|a, b| {
        if b.is_exact_code_match != a.is_exact_code_match {
            return b.is_exact_code_match.cmp(&a.is_exact_code_match);
        }
        match args.sort {
            SortBy::Code => a.ods_code.cmp(&b.ods_code),
            SortBy::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortBy::Postcode => a.postcode.to_lowercase().cmp(&b.postcode.to_lowercase()),
        }
    });

    let matched_count = matches.len();

    // Render output
    match args.format {
        OutputFormat::Json => {
            for r in &matches {
                let (succ_code, succ_name) = if r.status.eq_ignore_ascii_case("inactive") {
                    successors_map.get(&r.ods_code)
                        .and_then(|succs| succs.iter().find_map(|s| {
                            s.successor_code.clone().map(|c| (Some(c), s.successor.clone()))
                        }))
                        .unwrap_or((None, None))
                } else {
                    (None, None)
                };

                let json = serde_json::json!({
                    "ods_code": r.ods_code,
                    "entity_type": r.entity_type,
                    "status": r.status,
                    "primary_role_code": r.primary_role_code,
                    "role_codes": r.role_codes,
                    "category": r.category,
                    "role_name": r.role_name,
                    "name": r.name,
                    "address": if r.address.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(r.address.clone()) },
                    "town": if r.town.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(r.town.clone()) },
                    "county": if r.county.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(r.county.clone()) },
                    "postcode": if r.postcode.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(r.postcode.clone()) },
                    "country": if r.country.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(r.country.clone()) },
                    "uprn": if r.uprn.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(r.uprn.clone()) },
                    "telephone": if r.telephone.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(r.telephone.clone()) },
                    "website": if r.website.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(r.website.clone()) },
                    "commissioner_name": if r.commissioner_name.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(r.commissioner_name.clone()) },
                    "commissioner_code": if r.commissioner_code.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(r.commissioner_code.clone()) },
                    "parent_name": if r.parent_name.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(r.parent_name.clone()) },
                    "parent_code": if r.parent_code.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(r.parent_code.clone()) },
                    "pcn_name": if r.pcn_name.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(r.pcn_name.clone()) },
                    "pcn_code": if r.pcn_code.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(r.pcn_code.clone()) },
                    "trust_name": if r.trust_name.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(r.trust_name.clone()) },
                    "trust_code": if r.trust_code.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(r.trust_code.clone()) },
                    "icb_name": if r.icb_name.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(r.icb_name.clone()) },
                    "icb_code": if r.icb_code.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(r.icb_code.clone()) },
                    "region_name": if r.region_name.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(r.region_name.clone()) },
                    "region_code": if r.region_code.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(r.region_code.clone()) },
                    "successor_code": succ_code,
                    "successor": succ_name,
                    "legal_start": r.legal_start,
                    "legal_end": r.legal_end,
                    "operational_start": r.operational_start,
                    "operational_end": r.operational_end,
                    "last_changed": r.last_changed,
                    "publication_date": r.publication_date,
                });
                writeln!(writer, "{}", json)?;
            }
        }
        OutputFormat::Csv => {
            writeln!(writer, "ods_code,entity_type,status,primary_role_code,role_codes,category,role_name,name,address,town,county,postcode,country,uprn,telephone,website,commissioner_name,commissioner_code,parent_name,parent_code,pcn_name,pcn_code,trust_name,trust_code,icb_name,icb_code,region_name,region_code,successor_code,successor,legal_start,legal_end,operational_start,operational_end,last_changed,publication_date")?;
            let escape_csv = |s: &str| -> String {
                if s.contains(',') || s.contains('"') || s.contains('\n') {
                    format!("\"{}\"", s.replace('"', "\"\""))
                } else {
                    s.to_string()
                }
            };
            for r in &matches {
                // List columns flatten to a semicolon-separated field in CSV.
                let roles_str = r.role_codes.join("; ");
                let (succ_code, succ_name) = if r.status.eq_ignore_ascii_case("inactive") {
                    successors_map.get(&r.ods_code)
                        .and_then(|succs| succs.iter().find_map(|s| {
                            s.successor_code.clone().map(|c| (c, s.successor.clone().unwrap_or_default()))
                        }))
                        .unwrap_or_default()
                } else {
                    (String::new(), String::new())
                };

                let fields = [
                    r.ods_code.as_str(),
                    r.entity_type.as_str(),
                    r.status.as_str(),
                    r.primary_role_code.as_str(),
                    &escape_csv(&roles_str),
                    &escape_csv(&r.category),
                    &escape_csv(&r.role_name),
                    &escape_csv(&r.name),
                    &escape_csv(&r.address),
                    &escape_csv(&r.town),
                    &escape_csv(&r.county),
                    r.postcode.as_str(),
                    &escape_csv(&r.country),
                    r.uprn.as_str(),
                    r.telephone.as_str(),
                    &escape_csv(&r.website),
                    &escape_csv(&r.commissioner_name),
                    r.commissioner_code.as_str(),
                    &escape_csv(&r.parent_name),
                    r.parent_code.as_str(),
                    &escape_csv(&r.pcn_name),
                    r.pcn_code.as_str(),
                    &escape_csv(&r.trust_name),
                    r.trust_code.as_str(),
                    &escape_csv(&r.icb_name),
                    r.icb_code.as_str(),
                    &escape_csv(&r.region_name),
                    r.region_code.as_str(),
                    succ_code.as_str(),
                    &escape_csv(&succ_name),
                    r.legal_start.as_deref().unwrap_or(""),
                    r.legal_end.as_deref().unwrap_or(""),
                    r.operational_start.as_deref().unwrap_or(""),
                    r.operational_end.as_deref().unwrap_or(""),
                    r.last_changed.as_deref().unwrap_or(""),
                    r.publication_date.as_deref().unwrap_or(""),
                ];
                writeln!(writer, "{}", fields.join(","))?;
            }
        }
        OutputFormat::Table => {
            if args.verbose || matched_count == 1 {
                use std::io::IsTerminal;
                let use_color = std::io::stdout().is_terminal();

                // Shared Markdown inspector view
                for r in &matches {
                    // Roles other than the primary one, rendered as
                    // "GP Practice (RO76)".
                    // Roles other than the primary one, rendered as
                    // "GP Practice (RO76)".
                    let sec_roles: Vec<String> = r
                        .role_codes
                        .iter()
                        .filter(|c| **c != r.primary_role_code)
                        .map(|c| role_display(&role_vocab, c))
                        .collect();
                    let successors_vec: Vec<crate::formatting::SuccessorLink> = if r.status.eq_ignore_ascii_case("inactive") {
                        successors_map.get(&r.ods_code)
                            .map(|succs| succs.iter().filter_map(|s| {
                                s.successor_code.as_deref().map(|c| crate::formatting::SuccessorLink {
                                    successor_code: c,
                                    successor_name: s.successor.as_deref(),
                                })
                            }).collect())
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    };

                    let inspector = crate::formatting::InspectorRecord {
                        ods_code: &r.ods_code,
                        name: &r.name,
                        record_class: &r.entity_type,
                        status: &r.status,
                        role: &r.role_name,
                        role_code: &r.primary_role_code,
                        other_roles: &sec_roles,
                        address: &r.address,
                        country: &r.country,
                        uprn: &r.uprn,
                        telephone: &r.telephone,
                        website: &r.website,
                        commissioner: &r.commissioner_name,
                        commissioner_code: &r.commissioner_code,
                        parent: &r.parent_name,
                        parent_code: &r.parent_code,
                        pcn: &r.pcn_name,
                        pcn_code: &r.pcn_code,
                        trust: &r.trust_name,
                        trust_code: &r.trust_code,
                        icb: &r.icb_name,
                        icb_code: &r.icb_code,
                        region: &r.region_name,
                        region_code: &r.region_code,
                        successors: &successors_vec,
                        operational_start: r.operational_start.as_deref(),
                        operational_end: r.operational_end.as_deref(),
                        legal_start: r.legal_start.as_deref(),
                        legal_end: r.legal_end.as_deref(),
                        last_change_date: r.last_changed.as_deref(),
                    };

                    crate::formatting::render_inspector_markdown(&inspector, use_color, writer)?;
                    writeln!(writer, "\n─────────────────────────────────────────────────────────────────────────────\n")?;
                }
            } else {
                // Padded Markdown Table layout
                if args.all {
                    writeln!(
                        writer,
                        "| {:<10} | {:<45} | {:<9} | {:<25} | {:<5} | {:<8} | {}",
                        "ODS Code", "Name", "Postcode", "Category", "Class", "Status", "Successor"
                    )?;
                    writeln!(
                        writer,
                        "|{:-<12}|{:-<47}|{:-<11}|{:-<27}|{:-<7}|{:-<10}|{:-<9}",
                        "", "", "", "", "", "", ""
                    )?;
                } else {
                    writeln!(
                        writer,
                        "| {:<10} | {:<45} | {:<9} | {:<25} | {}",
                        "ODS Code", "Name", "Postcode", "Category", "Class"
                    )?;
                    writeln!(
                        writer,
                        "|{:-<12}|{:-<47}|{:-<11}|{:-<27}|{:-<5}",
                        "", "", "", "", ""
                    )?;
                }

                for r in &matches {
                    let name_truncated = if r.name.len() > 45 { &r.name[..42] } else { &r.name };
                    let name_display = if r.name.len() > 45 { format!("{}...", name_truncated) } else { r.name.to_string() };
                    let role_truncated = if r.category.len() > 25 { &r.category[..22] } else { &r.category };
                    let role_display = if r.category.len() > 25 { format!("{}...", role_truncated) } else { r.category.to_string() };

                    if args.all {
                        let succ_display = if r.status.eq_ignore_ascii_case("inactive") {
                            successors_map.get(&r.ods_code)
                                .and_then(|succs| succs.iter().find_map(|s| s.successor_code.clone()))
                                .unwrap_or_else(|| "".to_string())
                        } else {
                            "".to_string()
                        };

                        writeln!(
                            writer,
                            "| {:<10} | {:<45} | {:<9} | {:<25} | {:<5} | {:<8} | {}",
                            r.ods_code, name_display, r.postcode, role_display, r.entity_type, r.status, succ_display
                        )?;
                    } else {
                        writeln!(
                            writer,
                            "| {:<10} | {:<45} | {:<9} | {:<25} | {}",
                            r.ods_code, name_display, r.postcode, role_display, r.entity_type
                        )?;
                    }
                }
            }

            if args.all {
                writeln!(
                    writer,
                    "\nFound {} matching records in '{}' (including inactive/closed history).",
                    matched_count, file_name
                )?;
            } else {
                writeln!(
                    writer,
                    "\nFound {} matching active records in '{}'. Pass --all to include inactive/closed history.",
                    matched_count, file_name
                )?;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_load_role_vocabulary_falls_back_to_compiled_in() {
        let dir = tempdir().unwrap();
        // No roles.parquet present: fall back to the vocabulary compiled
        // into the binary rather than rendering bare RO codes.
        let vocab = load_role_vocabulary(dir.path());
        assert_eq!(vocab.get("RO76").map(|s| s.as_str()), Some("GP Practice"));
    }

    #[test]
    fn test_load_successors_map_nonexistent() {
        let dir = tempdir().unwrap();
        let map = load_successors_map(dir.path());
        assert!(map.is_empty());
    }

    fn setup_synthetic_parquet() -> (tempfile::TempDir, PathBuf) {
        let dir = tempdir().unwrap();
        let parquet_dir = dir.path().to_path_buf();

        // Create synthetic OdsRecord list
        let records = vec![
            crate::commands::ndjson::OdsRecord {
                ods_code: "A101".to_string(),
                name: "Alpha Health Centre".to_string(),
                status: "active".to_string(),
                role: "prescribing cost centre".to_string(),
                record_class: "org".to_string(),
                geo_loc: Some(crate::commands::ndjson::Location {
                    address_lines: vec!["1 Main Street".to_string()],
                    town: Some("London".to_string()),
                    county: Some("Greater London".to_string()),
                    postcode: Some("SW1A 1AA".to_string()),
                    country: Some("ENGLAND".to_string()),
                    uprn: Some("10001".to_string()),
                }),
                roles: vec![
                    crate::commands::ndjson::OdsRole {
                        id: "RO177".to_string(),
                        code: None,
                        display_name: Some("prescribing cost centre".to_string()),
                        unique_role_id: "1".to_string(),
                        primary_role: true,
                        status: "active".to_string(),
                        dates: vec![],
                    },
                    crate::commands::ndjson::OdsRole {
                        id: "RO76".to_string(),
                        code: None,
                        display_name: Some("gp practice".to_string()),
                        unique_role_id: "2".to_string(),
                        primary_role: false,
                        status: "active".to_string(),
                        dates: vec![],
                    },
                ],
                commissioner: Some("NHS LONDON ICB".to_string()),
                commissioner_code: Some("00A".to_string()),
                operational_start: Some("2020-01-01".to_string()),
                legal_start: None,
                legal_end: None,
                operational_end: None,
                ..Default::default()
            },
            crate::commands::ndjson::OdsRecord {
                ods_code: "B202".to_string(),
                name: "Beta Surgery".to_string(),
                status: "inactive".to_string(),
                role: "branch surgery".to_string(),
                record_class: "site".to_string(),
                geo_loc: Some(crate::commands::ndjson::Location {
                    address_lines: vec!["2 High Street".to_string()],
                    town: Some("Manchester".to_string()),
                    county: None,
                    postcode: Some("M1 1AA".to_string()),
                    country: Some("ENGLAND".to_string()),
                    uprn: None,
                }),
                successors: vec![
                    crate::commands::ndjson::OdsSuccessor {
                        unique_succ_id: "1".to_string(),
                        succ_type: "Successor".to_string(),
                        dates: vec![],
                        target: crate::commands::ndjson::OdsRelationshipTarget {
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
        prov.primary_role_scope = Some(vec![
            "RO177".to_string(),
            "RO261".to_string(),
            "RO198".to_string(),
            "RO180".to_string(),
            "RO76".to_string(),
            "RO318".to_string(),
            "RO7".to_string(),
            "RO269".to_string(),
            "RO270".to_string(),
        ]);

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
                role: None,
                all: false,
                verbose: false,
                sort: SortBy::Code,
                format: OutputFormat::Table,
                input: parquet_dir.clone(),
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
                role: Some("gp practice".to_string()),
                all: false,
                verbose: false,
                sort: SortBy::Code,
                format: OutputFormat::Table,
                input: parquet_dir.clone(),
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
                role: None,
                all: true,
                verbose: false,
                sort: SortBy::Code,
                format: OutputFormat::Table,
                input: parquet_dir.clone(),
            },
            &mut out,
            &parquet_dir,
        ).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("B202"));
        assert!(s.contains("A101")); // Successor code

        // Test 4: --verbose inspector mode
        let mut out = Vec::new();
        run_with_writer(
            Args {
                query: Some("A101".to_string()),
                role: None,
                all: false,
                verbose: true,
                sort: SortBy::Code,
                format: OutputFormat::Table,
                input: parquet_dir.clone(),
            },
            &mut out,
            &parquet_dir,
        ).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("# Alpha Health Centre (A101)"));
        assert!(s.contains("## Contact Details"));
        assert!(s.contains("## Relationships"));
        assert!(s.contains("Other Roles"));
        // Role names now come from the curated vocabulary, not the source's
        // lower-cased display string.
        assert!(s.contains("GP Practice (RO76)"), "expected curated role name\n{s}");

        // Test 5: CSV output
        let mut out = Vec::new();
        run_with_writer(
            Args {
                query: Some("Alpha".to_string()),
                role: None,
                all: false,
                verbose: false,
                sort: SortBy::Code,
                format: OutputFormat::Csv,
                input: parquet_dir.clone(),
            },
            &mut out,
            &parquet_dir,
        ).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.starts_with("ods_code,entity_type,status,primary_role_code,role_codes,category,role_name,name,address,town,county,postcode,country,uprn,telephone,website,commissioner_name,commissioner_code,parent_name,parent_code,pcn_name,pcn_code,trust_name,trust_code,icb_name,icb_code,region_name,region_code,successor_code,successor,legal_start,legal_end,operational_start,operational_end,last_changed,publication_date"));
        assert!(s.contains("A101,org,active"));

        // Test 6: JSON output
        let mut out = Vec::new();
        run_with_writer(
            Args {
                query: Some("Alpha".to_string()),
                role: None,
                all: false,
                verbose: false,
                sort: SortBy::Code,
                format: OutputFormat::Json,
                input: parquet_dir.clone(),
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
        let parquet_schema = crate::commands::parquet::orgs_schema();
        let parquet_fields: std::collections::HashSet<String> = parquet_schema
            .fields()
            .iter()
            .map(|f| f.name().clone())
            .collect();

        // List of fields from orgs.parquet intentionally excluded from find --format json (until Task 10 find rework)
        let ignored_fields: std::collections::HashSet<&str> =
            ["successor_codes", "predecessor_codes"].into_iter().collect();

        // Additional enriching fields resolved from the role vocabulary and
        // the successor chain rather than read straight from orgs.parquet.
        let enriching_fields: std::collections::HashSet<&str> =
            ["role_name", "successor_code", "successor"].into_iter().collect();

        // Construct a synthetic MatchedRecord with all fields populated
        let record = MatchedRecord {
            ods_code: "TEST1".to_string(),
            name: "Test Org".to_string(),
            entity_type: "org".to_string(),
            status: "active".to_string(),
            primary_role_code: "RO177".to_string(),
            role_name: "Prescribing Cost Centre".to_string(),
            role_codes: vec!["RO76".to_string(), "RO177".to_string()],
            category: "GP Practice".to_string(),
            address: "1 Main St".to_string(),
            town: "Town".to_string(),
            county: "County".to_string(),
            postcode: "SW1A 1AA".to_string(),
            country: "ENGLAND".to_string(),
            uprn: "100".to_string(),
            telephone: "0123".to_string(),
            website: "http://test".to_string(),
            commissioner_name: "Comm".to_string(),
            commissioner_code: "C1".to_string(),
            parent_name: "Parent".to_string(),
            parent_code: "P1".to_string(),
            pcn_name: "PCN".to_string(),
            pcn_code: "PCN1".to_string(),
            trust_name: "Trust".to_string(),
            trust_code: "T1".to_string(),
            icb_name: "ICB".to_string(),
            icb_code: "I1".to_string(),
            region_name: "Region".to_string(),
            region_code: "R1".to_string(),
            operational_start: Some("2020-01-01".to_string()),
            operational_end: None,
            legal_start: Some("2020-01-01".to_string()),
            legal_end: None,
            last_changed: Some("2023-01-01".to_string()),
            publication_date: Some("2026-07-28".to_string()),
            is_exact_code_match: true,
        };

        let _secondary_roles = vec!["gp practice (RO76)".to_string()];
        let succ_code = Some("SUCC1".to_string());
        let succ_name = Some("Successor Org".to_string());

        let json_val = serde_json::json!({
            "ods_code": record.ods_code,
            "entity_type": record.entity_type,
            "status": record.status,
            "primary_role_code": record.primary_role_code,
            "role_codes": record.role_codes,
            "category": record.category,
            "role_name": record.role_name,
            "name": record.name,
            "address": record.address,
            "town": record.town,
            "county": record.county,
            "postcode": record.postcode,
            "country": record.country,
            "uprn": record.uprn,
            "telephone": record.telephone,
            "website": record.website,
            "commissioner_name": record.commissioner_name,
            "commissioner_code": record.commissioner_code,
            "parent_name": record.parent_code,
            "parent_code": record.parent_code,
            "pcn_name": record.pcn_name,
            "pcn_code": record.pcn_code,
            "trust_name": record.trust_name,
            "trust_code": record.trust_code,
            "icb_name": record.icb_name,
            "icb_code": record.icb_code,
            "region_name": record.region_name,
            "region_code": record.region_code,
            "successor_code": succ_code,
            "successor": succ_name,
            "legal_start": record.legal_start,
            "legal_end": record.legal_end,
            "operational_start": record.operational_start,
            "operational_end": record.operational_end,
            "last_changed": record.last_changed,
            "publication_date": record.publication_date,
        });

        let json_keys_vec: Vec<String> = json_val.as_object().unwrap().keys().cloned().collect();
        let json_keys_set: std::collections::HashSet<String> = json_keys_vec.iter().cloned().collect();

        // 1. Assert every field in orgs.parquet (except ignored_fields) is present in JSON output
        for field in &parquet_fields {
            if !ignored_fields.contains(field.as_str()) {
                assert!(
                    json_keys_set.contains(field),
                    "Field '{}' from orgs.parquet schema is missing in find --format json output!",
                    field
                );
            }
        }

        // 2. Assert every enriching field is present in JSON output
        for field in &enriching_fields {
            assert!(
                json_keys_set.contains(*field),
                "Enriching field '{}' is missing in find --format json output!",
                field
            );
        }

        // 3. Assert property ordering matches Parquet schema order with grouped enriching properties
        assert_eq!(json_keys_vec[0], "ods_code");
        assert_eq!(json_keys_vec[1], "entity_type");
        assert_eq!(json_keys_vec[2], "status");
        assert_eq!(json_keys_vec[3], "primary_role_code");
        assert_eq!(json_keys_vec[4], "role_codes");
        assert_eq!(json_keys_vec[5], "category");
        assert_eq!(json_keys_vec[6], "role_name");
        assert_eq!(json_keys_vec[7], "name");
        assert_eq!(json_keys_vec[28], "successor_code");
        assert_eq!(json_keys_vec[29], "successor");
        assert_eq!(json_keys_vec[34], "last_changed");
        assert_eq!(json_keys_vec[35], "publication_date");
    }

    #[test]
    fn test_find_csv_schema_matches_json_schema() {
        let (dir, parquet_dir) = setup_synthetic_parquet();
        let _keep_dir = dir;

        let mut json_out = Vec::new();
        run_with_writer(
            Args {
                query: Some("A101".to_string()),
                role: None,
                all: false,
                verbose: false,
                sort: SortBy::Code,
                format: OutputFormat::Json,
                input: parquet_dir.clone(),
            },
            &mut json_out,
            &parquet_dir,
        ).unwrap();
        let json_str = String::from_utf8(json_out).unwrap();
        let json_val: serde_json::Value = serde_json::from_str(json_str.trim()).unwrap();
        let json_keys: Vec<String> = json_val.as_object().unwrap().keys().cloned().collect();

        let mut csv_out = Vec::new();
        run_with_writer(
            Args {
                query: Some("A101".to_string()),
                role: None,
                all: false,
                verbose: false,
                sort: SortBy::Code,
                format: OutputFormat::Csv,
                input: parquet_dir.clone(),
            },
            &mut csv_out,
            &parquet_dir,
        ).unwrap();
        let csv_str = String::from_utf8(csv_out).unwrap();
        let header_line = csv_str.lines().next().unwrap();
        let csv_headers: Vec<String> = header_line.split(',').map(|s| s.to_string()).collect();

        assert_eq!(
            csv_headers, json_keys,
            "CSV output headers do not match JSON key sequence!"
        );
    }
}

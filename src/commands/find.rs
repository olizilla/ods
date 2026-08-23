use anyhow::{bail, Context, Result};
use arrow::array::{Array, Date32Array, StringArray};
use clap::{Parser, ValueEnum};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Table,
    Markdown,
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
    /// Search query matching organisation name only. Omit to launch interactive TUI.
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

    /// Query the complete historical database (orgs_all.parquet) including inactive/closed entities
    #[arg(long, short)]
    pub all: bool,

    /// Show full role set in stored order without +N de-emphasis
    #[arg(long, short)]
    pub verbose: bool,

    /// Explicit sort order (overrides default relevance ranking)
    #[arg(long, short, value_enum)]
    pub sort: Option<SortBy>,

    /// Output format: table, markdown, csv, json
    #[arg(long, short, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,

    /// Input directory containing Parquet files (orgs.parquet, etc.)
    #[arg(long, short, default_value = ".")]
    pub input: PathBuf,
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
    /// Curated role names positionally aligned with role_codes.
    role_names: Vec<String>,
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
    trud_release_date: Option<String>,
    rank: u8,
    loc_level: Option<LocationLevel>,
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

struct ParsedLocation {
    norm: String,
    norm_postcode: String,
}

struct ParsedRoleFilter {
    codes: Vec<String>,
    names: Vec<String>,
}

pub fn run(args: Args) -> Result<()> {
    let user_input = if args.input == PathBuf::from(".") {
        None
    } else {
        Some(args.input.as_path())
    };
    let resolved_input = crate::workspace::discover_parquet_dir(user_input)?;

    if args.query.is_none() && args.code.is_empty() && args.location.is_none() && args.role.is_empty() {
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
        if let Some(workspace_root) = crate::workspace::find_workspace_root() {
            let releases = crate::workspace::list_releases(&workspace_root).unwrap_or_default();
            if !releases.is_empty() {
                let n = releases.len();
                let count_str = if n == 1 { "1 release".to_string() } else { format!("{} releases", n) };
                let newest_date = &releases[0].date;
                anyhow::bail!(
                    "✖ No active release pinned\n  {} in ods_data/releases/, none active.\n  Pin one:  ods pull {}",
                    count_str,
                    newest_date
                );
            }
        }
        anyhow::bail!(
            "✖ No dataset found in ods_data/current\n  Run `ods pull` to download the latest pre-built NHS ODS dataset release, or `ods make` to compile from source."
        );
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
    let parsed_roles = if !args.role.is_empty() {
        let vocab = crate::roles::role_names();
        let mut codes = Vec::new();
        let mut names = Vec::new();

        for r_input in &args.role {
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

    let successions_graph = load_succession_graph(parquet_dir);
    let org_metadata = load_org_metadata(parquet_dir);

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
        let role_names_arr = batch.column(schema.index_of("role_names")?)
            .as_any().downcast_ref::<arrow::array::ListArray>().context("role_names ListArray")?;

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
        let trud_release_date_idx = schema.index_of("trud_release_date").ok();

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
            let role_codes: Vec<String> = if roles_arr.is_valid(i) {
                let value_arr = roles_arr.value(i);
                let str_arr = value_arr.as_any().downcast_ref::<StringArray>();
                if let Some(str_arr) = str_arr {
                    (0..str_arr.len())
                        .filter(|j| str_arr.is_valid(*j))
                        .map(|j| str_arr.value(j).to_string())
                        .collect()
                } else {
                    vec![primary_role.to_string()]
                }
            } else {
                vec![primary_role.to_string()]
            };

            // Extract all role names
            let role_names: Vec<String> = if role_names_arr.is_valid(i) {
                let value_arr = role_names_arr.value(i);
                let str_arr = value_arr.as_any().downcast_ref::<StringArray>();
                if let Some(str_arr) = str_arr {
                    (0..str_arr.len())
                        .filter(|j| str_arr.is_valid(*j))
                        .map(|j| str_arr.value(j).to_string())
                        .collect()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };

            // Filter by --role if specified
            if let Some(ref rf) = parsed_roles {
                let matches_code = rf.codes.iter().any(|c| role_codes.iter().any(|rc| rc.eq_ignore_ascii_case(c)));
                let matches_name = rf.names.iter().any(|n| role_names.iter().any(|rn| rn.eq_ignore_ascii_case(n)));
                if !matches_code && !matches_name {
                    continue;
                }
            }

            let postcode = postcode_idx.and_then(|idx| {
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
            let country = country_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i)) } else { None }
            }).unwrap_or("");

            // Evaluate location matching level
            let loc_level: Option<LocationLevel> = if let Some(ref loc_info) = parsed_location {
                let norm_country = normalize_for_matching(country);
                let norm_county = normalize_for_matching(county);
                let norm_town = normalize_for_matching(town);
                let norm_post = normalize_postcode(postcode);

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

            let commissioner = commissioner_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i).to_string()) } else { None }
            }).unwrap_or_default();
            let commissioner_code = commissioner_code_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i).to_string()) } else { None }
            }).unwrap_or_default();
            let parent = parent_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i).to_string()) } else { None }
            }).unwrap_or_default();
            let parent_code = parent_code_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i).to_string()) } else { None }
            }).unwrap_or_default();
            let pcn = pcn_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i).to_string()) } else { None }
            }).unwrap_or_default();
            let pcn_code = pcn_code_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i).to_string()) } else { None }
            }).unwrap_or_default();
            let trust = trust_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i)) } else { None }
            }).unwrap_or("");
            let trust_code = trust_code_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i).to_string()) } else { None }
            }).unwrap_or_default();
            let icb = icb_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i).to_string()) } else { None }
            }).unwrap_or_default();
            let icb_code = icb_code_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i).to_string()) } else { None }
            }).unwrap_or_default();
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
            let region = region_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i)) } else { None }
            }).unwrap_or("");
            let region_code = region_code_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i)) } else { None }
            }).unwrap_or("");

            let address = address_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i).to_string()) } else { None }
            }).unwrap_or_default();

            let op_start = extract_date(&batch, op_start_idx, i);
            let op_end = extract_date(&batch, op_end_idx, i);
            let leg_start = extract_date(&batch, leg_start_idx, i);
            let leg_end = extract_date(&batch, leg_end_idx, i);
            let last_change_date = extract_date(&batch, last_change_idx, i);
            let trud_release_date = extract_date(&batch, trud_release_date_idx, i);

            let primary_role_idx = role_codes.iter().position(|c| c == primary_role);
            let role_name = primary_role_idx
                .and_then(|idx| role_names.get(idx))
                .cloned()
                .unwrap_or_else(|| primary_role.to_string());

            matches.push(MatchedRecord {
                ods_code: ods_code.to_string(),
                name: name.to_string(),
                entity_type: record_class_arr.value(i).to_string(),
                status: status_arr.value(i).to_string(),
                primary_role_code: primary_role.to_string(),
                role_codes,
                role_names,
                role_name,
                address,
                town: town.to_string(),
                county: county.to_string(),
                postcode: postcode.to_string(),
                country: country.to_string(),
                uprn: uprn.to_string(),
                telephone: telephone.to_string(),
                website: website.to_string(),
                commissioner_name: commissioner,
                commissioner_code,
                parent_name: parent,
                parent_code,
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
                trud_release_date,
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
                SortBy::Code => a.ods_code.cmp(&b.ods_code),
                SortBy::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                SortBy::Postcode => a.postcode.to_lowercase().cmp(&b.postcode.to_lowercase()),
            }
        } else if args.query.is_some() {
            a.rank.cmp(&b.rank).then_with(|| a.ods_code.cmp(&b.ods_code))
        } else {
            a.ods_code.cmp(&b.ods_code)
        }
    });

    let matched_count = matches.len();

    match args.format {
        OutputFormat::Json => {
            for r in &matches {
                let succ_hops = walk_succession_chain(&r.ods_code, &successions_graph, &org_metadata);
                let pred_hops = get_predecessors(&r.ods_code, &successions_graph, &org_metadata);

                let succ_codes: Vec<String> = if let Some(edges) = successions_graph.forward.get(&r.ods_code) {
                    edges.iter().map(|e| e.target_code.clone()).collect()
                } else {
                    Vec::new()
                };
                let pred_codes: Vec<String> = if let Some(edges) = successions_graph.reverse.get(&r.ods_code) {
                    edges.iter().map(|e| e.target_code.clone()).collect()
                } else {
                    Vec::new()
                };

                let successors_json: Vec<serde_json::Value> = succ_hops.iter().map(|h| serde_json::json!({
                    "code": h.code,
                    "name": h.name,
                    "status": h.status,
                    "date": h.date,
                })).collect();

                let predecessors_json: Vec<serde_json::Value> = pred_hops.iter().map(|h| serde_json::json!({
                    "code": h.code,
                    "name": h.name,
                    "status": h.status,
                    "date": h.date,
                })).collect();

                let json = serde_json::json!({
                    "ods_code": r.ods_code,
                    "entity_type": r.entity_type,
                    "status": r.status,
                    "primary_role_code": r.primary_role_code,
                    "role_codes": r.role_codes,
                    "role_names": r.role_names,
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
                    "successor_codes": succ_codes,
                    "predecessor_codes": pred_codes,
                    "successors": successors_json,
                    "predecessors": predecessors_json,
                    "legal_start": r.legal_start,
                    "legal_end": r.legal_end,
                    "operational_start": r.operational_start,
                    "operational_end": r.operational_end,
                    "last_changed": r.last_changed,
                    "trud_release_date": r.trud_release_date,
                });
                writeln!(writer, "{}", json)?;
            }
        }
        OutputFormat::Csv => {
            writeln!(writer, "ods_code,entity_type,status,primary_role_code,role_codes,role_names,role_name,name,address,town,county,postcode,country,uprn,telephone,website,commissioner_name,commissioner_code,parent_name,parent_code,pcn_name,pcn_code,trust_name,trust_code,icb_name,icb_code,region_name,region_code,successor_codes,predecessor_codes,successors,predecessors,legal_start,legal_end,operational_start,operational_end,last_changed,trud_release_date")?;
            let escape_csv = |s: &str| -> String {
                if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains(';') {
                    format!("\"{}\"", s.replace('"', "\"\""))
                } else {
                    s.to_string()
                }
            };
            for r in &matches {
                let roles_str = r.role_codes.join("; ");
                let role_names_str = r.role_names.join("; ");
                let succ_codes: Vec<String> = if let Some(edges) = successions_graph.forward.get(&r.ods_code) {
                    edges.iter().map(|e| e.target_code.clone()).collect()
                } else {
                    Vec::new()
                };
                let pred_codes: Vec<String> = if let Some(edges) = successions_graph.reverse.get(&r.ods_code) {
                    edges.iter().map(|e| e.target_code.clone()).collect()
                } else {
                    Vec::new()
                };
                let succ_codes_str = succ_codes.join("; ");
                let pred_codes_str = pred_codes.join("; ");

                let fields = [
                    r.ods_code.as_str(),
                    r.entity_type.as_str(),
                    r.status.as_str(),
                    r.primary_role_code.as_str(),
                    &escape_csv(&roles_str),
                    &escape_csv(&role_names_str),
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
                    &escape_csv(&succ_codes_str),
                    &escape_csv(&pred_codes_str),
                    &escape_csv(&succ_codes_str),
                    &escape_csv(&pred_codes_str),
                    r.legal_start.as_deref().unwrap_or(""),
                    r.legal_end.as_deref().unwrap_or(""),
                    r.operational_start.as_deref().unwrap_or(""),
                    r.operational_end.as_deref().unwrap_or(""),
                    r.last_changed.as_deref().unwrap_or(""),
                    r.trud_release_date.as_deref().unwrap_or(""),
                ];
                writeln!(writer, "{}", fields.join(","))?;
            }
        }
        OutputFormat::Table | OutputFormat::Markdown => {
            use std::io::IsTerminal;
            use comfy_table::{Table, ContentArrangement, presets};

            let mut table = Table::new();

            if args.format == OutputFormat::Markdown {
                table.load_preset(presets::ASCII_MARKDOWN);
                table.set_content_arrangement(ContentArrangement::Disabled);
            } else {
                table.load_preset(presets::UTF8_FULL_CONDENSED);
                table.set_content_arrangement(ContentArrangement::Dynamic);
                if std::io::stdout().is_terminal() {
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

            for r in &matches {
                let full_name = if args.all && r.status.eq_ignore_ascii_case("inactive") {
                    let succ_hops = walk_succession_chain(&r.ods_code, &successions_graph, &org_metadata);
                    if let Some(live_succ) = resolve_table_successor_display(&succ_hops) {
                        format!("{} → {}", r.name, live_succ)
                    } else {
                        r.name.clone()
                    }
                } else {
                    r.name.clone()
                };

                let role_display = crate::roles::format_roles_for_display(&r.role_codes, &r.role_names, args.verbose);

                if args.all {
                    table.add_row(vec![
                        &r.ods_code,
                        &full_name,
                        &r.postcode,
                        &role_display,
                        &r.entity_type,
                        &r.status,
                    ]);
                } else {
                    table.add_row(vec![
                        &r.ods_code,
                        &full_name,
                        &r.postcode,
                        &role_display,
                        &r.entity_type,
                    ]);
                }
            }

            writeln!(writer, "{}", table)?;

            if let Some(lvl) = matched_loc_level {
                writeln!(
                    writer,
                    "\n* matched {} — {} organisations",
                    lvl.name(),
                    matched_count
                )?;
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

    // Print ODS code hint if query happens to be a valid ODS code in the dataset
    if args.format == OutputFormat::Table || args.format == OutputFormat::Markdown {
        if let Some(ref q) = args.query {
            let q_clean = q.trim().to_uppercase();
            if org_metadata.contains_key(&q_clean) {
                writeln!(writer, "\n* '{}' is also an ODS code — ods info {}", q_clean, q_clean)?;
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
                code: Vec::new(),
                location: None,
                role: Vec::new(),
                all: false,
                verbose: false,
                sort: Some(SortBy::Code),
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
                code: Vec::new(),
                location: None,
                role: vec!["gp practice".to_string()],
                all: false,
                verbose: false,
                sort: Some(SortBy::Code),
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
                code: Vec::new(),
                location: None,
                role: Vec::new(),
                all: true,
                verbose: false,
                sort: Some(SortBy::Code),
                format: OutputFormat::Table,
                input: parquet_dir.clone(),
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
                location: None,
                role: Vec::new(),
                all: false,
                verbose: false,
                sort: Some(SortBy::Code),
                format: OutputFormat::Table,
                input: parquet_dir.clone(),
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
            },
            &mut info_out,
            &parquet_dir,
        ).unwrap();
        let info_s = String::from_utf8(info_out).unwrap();
        assert!(info_s.contains("# Alpha Health Centre (A101)"));
        assert!(info_s.contains("## Contact Details"));
        assert!(info_s.contains("## Relationships"));
        assert!(info_s.contains("Other Roles"));
        assert!(info_s.contains("GP Practice (RO76)"), "expected curated role name\n{info_s}");

        // Test 5: CSV output
        let mut out = Vec::new();
        run_with_writer(
            Args {
                query: Some("Alpha".to_string()),
                code: Vec::new(),
                location: None,
                role: Vec::new(),
                all: false,
                verbose: false,
                sort: Some(SortBy::Code),
                format: OutputFormat::Csv,
                input: parquet_dir.clone(),
            },
            &mut out,
            &parquet_dir,
        ).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.starts_with("ods_code,entity_type,status,primary_role_code,role_codes,role_names,role_name,name,address,town,county,postcode,country,uprn,telephone,website,commissioner_name,commissioner_code,parent_name,parent_code,pcn_name,pcn_code,trust_name,trust_code,icb_name,icb_code,region_name,region_code,successor_codes,predecessor_codes,successors,predecessors,legal_start,legal_end,operational_start,operational_end,last_changed,trud_release_date"));
        assert!(s.contains("A101,org,active"));

        // Test 6: JSON output
        let mut out = Vec::new();
        run_with_writer(
            Args {
                query: Some("Alpha".to_string()),
                code: Vec::new(),
                location: None,
                role: Vec::new(),
                all: false,
                verbose: false,
                sort: Some(SortBy::Code),
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

        let ignored_fields: std::collections::HashSet<&str> =
            ["successor_codes", "predecessor_codes"].into_iter().collect();

        let enriching_fields: std::collections::HashSet<&str> =
            ["role_name", "successor_codes", "predecessor_codes", "successors", "predecessors"].into_iter().collect();

        // Construct a synthetic MatchedRecord with all fields populated
        let record = MatchedRecord {
            ods_code: "TEST1".to_string(),
            name: "Test Org".to_string(),
            entity_type: "org".to_string(),
            status: "active".to_string(),
            primary_role_code: "RO177".to_string(),
            role_name: "Prescribing Cost Centre".to_string(),
            role_codes: vec!["RO76".to_string(), "RO177".to_string()],
            role_names: vec!["GP Practice".to_string(), "Prescribing Cost Centre".to_string()],
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
            trud_release_date: Some("2026-07-31".to_string()),
            rank: 1,
            loc_level: None,
        };

        let json_val = serde_json::json!({
            "ods_code": record.ods_code,
            "entity_type": record.entity_type,
            "status": record.status,
            "primary_role_code": record.primary_role_code,
            "role_codes": record.role_codes,
            "role_names": record.role_names,
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
            "successor_codes": vec!["SUCC1".to_string()],
            "predecessor_codes": Vec::<String>::new(),
            "successors": vec![serde_json::json!({"code": "SUCC1", "name": "Successor Org", "status": "active", "date": null})],
            "predecessors": Vec::<serde_json::Value>::new(),
            "legal_start": record.legal_start,
            "legal_end": record.legal_end,
            "operational_start": record.operational_start,
            "operational_end": record.operational_end,
            "last_changed": record.last_changed,
            "trud_release_date": record.trud_release_date,
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
        assert_eq!(json_keys_vec[5], "role_names");
        assert_eq!(json_keys_vec[6], "role_name");
        assert_eq!(json_keys_vec[7], "name");
        assert_eq!(json_keys_vec[28], "successor_codes");
        assert_eq!(json_keys_vec[29], "predecessor_codes");
        assert_eq!(json_keys_vec[30], "successors");
        assert_eq!(json_keys_vec[31], "predecessors");
        assert_eq!(json_keys_vec[36], "last_changed");
        assert_eq!(json_keys_vec[37], "trud_release_date");
    }

    #[test]
    fn test_find_csv_schema_matches_json_schema() {
        let (dir, parquet_dir) = setup_synthetic_parquet();
        let _keep_dir = dir;

        let mut json_out = Vec::new();
        run_with_writer(
            Args {
                query: Some("Alpha".to_string()),
                code: Vec::new(),
                location: None,
                role: Vec::new(),
                all: false,
                verbose: false,
                sort: Some(SortBy::Code),
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
                query: Some("Alpha".to_string()),
                code: Vec::new(),
                location: None,
                role: Vec::new(),
                all: false,
                verbose: false,
                sort: Some(SortBy::Code),
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


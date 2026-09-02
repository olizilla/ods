use anyhow::{Context, Result};
use arrow::array::{Array, Date32Array, ListArray, StringArray};
use clap::{Args as ClapArgs, ValueEnum};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde::Serialize;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputFormat {
    Markdown,
    Json,
}

#[derive(ClapArgs, Debug, Clone)]
pub struct Args {
    /// Exact ODS organisation code (e.g. A82608, RJZ, 0AF)
    pub ods_code: String,

    /// Output format (markdown for human-readable card, json for structured record)
    #[arg(short, long, value_enum, default_value = "markdown")]
    pub format: OutputFormat,

    /// Directory containing Parquet files (defaults to active release)
    #[arg(short, long)]
    pub input: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SuccessionHopJson {
    pub depth: usize,
    pub date: Option<String>,
    pub code: String,
    pub name: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HierarchyEntityJson {
    pub code: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RelationshipItemJson {
    pub rel_code: String,
    pub rel_name: String,
    pub direction: String,
    pub code: String,
    pub name: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operational_start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operational_end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legal_start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legal_end: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InfoRecordJson {
    pub ods_code: String,
    pub name: String,
    pub record_class: String,
    pub status: String,
    pub primary_role_code: String,
    pub primary_role_name: String,
    pub role_codes: Vec<String>,
    pub role_names: Vec<String>,
    pub other_roles: Vec<String>,
    pub address: String,
    pub town: Option<String>,
    pub county: Option<String>,
    pub postcode: Option<String>,
    pub country: Option<String>,
    pub uprn: Option<String>,
    pub telephone: Option<String>,
    pub website: Option<String>,
    pub relationships: Vec<RelationshipItemJson>,
    pub succession: Vec<SuccessionHopJson>,
    pub predecessors: Vec<SuccessionHopJson>,
    pub legal_start: Option<String>,
    pub legal_end: Option<String>,
    pub operational_start: Option<String>,
    pub operational_end: Option<String>,
    pub last_changed: Option<String>,
    pub trud_release_date: String,
}

pub fn run(args: Args) -> Result<()> {
    use std::io::IsTerminal;
    let use_color = std::io::stdout().is_terminal();
    let resolved_input = crate::workspace::discover_parquet_dir(args.input.as_deref())?;
    if let Err(e) = run_with_writer_color(args, &mut std::io::stdout(), &resolved_input, use_color) {
        if let Some(io_err) = e.downcast_ref::<std::io::Error>() {
            if io_err.kind() == std::io::ErrorKind::BrokenPipe {
                return Ok(());
            }
        }
        return Err(e);
    }
    Ok(())
}

pub fn run_with_writer<W: Write + ?Sized>(
    args: Args,
    writer: &mut W,
    parquet_dir: &Path,
) -> Result<()> {
    run_with_writer_color(args, writer, parquet_dir, false)
}

pub fn run_with_writer_color<W: Write + ?Sized>(
    args: Args,
    writer: &mut W,
    parquet_dir: &Path,
    use_color: bool,
) -> Result<()> {
    let path = if parquet_dir.join("orgs_all.parquet").exists() {
        parquet_dir.join("orgs_all.parquet")
    } else if parquet_dir.join("orgs.parquet").exists() {
        parquet_dir.join("orgs.parquet")
    } else {
        if let Some(workspace_root) = crate::workspace::find_workspace_root(None) {
            let releases = crate::workspace::list_releases(&workspace_root).unwrap_or_default();
            if !releases.is_empty() {
                let n = releases.len();
                let count_str = if n == 1 {
                    "1 release".to_string()
                } else {
                    format!("{} releases", n)
                };
                let newest_date = &releases[0].date;
                let ws_name = workspace_root.file_name().and_then(|n| n.to_str()).unwrap_or(crate::workspace::DEFAULT_WORKSPACE_DIR);
                anyhow::bail!(
                    "✖ No active release pinned\n  {} in {}/releases/, none active.\n  Pin one:  ods use {}",
                    count_str,
                    ws_name,
                    newest_date
                );
            }
        }
        anyhow::bail!(
            "✖ no ods workspace found here\n  Pass -i <trud.zip> -o <dir>, or run `ods pull` to create a workspace."
        );
    };

    let file = File::open(&path).with_context(|| format!("opening {}", path.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let reader = builder.build()?;

    let target_code = args.ods_code.trim();

    struct FoundRecord {
        ods_code: String,
        name: String,
        record_class: String,
        status: String,
        primary_role_code: String,
        role_codes: Vec<String>,
        role_names: Vec<String>,
        address: String,
        town: Option<String>,
        county: Option<String>,
        postcode: Option<String>,
        country: Option<String>,
        uprn: Option<String>,
        telephone: Option<String>,
        website: Option<String>,
        legal_start: Option<String>,
        legal_end: Option<String>,
        operational_start: Option<String>,
        operational_end: Option<String>,
        last_changed: Option<String>,
        trud_release_date: String,
    }

    let mut found: Option<FoundRecord> = None;

    let extract_date = |batch: &arrow::record_batch::RecordBatch,
                        idx: Option<usize>,
                        row: usize|
     -> Option<String> {
        let idx = idx?;
        let arr = batch.column(idx).as_any().downcast_ref::<Date32Array>()?;
        if arr.is_valid(row) {
            let days = arr.value(row);
            let epoch = chrono::NaiveDate::from_ymd_opt(1970, 1, 1)?;
            let d = epoch.checked_add_signed(chrono::Duration::days(days as i64))?;
            Some(d.format("%Y-%m-%d").to_string())
        } else {
            None
        }
    };

    for batch in reader {
        let batch = batch?;
        let schema = batch.schema();
        let num_rows = batch.num_rows();

        let ods_code_arr = batch
            .column(schema.index_of("ods_code")?)
            .as_any()
            .downcast_ref::<StringArray>()
            .context("ods_code StringArray")?;

        let mut match_idx: Option<usize> = None;
        for i in 0..num_rows {
            if ods_code_arr.value(i).eq_ignore_ascii_case(target_code) {
                match_idx = Some(i);
                break;
            }
        }

        if let Some(i) = match_idx {
            let name_arr = batch
                .column(schema.index_of("name")?)
                .as_any()
                .downcast_ref::<StringArray>()
                .context("name StringArray")?;
            let record_class_arr = batch
                .column(schema.index_of("record_class")?)
                .as_any()
                .downcast_ref::<StringArray>()
                .context("record_class StringArray")?;
            let status_arr = batch
                .column(schema.index_of("status")?)
                .as_any()
                .downcast_ref::<StringArray>()
                .context("status StringArray")?;
            let primary_role_arr = batch
                .column(schema.index_of("primary_role_code")?)
                .as_any()
                .downcast_ref::<StringArray>()
                .context("primary_role_code StringArray")?;
            let roles_arr = batch
                .column(schema.index_of("role_codes")?)
                .as_any()
                .downcast_ref::<ListArray>()
                .context("role_codes ListArray")?;
            let role_names_arr = batch
                .column(schema.index_of("role_names")?)
                .as_any()
                .downcast_ref::<ListArray>()
                .context("role_names ListArray")?;

            let address_idx = schema.index_of("address").ok();
            let town_idx = schema.index_of("town").ok();
            let county_idx = schema.index_of("county").ok();
            let postcode_idx = schema.index_of("postcode").ok();
            let country_idx = schema.index_of("country").ok();
            let uprn_idx = schema.index_of("uprn").ok();
            let telephone_idx = schema.index_of("telephone").ok();
            let website_idx = schema.index_of("website").ok();
            let op_start_idx = schema.index_of("operational_start").ok();
            let op_end_idx = schema.index_of("operational_end").ok();
            let leg_start_idx = schema.index_of("legal_start").ok();
            let leg_end_idx = schema.index_of("legal_end").ok();
            let last_change_idx = schema.index_of("last_changed").ok();
            let trud_release_date_idx = schema.index_of("trud_release_date").ok();

            let roles_list_val = roles_arr.value(i);
            let roles_str_arr = roles_list_val
                .as_any()
                .downcast_ref::<StringArray>()
                .context("roles_list StringArray")?;
            let mut role_codes = Vec::with_capacity(roles_str_arr.len());
            for j in 0..roles_str_arr.len() {
                role_codes.push(roles_str_arr.value(j).to_string());
            }

            let role_names_list_val = role_names_arr.value(i);
            let role_names_str_arr = role_names_list_val
                .as_any()
                .downcast_ref::<StringArray>()
                .context("role_names_list StringArray")?;
            let mut role_names = Vec::with_capacity(role_names_str_arr.len());
            for j in 0..role_names_str_arr.len() {
                role_names.push(role_names_str_arr.value(j).to_string());
            }

            let col_opt = |idx: Option<usize>| -> Option<String> {
                let idx = idx?;
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) {
                    let s = arr.value(i).trim();
                    if !s.is_empty() {
                        Some(s.to_string())
                    } else {
                        None
                    }
                } else {
                    None
                }
            };

            let col_str = |idx: Option<usize>| -> String { col_opt(idx).unwrap_or_default() };

            let trud_date = extract_date(&batch, trud_release_date_idx, i).unwrap_or_default();

            found = Some(FoundRecord {
                ods_code: ods_code_arr.value(i).to_string(),
                name: name_arr.value(i).to_string(),
                record_class: record_class_arr.value(i).to_string(),
                status: status_arr.value(i).to_string(),
                primary_role_code: primary_role_arr.value(i).to_string(),
                role_codes,
                role_names,
                address: col_str(address_idx),
                town: col_opt(town_idx),
                county: col_opt(county_idx),
                postcode: col_opt(postcode_idx),
                country: col_opt(country_idx),
                uprn: col_opt(uprn_idx),
                telephone: col_opt(telephone_idx),
                website: col_opt(website_idx),
                legal_start: extract_date(&batch, leg_start_idx, i),
                legal_end: extract_date(&batch, leg_end_idx, i),
                operational_start: extract_date(&batch, op_start_idx, i),
                operational_end: extract_date(&batch, op_end_idx, i),
                last_changed: extract_date(&batch, last_change_idx, i),
                trud_release_date: trud_date,
            });
            break;
        }
    }

    let Some(rec) = found else {
        anyhow::bail!("✖ Organisation '{}' not found", target_code);
    };

    let successions_graph = crate::commands::find::load_succession_graph(parquet_dir);
    let org_metadata = crate::commands::find::load_org_metadata(parquet_dir);

    let succ_hops = crate::commands::find::walk_succession_chain(
        &rec.ods_code,
        &successions_graph,
        &org_metadata,
    );
    let pred_hops =
        crate::commands::find::get_predecessors(&rec.ods_code, &successions_graph, &org_metadata);
    let loaded_rels = load_relationships(parquet_dir, &rec.ods_code, &org_metadata);

    // Primary role name and secondary roles
    let mut primary_role_name = String::new();
    let mut other_roles = Vec::new();

    for (code, name) in rec.role_codes.iter().zip(rec.role_names.iter()) {
        if code == &rec.primary_role_code {
            primary_role_name = name.clone();
        } else {
            other_roles.push(format!("{name} ({code})"));
        }
    }

    match args.format {
        OutputFormat::Json => {
            let succ_json: Vec<SuccessionHopJson> = succ_hops
                .into_iter()
                .map(|h| SuccessionHopJson {
                    depth: h.depth,
                    date: h.date,
                    code: h.code,
                    name: h.name,
                    status: h.status,
                })
                .collect();

            let pred_json: Vec<SuccessionHopJson> = pred_hops
                .into_iter()
                .map(|h| SuccessionHopJson {
                    depth: h.depth,
                    date: h.date,
                    code: h.code,
                    name: h.name,
                    status: h.status,
                })
                .collect();

            let rel_json: Vec<RelationshipItemJson> = loaded_rels
                .into_iter()
                .map(|r| RelationshipItemJson {
                    rel_code: r.rel_code,
                    rel_name: r.rel_name,
                    direction: r.direction,
                    code: r.code,
                    name: r.name,
                    status: r.status,
                    operational_start: r.operational_start,
                    operational_end: r.operational_end,
                    legal_start: r.legal_start,
                    legal_end: r.legal_end,
                })
                .collect();

            let info_json = InfoRecordJson {
                ods_code: rec.ods_code,
                name: rec.name,
                record_class: rec.record_class,
                status: rec.status,
                primary_role_code: rec.primary_role_code,
                primary_role_name,
                role_codes: rec.role_codes,
                role_names: rec.role_names,
                other_roles,
                address: rec.address,
                town: rec.town,
                county: rec.county,
                postcode: rec.postcode,
                country: rec.country,
                uprn: rec.uprn,
                telephone: rec.telephone,
                website: rec.website,
                relationships: rel_json,
                succession: succ_json,
                predecessors: pred_json,
                legal_start: rec.legal_start,
                legal_end: rec.legal_end,
                operational_start: rec.operational_start,
                operational_end: rec.operational_end,
                last_changed: rec.last_changed,
                trud_release_date: rec.trud_release_date,
            };

            writeln!(writer, "{}", serde_json::to_string_pretty(&info_json)?)?;
        }
        OutputFormat::Markdown => {
            let succ_hop_links: Vec<crate::formatting::SuccessionHopLink> = succ_hops
                .iter()
                .map(|h| crate::formatting::SuccessionHopLink {
                    depth: h.depth,
                    date: h.date.as_deref(),
                    code: &h.code,
                    name: &h.name,
                    status: &h.status,
                })
                .collect();

            let pred_hop_links: Vec<crate::formatting::SuccessionHopLink> = pred_hops
                .iter()
                .map(|h| crate::formatting::SuccessionHopLink {
                    depth: h.depth,
                    date: h.date.as_deref(),
                    code: &h.code,
                    name: &h.name,
                    status: &h.status,
                })
                .collect();

            let mut grouped_map: std::collections::BTreeMap<
                (bool, String, String),
                Vec<&LoadedRelationship>,
            > = std::collections::BTreeMap::new();
            for rel in &loaded_rels {
                let is_inbound = rel.direction == "inbound";
                grouped_map
                    .entry((is_inbound, rel.rel_code.clone(), rel.rel_name.clone()))
                    .or_default()
                    .push(rel);
            }

            let rel_groups: Vec<crate::formatting::RelationshipGroup> = grouped_map
                .into_iter()
                .map(|((is_inbound, rel_code, rel_name), items)| {
                    let total_count = items.len();
                    let display_items: Vec<crate::formatting::RelationshipItemView> = items
                        .into_iter()
                        .take(10)
                        .map(|it| crate::formatting::RelationshipItemView {
                            code: &it.code,
                            name: &it.name,
                            status: &it.status,
                            operational_start: it.operational_start.as_deref(),
                            operational_end: it.operational_end.as_deref(),
                            legal_start: it.legal_start.as_deref(),
                            legal_end: it.legal_end.as_deref(),
                        })
                        .collect();

                    crate::formatting::RelationshipGroup {
                        rel_code: Box::leak(rel_code.into_boxed_str()),
                        rel_name: Box::leak(rel_name.into_boxed_str()),
                        is_inbound,
                        items: display_items,
                        total_count,
                    }
                })
                .collect();

            let inspector = crate::formatting::InspectorRecord {
                ods_code: &rec.ods_code,
                name: &rec.name,
                record_class: &rec.record_class,
                status: &rec.status,
                role: &primary_role_name,
                role_code: &rec.primary_role_code,
                other_roles: &other_roles,
                address: &rec.address,
                country: rec.country.as_deref().unwrap_or(""),
                uprn: rec.uprn.as_deref().unwrap_or(""),
                telephone: rec.telephone.as_deref().unwrap_or(""),
                website: rec.website.as_deref().unwrap_or(""),
                relationships: &rel_groups,
                succession: &succ_hop_links,
                predecessors: &pred_hop_links,
                operational_start: rec.operational_start.as_deref(),
                operational_end: rec.operational_end.as_deref(),
                legal_start: rec.legal_start.as_deref(),
                legal_end: rec.legal_end.as_deref(),
                last_change_date: rec.last_changed.as_deref(),
            };

            crate::formatting::render_inspector_markdown(&inspector, use_color, writer)?;
        }
    }

    Ok(())
}

#[derive(Debug, Clone)]
pub struct LoadedRelationship {
    pub rel_code: String,
    pub rel_name: String,
    pub direction: String,
    pub code: String,
    pub name: String,
    pub status: String,
    pub operational_start: Option<String>,
    pub operational_end: Option<String>,
    pub legal_start: Option<String>,
    pub legal_end: Option<String>,
}

pub fn load_relationships(
    parquet_dir: &Path,
    target_code: &str,
    org_meta: &std::collections::HashMap<String, (String, String)>,
) -> Vec<LoadedRelationship> {
    let path = parquet_dir.join("relationships.parquet");
    let Ok(file) = File::open(&path) else {
        return Vec::new();
    };
    let Ok(builder) = ParquetRecordBatchReaderBuilder::try_new(file) else {
        return Vec::new();
    };
    let Ok(reader) = builder.build() else {
        return Vec::new();
    };

    let mut rels = Vec::new();

    for batch in reader.flatten() {
        let schema = batch.schema();
        let Ok(src_idx) = schema.index_of("source_code") else {
            continue;
        };
        let Ok(tgt_idx) = schema.index_of("target_code") else {
            continue;
        };
        let type_idx = schema
            .index_of("rel_code")
            .or_else(|_| schema.index_of("rel_type_code"))
            .ok();
        let name_idx = schema
            .index_of("rel_name")
            .or_else(|_| schema.index_of("rel_type_name"))
            .ok();
        let status_idx = schema.index_of("rel_status").ok();
        let leg_start_idx = schema.index_of("legal_start").ok();
        let leg_end_idx = schema.index_of("legal_end").ok();
        let op_start_idx = schema.index_of("operational_start").ok();
        let op_end_idx = schema.index_of("operational_end").ok();

        let src_arr = batch.column(src_idx).as_any().downcast_ref::<StringArray>();
        let tgt_arr = batch.column(tgt_idx).as_any().downcast_ref::<StringArray>();

        let (Some(src_arr), Some(tgt_arr)) = (src_arr, tgt_arr) else {
            continue;
        };

        let extract_date = |idx: Option<usize>, row: usize| -> Option<String> {
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

        let col_str = |idx: Option<usize>, row: usize| -> String {
            if let Some(idx) = idx {
                if let Some(arr) = batch.column(idx).as_any().downcast_ref::<StringArray>() {
                    if arr.is_valid(row) {
                        return arr.value(row).to_string();
                    }
                }
            }
            String::new()
        };

        for i in 0..batch.num_rows() {
            let is_outbound =
                src_arr.is_valid(i) && src_arr.value(i).eq_ignore_ascii_case(target_code);
            let is_inbound =
                tgt_arr.is_valid(i) && tgt_arr.value(i).eq_ignore_ascii_case(target_code);

            if is_outbound || is_inbound {
                let other_code = if is_outbound {
                    tgt_arr.value(i).to_string()
                } else {
                    src_arr.value(i).to_string()
                };

                let other_name = if let Some((name, _)) = org_meta.get(&other_code) {
                    name.clone()
                } else {
                    String::new()
                };

                let rel_code = col_str(type_idx, i);
                let rel_name = col_str(name_idx, i);
                let status = col_str(status_idx, i);

                rels.push(LoadedRelationship {
                    rel_code,
                    rel_name,
                    direction: if is_outbound {
                        "outbound".to_string()
                    } else {
                        "inbound".to_string()
                    },
                    code: other_code,
                    name: other_name,
                    status: if status.is_empty() {
                        "active".to_string()
                    } else {
                        status
                    },
                    operational_start: extract_date(op_start_idx, i),
                    operational_end: extract_date(op_end_idx, i),
                    legal_start: extract_date(leg_start_idx, i),
                    legal_end: extract_date(leg_end_idx, i),
                });
            }
        }
    }

    rels.sort_by(|a, b| {
        a.direction
            .cmp(&b.direction)
            .reverse()
            .then_with(|| a.rel_name.cmp(&b.rel_name))
            .then_with(|| {
                let a_act = a.status.eq_ignore_ascii_case("active");
                let b_act = b.status.eq_ignore_ascii_case("active");
                b_act.cmp(&a_act)
            })
            .then_with(|| b.operational_start.cmp(&a.operational_start))
            .then_with(|| a.code.cmp(&b.code))
    });

    rels
}

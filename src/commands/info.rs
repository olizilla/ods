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
pub struct InfoRecordJson {
    pub ods_code: String,
    pub name: String,
    pub entity_type: String,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commissioner: Option<HierarchyEntityJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<HierarchyEntityJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pcn: Option<HierarchyEntityJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trust: Option<HierarchyEntityJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icb: Option<HierarchyEntityJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<HierarchyEntityJson>,
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
    let resolved_input = crate::workspace::discover_parquet_dir(args.input.as_deref())?;
    if let Err(e) = run_with_writer(args, &mut std::io::stdout(), &resolved_input) {
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
    let path = if parquet_dir.join("orgs_all.parquet").exists() {
        parquet_dir.join("orgs_all.parquet")
    } else if parquet_dir.join("orgs.parquet").exists() {
        parquet_dir.join("orgs.parquet")
    } else {
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
    };

    let file = File::open(&path).with_context(|| format!("opening {}", path.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let reader = builder.build()?;

    let target_code = args.ods_code.trim();

    struct FoundRecord {
        ods_code: String,
        name: String,
        entity_type: String,
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
        legal_start: Option<String>,
        legal_end: Option<String>,
        operational_start: Option<String>,
        operational_end: Option<String>,
        last_changed: Option<String>,
        trud_release_date: String,
    }

    let mut found: Option<FoundRecord> = None;

    let extract_date = |batch: &arrow::record_batch::RecordBatch, idx: Option<usize>, row: usize| -> Option<String> {
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
            let name_arr = batch.column(schema.index_of("name")?).as_any().downcast_ref::<StringArray>().context("name StringArray")?;
            let record_class_arr = batch.column(schema.index_of("entity_type")?).as_any().downcast_ref::<StringArray>().context("entity_type StringArray")?;
            let status_arr = batch.column(schema.index_of("status")?).as_any().downcast_ref::<StringArray>().context("status StringArray")?;
            let primary_role_arr = batch.column(schema.index_of("primary_role_code")?).as_any().downcast_ref::<StringArray>().context("primary_role_code StringArray")?;
            let roles_arr = batch.column(schema.index_of("role_codes")?).as_any().downcast_ref::<ListArray>().context("role_codes ListArray")?;
            let role_names_arr = batch.column(schema.index_of("role_names")?).as_any().downcast_ref::<ListArray>().context("role_names ListArray")?;

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

            let roles_list_val = roles_arr.value(i);
            let roles_str_arr = roles_list_val.as_any().downcast_ref::<StringArray>().context("roles_list StringArray")?;
            let mut role_codes = Vec::with_capacity(roles_str_arr.len());
            for j in 0..roles_str_arr.len() {
                role_codes.push(roles_str_arr.value(j).to_string());
            }

            let role_names_list_val = role_names_arr.value(i);
            let role_names_str_arr = role_names_list_val.as_any().downcast_ref::<StringArray>().context("role_names_list StringArray")?;
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

            let col_str = |idx: Option<usize>| -> String {
                col_opt(idx).unwrap_or_default()
            };

            let trud_date = extract_date(&batch, trud_release_date_idx, i).unwrap_or_default();

            found = Some(FoundRecord {
                ods_code: ods_code_arr.value(i).to_string(),
                name: name_arr.value(i).to_string(),
                entity_type: record_class_arr.value(i).to_string(),
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
                commissioner_name: col_str(commissioner_idx),
                commissioner_code: col_str(commissioner_code_idx),
                parent_name: col_str(parent_idx),
                parent_code: col_str(parent_code_idx),
                pcn_name: col_str(pcn_idx),
                pcn_code: col_str(pcn_code_idx),
                trust_name: col_str(trust_idx),
                trust_code: col_str(trust_code_idx),
                icb_name: col_str(icb_idx),
                icb_code: col_str(icb_code_idx),
                region_name: col_str(region_idx),
                region_code: col_str(region_code_idx),
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

    let succ_hops = crate::commands::find::walk_succession_chain(&rec.ods_code, &successions_graph, &org_metadata);
    let pred_hops = crate::commands::find::get_predecessors(&rec.ods_code, &successions_graph, &org_metadata);

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

            let opt_entity = |code: String, name: String| -> Option<HierarchyEntityJson> {
                if !code.is_empty() || !name.is_empty() {
                    Some(HierarchyEntityJson { code, name })
                } else {
                    None
                }
            };

            let info_json = InfoRecordJson {
                ods_code: rec.ods_code,
                name: rec.name,
                entity_type: rec.entity_type,
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
                commissioner: opt_entity(rec.commissioner_code, rec.commissioner_name),
                parent: opt_entity(rec.parent_code, rec.parent_name),
                pcn: opt_entity(rec.pcn_code, rec.pcn_name),
                trust: opt_entity(rec.trust_code, rec.trust_name),
                icb: opt_entity(rec.icb_code, rec.icb_name),
                region: opt_entity(rec.region_code, rec.region_name),
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
            use std::io::IsTerminal;
            let use_color = std::io::stdout().is_terminal();

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

            let inspector = crate::formatting::InspectorRecord {
                ods_code: &rec.ods_code,
                name: &rec.name,
                record_class: &rec.entity_type,
                status: &rec.status,
                role: &primary_role_name,
                role_code: &rec.primary_role_code,
                other_roles: &other_roles,
                address: &rec.address,
                country: rec.country.as_deref().unwrap_or(""),
                uprn: rec.uprn.as_deref().unwrap_or(""),
                telephone: rec.telephone.as_deref().unwrap_or(""),
                website: rec.website.as_deref().unwrap_or(""),
                commissioner: &rec.commissioner_name,
                commissioner_code: &rec.commissioner_code,
                parent: &rec.parent_name,
                parent_code: &rec.parent_code,
                pcn: &rec.pcn_name,
                pcn_code: &rec.pcn_code,
                trust: &rec.trust_name,
                trust_code: &rec.trust_code,
                icb: &rec.icb_name,
                icb_code: &rec.icb_code,
                region: &rec.region_name,
                region_code: &rec.region_code,
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

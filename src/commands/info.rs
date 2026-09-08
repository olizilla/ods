use anyhow::Result;
use arrow::array::{Array, StringArray};
use clap::{Args as ClapArgs, ValueEnum};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
pub mod model;
pub mod render;

use crate::commands::find::OrgRow;
use model::{
    InfoRecord, InfoRecordJson, InfoRelationship, InfoRole, LiveSuccessor, RelationshipItemJson,
    SuccessionHopJson,
};
use render::{render_info, RenderOptions, ResponsiveBand, TableStyle};

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputFormat {
    Table,
    Markdown,
    Json,
}

#[derive(ClapArgs, Debug, Clone)]
pub struct Args {
    /// Exact ODS organisation code (e.g. A82608, RJZ, 0AF)
    pub ods_code: String,

    /// Output format
    #[arg(short, long, value_enum, default_value = "table")]
    pub format: OutputFormat,

    /// Directory containing Parquet files (defaults to active release)
    #[arg(short, long)]
    pub input: Option<PathBuf>,

    /// Show all roles and relationships including closed/inactive
    #[arg(short, long)]
    pub all: bool,

    /// Override terminal width for responsive layout (e.g. 58, 78, 98)
    #[arg(short, long)]
    pub width: Option<u16>,

    /// Disable ANSI colored output
    #[arg(long)]
    pub plain: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            ods_code: String::new(),
            format: OutputFormat::Table,
            input: None,
            all: false,
            width: None,
            plain: false,
        }
    }
}

pub fn run(args: Args) -> Result<()> {
    let use_color = crate::ansi::stdout_color_enabled(args.plain);
    let resolved_input = crate::workspace::resolve_parquet_input(args.input.as_deref())?;
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
    if !parquet_dir.join("orgs.parquet").exists() && !parquet_dir.join("orgs_all.parquet").exists() {
        if args.input.is_some() || parquet_dir.join(crate::provenance::PROVENANCE_FILENAME).exists() {
            anyhow::bail!(
                "✖ Parquet file 'orgs.parquet' not found in '{}'",
                parquet_dir.display()
            );
        }
        if let Ok(ws) = crate::workspace::Workspace::open(None) {
            let releases = ws.releases().unwrap_or_default();
            if !releases.is_empty() {
                let n = releases.len();
                let count_str = if n == 1 {
                    "1 release".to_string()
                } else {
                    format!("{} releases", n)
                };
                let newest_date = &releases[0].date;
                let ws_name = ws.root()
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(crate::workspace::DEFAULT_WORKSPACE_DIR);
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

    let target_code = args.ods_code.trim();

    let (rec, is_from_orgs_all) = crate::commands::find::find_org_in_parquet(parquet_dir, target_code)?
        .ok_or_else(|| anyhow::anyhow!("✖ Organisation '{}' not found", target_code))?;

    let successions_graph = crate::commands::find::load_succession_graph(parquet_dir);
    let org_metadata = crate::commands::find::load_org_metadata(parquet_dir);

    let (successors_live, successor_hops) = if rec.operational_end.is_some()
        || rec.status.eq_ignore_ascii_case("inactive")
    {
        resolve_live_successors(&rec.ods_code, &successions_graph, &org_metadata)
    } else {
        (Vec::new(), 0)
    };

    let roles = load_roles(parquet_dir, &rec);
    let loaded_rels = load_relationships(parquet_dir, &rec.ods_code, &org_metadata);

    let outbound_rels: Vec<InfoRelationship> = loaded_rels
        .iter()
        .filter(|r| r.direction == "outbound")
        .map(InfoRelationship::from)
        .collect();

    let info_record = InfoRecord {
        org: rec.clone(),
        roles,
        relationships: outbound_rels,
        successors_live,
        successor_hops,
    };

    match args.format {
        OutputFormat::Table | OutputFormat::Markdown => {
            let is_markdown = args.format == OutputFormat::Markdown;
            let (band, color_enabled, style) = if is_markdown {
                (ResponsiveBand::Wide, false, TableStyle::Markdown)
            } else {
                let terminal_w = if let Some(w) = args.width {
                    w
                } else if let Some(cols) = std::env::var("COLUMNS").ok().and_then(|s| s.parse::<u16>().ok()) {
                    cols
                } else if use_color {
                    crossterm::terminal::size().map(|(w, _)| w).unwrap_or(78)
                } else {
                    78
                };

                let band = ResponsiveBand::from_width(terminal_w);
                let color_enabled = use_color;

                (band, color_enabled, TableStyle::Table)
            };

            let source_filename = if is_from_orgs_all {
                "orgs-all.parquet"
            } else {
                "orgs.parquet"
            };
            let source_header = crate::workspace::format_source_header(parquet_dir, source_filename, color_enabled);

            let options = RenderOptions {
                band,
                all: args.all,
                color: color_enabled,
                source_header: &source_header,
                style,
            };

            let rendered = render_info(&info_record, &options);
            write!(writer, "{}", rendered)?;
        }
        OutputFormat::Json => {
            let succ_hops = crate::commands::find::walk_succession_chain(
                &rec.ods_code,
                &successions_graph,
                &org_metadata,
            );
            let pred_hops = crate::commands::find::get_predecessors(
                &rec.ods_code,
                &successions_graph,
                &org_metadata,
            );

            let mut primary_role_name = String::new();
            let mut other_roles = Vec::new();

            for (code, name) in rec.role_codes.iter().zip(rec.role_names.iter()) {
                if code == &rec.primary_role_code {
                    primary_role_name = name.clone();
                } else {
                    other_roles.push(format!("{name} ({code})"));
                }
            }

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

            let info_json = InfoRecordJson {
                org: rec,
                primary_role_name,
                other_roles,
                relationships: loaded_rels,
                succession: succ_json,
                predecessors: pred_json,
            };

            writeln!(writer, "{}", serde_json::to_string_pretty(&info_json)?)?;
        }
    }
    Ok(())
}

pub fn load_relationships(
    parquet_dir: &Path,
    target_code: &str,
    org_meta: &std::collections::HashMap<String, (String, String)>,
) -> Vec<RelationshipItemJson> {
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

                rels.push(RelationshipItemJson {
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
                    operational_start: crate::commands::find::extract_batch_date(&batch, op_start_idx, i),
                    operational_end: crate::commands::find::extract_batch_date(&batch, op_end_idx, i),
                    legal_start: crate::commands::find::extract_batch_date(&batch, leg_start_idx, i),
                    legal_end: crate::commands::find::extract_batch_date(&batch, leg_end_idx, i),
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

fn load_roles(parquet_dir: &Path, org: &OrgRow) -> Vec<InfoRole> {
    let path = parquet_dir.join("roles.parquet");
    if let Ok(file) = File::open(&path) {
        if let Ok(builder) = ParquetRecordBatchReaderBuilder::try_new(file) {
            if let Ok(reader) = builder.build() {
                let mut roles = Vec::new();
                for batch in reader.flatten() {
                    let schema = batch.schema();
                    let Ok(ods_idx) = schema.index_of("ods_code") else {
                        continue;
                    };
                    let Ok(code_idx) = schema.index_of("role_code") else {
                        continue;
                    };
                    let Ok(name_idx) = schema.index_of("role_name") else {
                        continue;
                    };
                    let primary_idx = schema.index_of("is_primary").ok();
                    let status_idx = schema.index_of("role_status").ok();
                    let op_start_idx = schema.index_of("operational_start").ok();
                    let op_end_idx = schema.index_of("operational_end").ok();

                    let (Some(ods_arr), Some(code_arr), Some(name_arr)) = (
                        batch.column(ods_idx).as_any().downcast_ref::<StringArray>(),
                        batch.column(code_idx).as_any().downcast_ref::<StringArray>(),
                        batch.column(name_idx).as_any().downcast_ref::<StringArray>(),
                    ) else {
                        continue;
                    };

                    for i in 0..batch.num_rows() {
                        if ods_arr.is_valid(i) && ods_arr.value(i).eq_ignore_ascii_case(&org.ods_code)
                        {
                            let role_code = code_arr.value(i).to_string();
                            let role_name = name_arr.value(i).to_string();
                            let primary = primary_idx
                                .and_then(|idx| {
                                    batch
                                        .column(idx)
                                        .as_any()
                                        .downcast_ref::<arrow::array::BooleanArray>()
                                        .map(|a| a.value(i))
                                })
                                .unwrap_or_else(|| role_code == org.primary_role_code);

                            let status = status_idx
                                .and_then(|idx| {
                                    batch
                                        .column(idx)
                                        .as_any()
                                        .downcast_ref::<StringArray>()
                                        .map(|a| a.value(i).to_string())
                                })
                                .unwrap_or_else(|| "active".to_string());

                            let op_start = crate::commands::find::extract_batch_date(&batch, op_start_idx, i);
                            let op_end = crate::commands::find::extract_batch_date(&batch, op_end_idx, i);

                            roles.push(InfoRole {
                                role_code,
                                role_name,
                                primary,
                                status,
                                operational_start: op_start,
                                operational_end: op_end,
                            });
                        }
                    }
                }
                if !roles.is_empty() {
                    return roles;
                }
            }
        }
    }

    org.role_codes
        .iter()
        .zip(org.role_names.iter())
        .map(|(c, n)| InfoRole {
            role_code: c.clone(),
            role_name: n.clone(),
            primary: c == &org.primary_role_code,
            status: "active".to_string(),
            operational_start: None,
            operational_end: None,
        })
        .collect()
}

fn resolve_live_successors(
    code: &str,
    graph: &crate::commands::find::SuccessionGraph,
    org_meta: &HashMap<String, (String, String)>,
) -> (Vec<LiveSuccessor>, usize) {
    let mut live = Vec::new();
    let mut total_hops = 0;
    let mut visited = std::collections::HashSet::new();
    let mut queue = std::collections::VecDeque::new();

    visited.insert(code.to_string());
    if let Some(edges) = graph.forward.get(code) {
        for edge in edges {
            queue.push_back(edge.target_code.clone());
            visited.insert(edge.target_code.clone());
        }
    }

    while let Some(curr) = queue.pop_front() {
        let (name, status) = org_meta
            .get(&curr)
            .cloned()
            .unwrap_or_else(|| (String::new(), "unknown".to_string()));

        if status.eq_ignore_ascii_case("active") {
            live.push(LiveSuccessor {
                code: curr,
                name,
            });
        } else {
            total_hops += 1;
            if let Some(edges) = graph.forward.get(&curr) {
                for edge in edges {
                    if !visited.contains(&edge.target_code) {
                        visited.insert(edge.target_code.clone());
                        queue.push_back(edge.target_code.clone());
                    }
                }
            }
        }
    }

    live.sort_by(|a, b| a.code.cmp(&b.code));

    (live, total_hops)
}

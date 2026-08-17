use anyhow::{Context, Result};
use arrow::array::{Array, BooleanArray, StringArray, Date32Array};
use clap::Parser;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use rayon::prelude::*;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

fn col_str<'a>(batch: &'a arrow::record_batch::RecordBatch, idx: usize, col_name: &str) -> &'a StringArray {
    batch.column(idx)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap_or_else(|| panic!("Expected StringArray for column '{}'", col_name))
}

fn col_bool<'a>(batch: &'a arrow::record_batch::RecordBatch, idx: usize, col_name: &str) -> &'a BooleanArray {
    batch.column(idx)
        .as_any()
        .downcast_ref::<BooleanArray>()
        .unwrap_or_else(|| panic!("Expected BooleanArray for column '{}'", col_name))
}

fn col_opt_str(batch: &arrow::record_batch::RecordBatch, idx: usize, row_idx: usize, col_name: &str) -> Option<String> {
    let arr = col_str(batch, idx, col_name);
    if arr.is_valid(row_idx) {
        Some(arr.value(row_idx).to_string())
    } else {
        None
    }
}

fn col_date_str(batch: &arrow::record_batch::RecordBatch, idx: usize, row_idx: usize) -> Option<String> {
    let arr = batch.column(idx)
        .as_any()
        .downcast_ref::<Date32Array>()?;
    if arr.is_valid(row_idx) {
        let days = arr.value(row_idx);
        chrono::NaiveDate::from_ymd_opt(1970, 1, 1)?
            .checked_add_signed(chrono::Duration::days(days as i64))?
            .format("%Y-%m-%d")
            .to_string()
            .into()
    } else {
        None
    }
}

#[derive(Parser, Debug)]
pub struct Args {
    /// Input directory containing Parquet files (orgs.parquet, etc.)
    #[arg(long, short, default_value = ".")]
    pub input: PathBuf,

    /// Output zip file path (e.g. dist/wiki.zip)
    #[arg(long, short, default_value = "./wiki.zip")]
    pub output: PathBuf,
}

struct RoleRow {
    role: String,
    is_primary: bool,
    role_code: String,
}

struct RelRow {
    target: Option<String>,
    target_code: String,
    rel_type_code: String,
}

struct OrgRow {
    ods_code: String,
    record_class: String,
    status: String,
    role: String,
    role_code: String,
    name: String,
    address: Option<String>,
    country: Option<String>,
    uprn: Option<String>,
    telephone: Option<String>,
    website: Option<String>,
    commissioner: Option<String>,
    commissioner_code: Option<String>,
    parent: Option<String>,
    parent_code: Option<String>,
    pcn: Option<String>,
    pcn_code: Option<String>,
    trust: Option<String>,
    trust_code: Option<String>,
    icb: Option<String>,
    icb_code: Option<String>,
    region: Option<String>,
    region_code: Option<String>,
    operational_start: Option<String>,
    operational_end: Option<String>,
    legal_start: Option<String>,
    legal_end: Option<String>,
    last_change_date: Option<String>,
    postcode: Option<String>,
}

fn load_roles(path: &Path) -> Result<HashMap<String, Vec<RoleRow>>> {
    let file = File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let reader = builder.build()?;
    let mut map: HashMap<String, Vec<RoleRow>> = HashMap::new();

    for batch in reader {
        let batch = batch?;
        let schema = batch.schema();
        let ods_code_idx = schema.index_of("ods_code")?;
        let is_primary_idx = schema.index_of("is_primary")?;
        let role_code_idx = schema.index_of("role_code")?;

        let ods_code_arr = col_str(&batch, ods_code_idx, "ods_code");
        let is_primary_arr = col_bool(&batch, is_primary_idx, "is_primary");
        let role_code_arr = col_str(&batch, role_code_idx, "role_code");

        // org_roles.parquet carries codes only; names come from the curated
        // vocabulary, which this same `ods make` run just wrote.
        let vocab = crate::roles::role_names();

        for i in 0..batch.num_rows() {
            let ods_code = ods_code_arr.value(i).to_string();
            let role_code = role_code_arr.value(i).to_string();
            let row = RoleRow {
                role: vocab.name(&role_code).unwrap_or(&role_code).to_string(),
                is_primary: is_primary_arr.value(i),
                role_code,
            };
            map.entry(ods_code).or_default().push(row);
        }
    }

    Ok(map)
}

fn load_rels(path: &Path) -> Result<HashMap<String, Vec<RelRow>>> {
    let file = File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let reader = builder.build()?;
    let mut map: HashMap<String, Vec<RelRow>> = HashMap::new();

    for batch in reader {
        let batch = batch?;
        let schema = batch.schema();
        let source_code_idx = schema.index_of("source_code")?;
        let target_idx = schema.index_of("target").ok();
        let target_code_idx = schema.index_of("target_code")?;
        let rel_type_code_idx = schema.index_of("rel_type_code")?;

        let source_code_arr = col_str(&batch, source_code_idx, "source_code");
        let target_code_arr = col_str(&batch, target_code_idx, "target_code");
        let rel_type_code_arr = col_str(&batch, rel_type_code_idx, "rel_type_code");

        for i in 0..batch.num_rows() {
            let source_code = source_code_arr.value(i).to_string();
            let target_val = target_idx.and_then(|idx| col_opt_str(&batch, idx, i, "target"));
            let row = RelRow {
                target: target_val,
                target_code: target_code_arr.value(i).to_string(),
                rel_type_code: rel_type_code_arr.value(i).to_string(),
            };
            map.entry(source_code).or_default().push(row);
        }
    }

    Ok(map)
}

pub fn run(args: Args) -> Result<()> {
    let roles_path = args.input.join("org_roles.parquet");
    let rels_path = if args.input.join("relationships.parquet").exists() {
        args.input.join("relationships.parquet")
    } else {
        args.input.join("rels.parquet")
    };
    let orgs_path = args.input.join("orgs.parquet");

    // 1. Fail early: check input files
    if !orgs_path.exists() {
        anyhow::bail!("Missing 'orgs.parquet' in input directory: {}", args.input.display());
    }
    if !roles_path.exists() {
        anyhow::bail!("Missing 'org_roles.parquet' in input directory: {}", args.input.display());
    }
    if !rels_path.exists() {
        anyhow::bail!("Missing 'relationships.parquet' in input directory: {}", args.input.display());
    }

    // 2. Fail early: create output parent directory and zip file handle
    if let Some(parent) = args.output.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating directory for output: {}", parent.display()))?;
        }
    }
    let zip_file = File::create(&args.output)
        .with_context(|| format!("creating output zip file: {}", args.output.display()))?;

    println!("Loading roles and relationships from Parquet...");
    let roles_map = load_roles(&roles_path)
        .with_context(|| format!("loading roles from {}", roles_path.display()))?;
    let rels_map = load_rels(&rels_path)
        .with_context(|| format!("loading relationships from {}", rels_path.display()))?;

    // Load all organisations sequentially from orgs.parquet
    println!("Loading organisations from Parquet...");
    let file = File::open(&orgs_path)
        .with_context(|| format!("opening orgs.parquet from {}", orgs_path.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let reader = builder.build()?;

    let mut org_rows = Vec::new();

    for batch in reader {
        let batch = batch?;
        let schema = batch.schema();
        let ods_code_idx = schema.index_of("ods_code")?;
        let record_class_idx = schema.index_of("entity_type")?;
        let status_idx = schema.index_of("status")?;
        let primary_role_idx = schema.index_of("primary_role_code")?;
        let name_idx = schema.index_of("name")?;
        let address_idx = schema.index_of("address")?;
        let postcode_idx = schema.index_of("postcode")?;
        let country_idx = schema.index_of("country")?;
        let uprn_idx = schema.index_of("uprn")?;
        let telephone_idx = schema.index_of("telephone")?;
        let website_idx = schema.index_of("website")?;
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

        let ods_code_arr = col_str(&batch, ods_code_idx, "ods_code");
        let record_class_arr = col_str(&batch, record_class_idx, "entity_type");
        let status_arr = col_str(&batch, status_idx, "status");
        let primary_role_arr = col_str(&batch, primary_role_idx, "primary_role_code");
        let org_vocab = crate::roles::role_names();
        let name_arr = col_str(&batch, name_idx, "name");

        for i in 0..batch.num_rows() {
            org_rows.push(OrgRow {
                ods_code: ods_code_arr.value(i).to_string(),
                record_class: record_class_arr.value(i).to_string(),
                status: status_arr.value(i).to_string(),
                role: org_vocab
                    .name(primary_role_arr.value(i))
                    .unwrap_or(primary_role_arr.value(i))
                    .to_string(),
                role_code: primary_role_arr.value(i).to_string(),
                name: name_arr.value(i).to_string(),
                address: col_opt_str(&batch, address_idx, i, "address"),
                postcode: col_opt_str(&batch, postcode_idx, i, "postcode"),
                country: col_opt_str(&batch, country_idx, i, "country"),
                uprn: col_opt_str(&batch, uprn_idx, i, "uprn"),
                telephone: col_opt_str(&batch, telephone_idx, i, "telephone"),
                website: col_opt_str(&batch, website_idx, i, "website"),
                commissioner: commissioner_idx.and_then(|idx| col_opt_str(&batch, idx, i, "commissioner")),
                commissioner_code: commissioner_code_idx.and_then(|idx| col_opt_str(&batch, idx, i, "commissioner_code")),
                parent: parent_idx.and_then(|idx| col_opt_str(&batch, idx, i, "parent")),
                parent_code: parent_code_idx.and_then(|idx| col_opt_str(&batch, idx, i, "parent_code")),
                pcn: pcn_idx.and_then(|idx| col_opt_str(&batch, idx, i, "pcn")),
                pcn_code: pcn_code_idx.and_then(|idx| col_opt_str(&batch, idx, i, "pcn_code")),
                trust: trust_idx.and_then(|idx| col_opt_str(&batch, idx, i, "trust")),
                trust_code: trust_code_idx.and_then(|idx| col_opt_str(&batch, idx, i, "trust_code")),
                icb: icb_idx.and_then(|idx| col_opt_str(&batch, idx, i, "icb")),
                icb_code: icb_code_idx.and_then(|idx| col_opt_str(&batch, idx, i, "icb_code")),
                region: region_idx.and_then(|idx| col_opt_str(&batch, idx, i, "region")),
                region_code: region_code_idx.and_then(|idx| col_opt_str(&batch, idx, i, "region_code")),
                operational_start: col_date_str(&batch, op_start_idx.unwrap_or(usize::MAX), i),
                operational_end: col_date_str(&batch, op_end_idx.unwrap_or(usize::MAX), i),
                legal_start: col_date_str(&batch, leg_start_idx.unwrap_or(usize::MAX), i),
                legal_end: col_date_str(&batch, leg_end_idx.unwrap_or(usize::MAX), i),
                last_change_date: col_date_str(&batch, last_change_idx.unwrap_or(usize::MAX), i),
            });
        }
    }

    // Parallel render of Markdown pages in memory using Rayon
    println!("Rendering OKF Wiki pages in parallel...");
    let pages: Vec<(String, String)> = org_rows
        .into_par_iter()
        .map(|org| {
            let role_folder = role_to_folder(&org.role);
            let zip_path = format!("organisations/{}/{}.md", role_folder, org.ods_code);

            let mut content = Vec::new();
            
            // Write YAML frontmatter
            writeln!(content, "---").unwrap();
            writeln!(content, "type: {}", org.role).unwrap();
            writeln!(content, "title: {}", org.name).unwrap();
            if let Some(ref pc) = org.postcode {
                writeln!(content, "postcode: {}", pc).unwrap();
            }
            if let Some(ref u) = org.uprn {
                writeln!(content, "uprn: \"{}\"", u).unwrap();
            }
            if let Some(ref tel) = org.telephone {
                writeln!(content, "telephone: \"{}\"", tel).unwrap();
            }
            if let Some(ref web) = org.website {
                writeln!(content, "website: \"{}\"", web).unwrap();
            }
            writeln!(
                content,
                "resource: https://directory.spineservices.nhs.uk/OdsWebService/Ods/{}",
                org.ods_code
            ).unwrap();
            writeln!(content, "---").unwrap();
            writeln!(content).unwrap();

            // Render Markdown body using shared inspector renderer
            let sec_roles: Vec<String> = roles_map.get(&org.ods_code)
                .map(|roles| roles.iter().filter(|r| !r.is_primary).map(|r| {
                    format!("{} ({})", r.role, r.role_code)
                }).collect())
                .unwrap_or_default();

            let successors_vec: Vec<crate::formatting::SuccessionHopLink> = rels_map.get(&org.ods_code)
                .map(|rels| rels.iter().filter(|r| r.rel_type_code == "SUCCESSOR").map(|s| {
                    crate::formatting::SuccessionHopLink {
                        depth: 1,
                        date: None,
                        code: &s.target_code,
                        name: s.target.as_deref().unwrap_or(""),
                        status: "active",
                    }
                }).collect())
                .unwrap_or_default();

            let inspector = crate::formatting::InspectorRecord {
                ods_code: &org.ods_code,
                name: &org.name,
                record_class: &org.record_class,
                status: &org.status,
                role: &org.role,
                role_code: &org.role_code,
                other_roles: &sec_roles,
                address: org.address.as_deref().unwrap_or(""),
                country: org.country.as_deref().unwrap_or(""),
                uprn: org.uprn.as_deref().unwrap_or(""),
                telephone: org.telephone.as_deref().unwrap_or(""),
                website: org.website.as_deref().unwrap_or(""),
                commissioner: org.commissioner.as_deref().unwrap_or(""),
                commissioner_code: org.commissioner_code.as_deref().unwrap_or(""),
                parent: org.parent.as_deref().unwrap_or(""),
                parent_code: org.parent_code.as_deref().unwrap_or(""),
                pcn: org.pcn.as_deref().unwrap_or(""),
                pcn_code: org.pcn_code.as_deref().unwrap_or(""),
                trust: org.trust.as_deref().unwrap_or(""),
                trust_code: org.trust_code.as_deref().unwrap_or(""),
                icb: org.icb.as_deref().unwrap_or(""),
                icb_code: org.icb_code.as_deref().unwrap_or(""),
                region: org.region.as_deref().unwrap_or(""),
                region_code: org.region_code.as_deref().unwrap_or(""),
                succession: &successors_vec,
                predecessors: &[],
                operational_start: org.operational_start.as_deref(),
                operational_end: org.operational_end.as_deref(),
                legal_start: org.legal_start.as_deref(),
                legal_end: org.legal_end.as_deref(),
                last_change_date: org.last_change_date.as_deref(),
            };

            crate::formatting::render_inspector_markdown(&inspector, false, &mut content).unwrap();

            let markdown_string = String::from_utf8(content)
                .expect("OKF content is constructed from validated UTF-8 string fields");
            (zip_path, markdown_string)
        })
        .collect();

    // Stream rendered pages sequentially into ZipWriter
    println!("Streaming pages into Zip archive...");
    let mut zip = ZipWriter::new(std::io::BufWriter::new(zip_file));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

    for (zip_path, content) in &pages {
        zip.start_file(zip_path.as_str(), options)?;
        zip.write_all(content.as_bytes())?;
    }

    zip.finish()?;
    println!("Generated {} OKF Markdown files under {}.", pages.len(), args.output.display());
    Ok(())
}

fn role_to_folder(role: &str) -> String {
    role.to_lowercase()
        .replace([' ', '/', '-'], "_")
}

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
    #[arg(long, short)]
    pub input: PathBuf,

    /// Output zip file path (e.g. dist/wiki.zip)
    #[arg(long, short)]
    pub output: PathBuf,
}

struct RoleRow {
    role: String,
    is_primary: bool,
    status: String,
    role_code: String,
    legal_start: Option<String>,
    legal_end: Option<String>,
    operational_start: Option<String>,
    operational_end: Option<String>,
}

struct RelRow {
    rel_type: String,
    status: String,
    target: Option<String>,
    target_code: String,
    rel_type_code: String,
    legal_start: Option<String>,
    legal_end: Option<String>,
    operational_start: Option<String>,
    operational_end: Option<String>,
}

struct OrgRow {
    ods_code: String,
    status: String,
    role: String,
    name: String,
    address: Option<String>,
    town: Option<String>,
    county: Option<String>,
    postcode: Option<String>,
    country: Option<String>,
    uprn: Option<String>,
    telephone: Option<String>,
    website: Option<String>,
    parent_name: Option<String>,
    parent_code: Option<String>,
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
        let role_idx = schema.index_of("role")?;
        let is_primary_idx = schema.index_of("is_primary")?;
        let status_idx = schema.index_of("status")?;
        let role_code_idx = schema.index_of("role_code")?;
        let legal_start_idx = schema.index_of("legal_start")?;
        let legal_end_idx = schema.index_of("legal_end")?;
        let operational_start_idx = schema.index_of("operational_start")?;
        let operational_end_idx = schema.index_of("operational_end")?;

        let ods_code_arr = col_str(&batch, ods_code_idx, "ods_code");
        let role_arr = col_str(&batch, role_idx, "role");
        let is_primary_arr = col_bool(&batch, is_primary_idx, "is_primary");
        let status_arr = col_str(&batch, status_idx, "status");
        let role_code_arr = col_str(&batch, role_code_idx, "role_code");

        for i in 0..batch.num_rows() {
            let ods_code = ods_code_arr.value(i).to_string();
            let row = RoleRow {
                role: role_arr.value(i).to_string(),
                is_primary: is_primary_arr.value(i),
                status: status_arr.value(i).to_string(),
                role_code: role_code_arr.value(i).to_string(),
                legal_start: col_date_str(&batch, legal_start_idx, i),
                legal_end: col_date_str(&batch, legal_end_idx, i),
                operational_start: col_date_str(&batch, operational_start_idx, i),
                operational_end: col_date_str(&batch, operational_end_idx, i),
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
        let rel_type_idx = schema.index_of("rel_type")?;
        let status_idx = schema.index_of("status")?;
        let target_idx = schema.index_of("target")?;
        let target_code_idx = schema.index_of("target_code")?;
        let rel_type_code_idx = schema.index_of("rel_type_code")?;
        let legal_start_idx = schema.index_of("legal_start")?;
        let legal_end_idx = schema.index_of("legal_end")?;
        let operational_start_idx = schema.index_of("operational_start")?;
        let operational_end_idx = schema.index_of("operational_end")?;

        let source_code_arr = col_str(&batch, source_code_idx, "source_code");
        let rel_type_arr = col_str(&batch, rel_type_idx, "rel_type");
        let status_arr = col_str(&batch, status_idx, "status");
        let target_code_arr = col_str(&batch, target_code_idx, "target_code");
        let rel_type_code_arr = col_str(&batch, rel_type_code_idx, "rel_type_code");

        for i in 0..batch.num_rows() {
            let source_code = source_code_arr.value(i).to_string();
            let row = RelRow {
                rel_type: rel_type_arr.value(i).to_string(),
                status: status_arr.value(i).to_string(),
                target: col_opt_str(&batch, target_idx, i, "target"),
                target_code: target_code_arr.value(i).to_string(),
                rel_type_code: rel_type_code_arr.value(i).to_string(),
                legal_start: col_date_str(&batch, legal_start_idx, i),
                legal_end: col_date_str(&batch, legal_end_idx, i),
                operational_start: col_date_str(&batch, operational_start_idx, i),
                operational_end: col_date_str(&batch, operational_end_idx, i),
            };
            map.entry(source_code).or_default().push(row);
        }
    }

    Ok(map)
}

fn load_org_roles(path: &Path) -> Result<HashMap<String, String>> {
    let file = File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let reader = builder.build()?;
    let mut map = HashMap::new();

    for batch in reader {
        let batch = batch?;
        let schema = batch.schema();
        let ods_idx = schema.index_of("ods_code")?;
        let role_idx = schema.index_of("role")?;

        let ods_arr = col_str(&batch, ods_idx, "ods_code");
        let role_arr = col_str(&batch, role_idx, "role");

        for i in 0..batch.num_rows() {
            map.insert(ods_arr.value(i).to_string(), role_arr.value(i).to_string());
        }
    }
    Ok(map)
}

fn format_dates(l_start: &Option<String>, l_end: &Option<String>, o_start: &Option<String>, o_end: &Option<String>) -> (String, String) {
    let start = l_start.as_deref().or(o_start.as_deref()).unwrap_or("-").to_string();
    let end = l_end.as_deref().or(o_end.as_deref()).unwrap_or("-").to_string();
    (start, end)
}

pub fn run(args: Args) -> Result<()> {
    let roles_path = args.input.join("roles.parquet");
    let rels_path = args.input.join("rels.parquet");
    let orgs_path = args.input.join("orgs.parquet");

    println!("Loading roles and relationships from Parquet...");
    let roles_map = load_roles(&roles_path)
        .with_context(|| format!("loading roles from {}", roles_path.display()))?;
    let rels_map = load_rels(&rels_path)
        .with_context(|| format!("loading relationships from {}", rels_path.display()))?;
    let org_role_map = load_org_roles(&orgs_path)
        .with_context(|| format!("indexing organisation roles from {}", orgs_path.display()))?;

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
        let status_idx = schema.index_of("status")?;
        let role_idx = schema.index_of("role")?;
        let name_idx = schema.index_of("name")?;
        let address_idx = schema.index_of("address")?;
        let town_idx = schema.index_of("town")?;
        let county_idx = schema.index_of("county")?;
        let postcode_idx = schema.index_of("postcode")?;
        let country_idx = schema.index_of("country")?;
        let uprn_idx = schema.index_of("uprn")?;
        let telephone_idx = schema.index_of("telephone")?;
        let website_idx = schema.index_of("website")?;
        let parent_idx = schema.index_of("parent")?;
        let parent_code_idx = schema.index_of("parent_code")?;

        let ods_code_arr = col_str(&batch, ods_code_idx, "ods_code");
        let status_arr = col_str(&batch, status_idx, "status");
        let role_arr = col_str(&batch, role_idx, "role");
        let name_arr = col_str(&batch, name_idx, "name");

        for i in 0..batch.num_rows() {
            org_rows.push(OrgRow {
                ods_code: ods_code_arr.value(i).to_string(),
                status: status_arr.value(i).to_string(),
                role: role_arr.value(i).to_string(),
                name: name_arr.value(i).to_string(),
                address: col_opt_str(&batch, address_idx, i, "address"),
                town: col_opt_str(&batch, town_idx, i, "town"),
                county: col_opt_str(&batch, county_idx, i, "county"),
                postcode: col_opt_str(&batch, postcode_idx, i, "postcode"),
                country: col_opt_str(&batch, country_idx, i, "country"),
                uprn: col_opt_str(&batch, uprn_idx, i, "uprn"),
                telephone: col_opt_str(&batch, telephone_idx, i, "telephone"),
                website: col_opt_str(&batch, website_idx, i, "website"),
                parent_name: col_opt_str(&batch, parent_idx, i, "parent"),
                parent_code: col_opt_str(&batch, parent_code_idx, i, "parent_code"),
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

            // Write body
            writeln!(content, "# {}", org.name).unwrap();
            writeln!(content).unwrap();
            writeln!(content, "Status: {}", org.status).unwrap();
            writeln!(content, "Role: {}", org.role).unwrap();
            writeln!(content).unwrap();

            if let (Some(p_code), Some(p_name)) = (&org.parent_code, &org.parent_name) {
                let parent_friendly = get_friendly_name(p_name);
                let parent_role = org_role_map.get(p_code).map(|s| s.as_str()).unwrap_or("NHS Trust");
                let parent_folder = role_to_folder(parent_role);
                writeln!(
                    content,
                    "Parent Organisation: [{parent_friendly}](/organisations/{parent_folder}/{}.md)",
                    p_code
                ).unwrap();
                writeln!(content).unwrap();
            }

            // Address
            if org.address.is_some() || org.town.is_some() || org.county.is_some() || org.postcode.is_some() || org.country.is_some() || org.uprn.is_some() {
                writeln!(content, "## Address").unwrap();
                writeln!(content).unwrap();
                if let Some(ref addr) = org.address {
                    for line in addr.split(", ") {
                        let trimmed = line.trim();
                        if !trimmed.is_empty() {
                            writeln!(content, "*   {}", trimmed).unwrap();
                        }
                    }
                }
                if let Some(ref t) = org.town {
                    writeln!(content, "*   Town: {}", t).unwrap();
                }
                if let Some(ref c) = org.county {
                    writeln!(content, "*   County: {}", c).unwrap();
                }
                if let Some(ref pc) = org.postcode {
                    writeln!(content, "*   Postcode: {}", pc).unwrap();
                }
                if let Some(ref co) = org.country {
                    writeln!(content, "*   Country: {}", co).unwrap();
                }
                if let Some(ref u) = org.uprn {
                    writeln!(content, "*   UPRN: {}", u).unwrap();
                }
                writeln!(content).unwrap();
            }

            // Contacts
            if org.telephone.is_some() || org.website.is_some() {
                writeln!(content, "## Contact Details").unwrap();
                writeln!(content).unwrap();
                if let Some(ref tel) = org.telephone {
                    writeln!(content, "*   **Telephone**: {}", tel).unwrap();
                }
                if let Some(ref web) = org.website {
                    writeln!(content, "*   **Website**: {}", web).unwrap();
                }
                writeln!(content).unwrap();
            }

            // Roles
            if let Some(roles) = roles_map.get(&org.ods_code) {
                if !roles.is_empty() {
                    writeln!(content, "## Roles").unwrap();
                    writeln!(content).unwrap();
                    writeln!(content, "| Role ID | Name | Primary | Status | Start Date | End Date |").unwrap();
                    writeln!(content, "| :--- | :--- | :---: | :--- | :--- | :--- |").unwrap();
                    for r in roles {
                        let primary = if r.is_primary { "Yes" } else { "No" };
                        let (start_str, end_str) = format_dates(&r.legal_start, &r.legal_end, &r.operational_start, &r.operational_end);
                        writeln!(
                            content,
                            "| {} | {} | {} | {} | {} | {} |",
                            r.role_code, r.role, primary, r.status, start_str, end_str
                        ).unwrap();
                    }
                    writeln!(content).unwrap();
                }
            }

            // Relationships & Successors
            if let Some(rels) = rels_map.get(&org.ods_code) {
                let mut normal_rels = Vec::new();
                let mut successor_history = Vec::new();

                for r in rels {
                    if r.rel_type_code == "SUCCESSOR" {
                        successor_history.push(r);
                    } else {
                        normal_rels.push(r);
                    }
                }

                if !normal_rels.is_empty() {
                    writeln!(content, "## Relationships").unwrap();
                    writeln!(content).unwrap();
                    writeln!(content, "| Relationship | Target | Target ODS Code | Status | Start Date | End Date |").unwrap();
                    writeln!(content, "| :--- | :--- | :--- | :--- | :--- | :--- |").unwrap();
                    for r in normal_rels {
                        let target_name = r.target.as_deref().unwrap_or("Unknown");
                        let target_role = org_role_map.get(&r.target_code).map(|s| s.as_str()).unwrap_or("Unknown");
                        let target_folder = role_to_folder(target_role);
                        let target_link = format!("[{}](/organisations/{}/{}.md)", target_name, target_folder, r.target_code);
                        let (start_str, end_str) = format_dates(&r.legal_start, &r.legal_end, &r.operational_start, &r.operational_end);

                        writeln!(
                            content,
                            "| {} | {} | {} | {} | {} | {} |",
                            r.rel_type, target_link, r.target_code, r.status, start_str, end_str
                        ).unwrap();
                    }
                    writeln!(content).unwrap();
                }

                if !successor_history.is_empty() {
                    writeln!(content, "## Successor History").unwrap();
                    writeln!(content).unwrap();
                    writeln!(content, "| Type | Target | Target ODS Code | Start Date | End Date |").unwrap();
                    writeln!(content, "| :--- | :--- | :--- | :--- | :--- |").unwrap();
                    for s in successor_history {
                        let target_name = s.target.as_deref().unwrap_or("Unknown");
                        let target_role = org_role_map.get(&s.target_code).map(|s| s.as_str()).unwrap_or("Unknown");
                        let target_folder = role_to_folder(target_role);
                        let target_link = format!("[{}](/organisations/{}/{}.md)", target_name, target_folder, s.target_code);
                        let (start_str, end_str) = format_dates(&s.legal_start, &s.legal_end, &s.operational_start, &s.operational_end);

                        writeln!(
                            content,
                            "| {} | {} | {} | {} | {}",
                            s.rel_type, target_link, s.target_code, start_str, end_str
                        ).unwrap();
                    }
                    writeln!(content).unwrap();
                }
            }

            let markdown_string = String::from_utf8(content)
                .expect("OKF content is constructed from validated UTF-8 string fields");
            (zip_path, markdown_string)
        })
        .collect();

    // Stream rendered pages sequentially into ZipWriter
    println!("Streaming pages into Zip archive...");
    let file = File::create(&args.output)
        .with_context(|| format!("creating output zip: {}", args.output.display()))?;
    let mut zip = ZipWriter::new(std::io::BufWriter::new(file));
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

fn get_friendly_name(full_name: &str) -> String {
    let mut words = Vec::new();
    for word in full_name.split_whitespace() {
        let lower = word.to_lowercase();
        if lower == "nhs" {
            words.push("NHS".to_string());
        } else {
            let mut chars = lower.chars();
            let first = chars.next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
            words.push(format!("{}{}", first, chars.as_str()));
        }
    }
    words.join(" ")
}

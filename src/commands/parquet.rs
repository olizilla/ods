use anyhow::{Context, Result};
use arrow::array::{ArrayRef, BooleanBuilder, StringBuilder, Date32Builder};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use clap::Parser;
use parquet::arrow::arrow_writer::ArrowWriter;
use parquet::file::properties::WriterProperties;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::compile::OdsRecord;

const BATCH_SIZE: usize = 50_000;

#[derive(Parser, Debug)]
pub struct Args {
    /// NDJSON input file
    #[arg(long, short, default_value = "./ods.ndjson")]
    pub input: PathBuf,

    /// Output Parquet directory path
    #[arg(long, short, default_value = ".")]
    pub output: PathBuf,
}

fn embed_metadata(schema: &Schema, prov: Option<&crate::provenance::OdsProvenance>) -> Arc<Schema> {
    let mut meta = HashMap::new();
    let now = chrono::Utc::now();
    let pub_date = prov.and_then(|p| p.publication_date.clone()).unwrap_or_else(|| now.format("%Y-%m-%d").to_string());
    let pub_seq = prov.and_then(|p| p.publication_seq_num.clone()).unwrap_or_else(|| "unknown".to_string());
    let pub_type = prov.and_then(|p| p.publication_type.clone()).unwrap_or_else(|| "Full".to_string());

    meta.insert("ods.publication_date".to_string(), pub_date.clone());
    meta.insert("ods.publication_seq_num".to_string(), pub_seq);
    meta.insert("ods.publication_type".to_string(), pub_type.clone());
    meta.insert("ods.edition_label".to_string(), format!("TRUD ODS {} {}", pub_type, pub_date));
    meta.insert("ods.compiler_version".to_string(), env!("CARGO_PKG_VERSION").to_string());
    meta.insert("ods.created_at".to_string(), now.to_rfc3339());
    Arc::new(schema.clone().with_metadata(meta))
}

fn writer_properties(prov: Option<&crate::provenance::OdsProvenance>) -> WriterProperties {
    let now = chrono::Utc::now();
    let pub_date = prov.and_then(|p| p.publication_date.clone()).unwrap_or_else(|| now.format("%Y-%m-%d").to_string());
    let pub_seq = prov.and_then(|p| p.publication_seq_num.clone()).unwrap_or_else(|| "unknown".to_string());
    let pub_type = prov.and_then(|p| p.publication_type.clone()).unwrap_or_else(|| "Full".to_string());
    let pub_source = prov.and_then(|p| p.publication_source.clone()).unwrap_or_else(|| "HSCIC".to_string());
    let xml_version = prov.and_then(|p| p.xml_version.clone()).unwrap_or_else(|| "2-0-0".to_string());

    let meta_kv = vec![
        parquet::file::metadata::KeyValue {
            key: "ods.publication_date".to_string(),
            value: Some(pub_date.clone()),
        },
        parquet::file::metadata::KeyValue {
            key: "ods.publication_seq_num".to_string(),
            value: Some(pub_seq),
        },
        parquet::file::metadata::KeyValue {
            key: "ods.publication_type".to_string(),
            value: Some(pub_type),
        },
        parquet::file::metadata::KeyValue {
            key: "ods.publication_source".to_string(),
            value: Some(pub_source),
        },
        parquet::file::metadata::KeyValue {
            key: "ods.xml_version".to_string(),
            value: Some(xml_version),
        },
        parquet::file::metadata::KeyValue {
            key: "ods.compiler_version".to_string(),
            value: Some(env!("CARGO_PKG_VERSION").to_string()),
        },
        parquet::file::metadata::KeyValue {
            key: "ods.created_at".to_string(),
            value: Some(now.to_rfc3339()),
        },
    ];
    WriterProperties::builder()
        .set_key_value_metadata(Some(meta_kv))
        .build()
}

pub fn run(args: Args) -> Result<()> {
    let input_file = File::open(&args.input)
        .with_context(|| format!("opening NDJSON input: {}", args.input.display()))?;
    let reader = std::io::BufReader::new(input_file);

    let mut records = Vec::new();
    let mut provenance = None;
    for line in reader.lines() {
        let line = line.context("reading line from NDJSON")?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if provenance.is_none() {
            if let Some(prov) = crate::provenance::try_parse_provenance_line(trimmed) {
                provenance = Some(prov);
                continue;
            }
        }
        let record: OdsRecord = serde_json::from_str(trimmed)
            .with_context(|| format!("parsing JSON line: {}", trimmed))?;
        records.push(record);
    }

    std::fs::create_dir_all(&args.output)
        .with_context(|| format!("creating output directory: {}", args.output.display()))?;

    // 1. Export orgs.parquet (Active only)
    export_orgs(&args.output, &records, provenance.as_ref())?;

    // 2. Export orgs_all.parquet (All records)
    export_orgs_all(&args.output, &records, provenance.as_ref())?;

    // 3. Export roles.parquet
    export_roles(&args.output, &records, provenance.as_ref())?;

    // 4. Export rels.parquet
    export_rels(&args.output, &records, provenance.as_ref())?;

    // 5. Export successors.parquet
    export_successors(&args.output, &records, provenance.as_ref())?;

    Ok(())
}

fn append_opt(builder: &mut StringBuilder, val: Option<&str>) {
    if let Some(v) = val {
        builder.append_value(v);
    } else {
        builder.append_null();
    }
}

fn parse_date_to_days(val: &str) -> Option<i32> {
    chrono::NaiveDate::parse_from_str(val, "%Y-%m-%d")
        .ok()
        .map(|date| {
            date.signed_duration_since(chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap())
                .num_days() as i32
        })
}

fn append_date(builder: &mut Date32Builder, val: Option<&str>) {
    if let Some(v) = val {
        if let Some(days) = parse_date_to_days(v) {
            builder.append_value(days);
        } else {
            builder.append_null();
        }
    } else {
        builder.append_null();
    }
}

fn extract_dates(dates: &[crate::commands::compile::OdsDate]) -> (Option<String>, Option<String>, Option<String>, Option<String>) {
    let mut legal_start = None;
    let mut legal_end = None;
    let mut operational_start = None;
    let mut operational_end = None;

    for d in dates {
        if d.date_type == "Legal" {
            legal_start = d.start.clone();
            legal_end = d.end.clone();
        } else if d.date_type == "Operational" {
            operational_start = d.start.clone();
            operational_end = d.end.clone();
        }
    }

    (legal_start, legal_end, operational_start, operational_end)
}

// ==========================================
// orgs.parquet
// ==========================================

pub fn orgs_schema() -> Schema {
    Schema::new(vec![
        Field::new("ods_code", DataType::Utf8, false),
        Field::new("record_class", DataType::Utf8, false),
        Field::new("status", DataType::Utf8, false),
        Field::new("role", DataType::Utf8, false),
        Field::new("role_code", DataType::Utf8, false),
        Field::new("name", DataType::Utf8, false),
        Field::new("address", DataType::Utf8, true),
        Field::new("town", DataType::Utf8, true),
        Field::new("county", DataType::Utf8, true),
        Field::new("postcode", DataType::Utf8, true),
        Field::new("country", DataType::Utf8, true),
        Field::new("uprn", DataType::Utf8, true),
        Field::new("telephone", DataType::Utf8, true),
        Field::new("website", DataType::Utf8, true),
        Field::new("commissioner", DataType::Utf8, true),
        Field::new("commissioner_code", DataType::Utf8, true),
        Field::new("parent", DataType::Utf8, true),
        Field::new("parent_code", DataType::Utf8, true),
        Field::new("pcn", DataType::Utf8, true),
        Field::new("pcn_code", DataType::Utf8, true),
        Field::new("trust", DataType::Utf8, true),
        Field::new("trust_code", DataType::Utf8, true),
        Field::new("icb", DataType::Utf8, true),
        Field::new("icb_code", DataType::Utf8, true),
        Field::new("region", DataType::Utf8, true),
        Field::new("region_code", DataType::Utf8, true),
        Field::new("legal_start", DataType::Date32, true),
        Field::new("legal_end", DataType::Date32, true),
        Field::new("operational_start", DataType::Date32, true),
        Field::new("operational_end", DataType::Date32, true),
        Field::new("last_change_date", DataType::Date32, true),
    ])
}

fn build_orgs_batch(schema: &Arc<Schema>, records: &[&OdsRecord]) -> Result<RecordBatch> {
    let mut ods_code = StringBuilder::new();
    let mut record_class = StringBuilder::new();
    let mut status = StringBuilder::new();
    let mut role = StringBuilder::new();
    let mut name = StringBuilder::new();
    let mut address = StringBuilder::new();
    let mut town = StringBuilder::new();
    let mut county = StringBuilder::new();
    let mut postcode = StringBuilder::new();
    let mut country = StringBuilder::new();
    let mut uprn = StringBuilder::new();
    let mut telephone = StringBuilder::new();
    let mut website = StringBuilder::new();
    let mut commissioner = StringBuilder::new();
    let mut commissioner_code = StringBuilder::new();
    let mut parent = StringBuilder::new();
    let mut parent_code = StringBuilder::new();
    let mut pcn = StringBuilder::new();
    let mut pcn_code = StringBuilder::new();
    let mut trust = StringBuilder::new();
    let mut trust_code = StringBuilder::new();
    let mut icb = StringBuilder::new();
    let mut icb_code = StringBuilder::new();
    let mut region = StringBuilder::new();
    let mut region_code = StringBuilder::new();
    let mut role_code = StringBuilder::new();
    let mut legal_start = Date32Builder::new();
    let mut legal_end = Date32Builder::new();
    let mut operational_start = Date32Builder::new();
    let mut operational_end = Date32Builder::new();
    let mut last_change_date = Date32Builder::new();

    for r in records {
        ods_code.append_value(&r.ods_code);
        record_class.append_value(&r.record_class);
        status.append_value(&r.status);
        role.append_value(&r.role);
        name.append_value(&r.name);

        if let Some(ref loc) = r.geo_loc {
            let mut parts = Vec::new();
            for line in &loc.address_lines {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    parts.push(trimmed.to_string());
                }
            }
            if let Some(ref t) = loc.town {
                let trimmed = t.trim();
                if !trimmed.is_empty() {
                    parts.push(trimmed.to_string());
                }
            }
            if let Some(ref p) = loc.postcode {
                let trimmed = p.trim();
                if !trimmed.is_empty() {
                    parts.push(trimmed.to_string());
                }
            }

            if parts.is_empty() {
                address.append_null();
            } else {
                address.append_value(parts.join(", "));
            }

            append_opt(&mut town, loc.town.as_deref());
            append_opt(&mut county, loc.county.as_deref());
            append_opt(&mut postcode, loc.postcode.as_deref());
            append_opt(&mut country, loc.country.as_deref());
            append_opt(&mut uprn, loc.uprn.as_deref());
        } else {
            address.append_null();
            town.append_null();
            county.append_null();
            postcode.append_null();
            country.append_null();
            uprn.append_null();
        }

        let mut tel_val = None;
        let mut http_val = None;
        for c in &r.contacts {
            match c.contact_type.as_str() {
                "tel" => { if tel_val.is_none() { tel_val = Some(&c.value); } }
                "http" => { if http_val.is_none() { http_val = Some(&c.value); } }
                _ => {}
            }
        }
        append_opt(&mut telephone, tel_val.map(|s| s.as_str()));
        append_opt(&mut website, http_val.map(|s| s.as_str()));

        append_opt(&mut commissioner, r.commissioner.as_deref());
        append_opt(&mut commissioner_code, r.commissioner_code.as_deref());
        append_opt(&mut parent, r.parent.as_deref());
        append_opt(&mut parent_code, r.parent_code.as_deref());
        append_opt(&mut pcn, r.pcn.as_deref());
        append_opt(&mut pcn_code, r.pcn_code.as_deref());
        append_opt(&mut trust, r.trust.as_deref());
        append_opt(&mut trust_code, r.trust_code.as_deref());
        append_opt(&mut icb, r.icb.as_deref());
        append_opt(&mut icb_code, r.icb_code.as_deref());
        append_opt(&mut region, r.region.as_deref());
        append_opt(&mut region_code, r.region_code.as_deref());

        let primary_role_id = r.roles.iter()
            .find(|role| role.primary_role)
            .map(|role| role.id.as_str())
            .unwrap_or("");
        role_code.append_value(primary_role_id);

        let (l_start, l_end, o_start, o_end) = extract_dates(&r.dates);
        append_date(&mut legal_start, l_start.as_deref());
        append_date(&mut legal_end, l_end.as_deref());
        append_date(&mut operational_start, o_start.as_deref());
        append_date(&mut operational_end, o_end.as_deref());
        append_date(&mut last_change_date, r.last_change_date.as_deref());
    }

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(ods_code.finish()) as ArrayRef,
            Arc::new(record_class.finish()) as ArrayRef,
            Arc::new(status.finish()) as ArrayRef,
            Arc::new(role.finish()) as ArrayRef,
            Arc::new(role_code.finish()) as ArrayRef,
            Arc::new(name.finish()) as ArrayRef,
            Arc::new(address.finish()) as ArrayRef,
            Arc::new(town.finish()) as ArrayRef,
            Arc::new(county.finish()) as ArrayRef,
            Arc::new(postcode.finish()) as ArrayRef,
            Arc::new(country.finish()) as ArrayRef,
            Arc::new(uprn.finish()) as ArrayRef,
            Arc::new(telephone.finish()) as ArrayRef,
            Arc::new(website.finish()) as ArrayRef,
            Arc::new(commissioner.finish()) as ArrayRef,
            Arc::new(commissioner_code.finish()) as ArrayRef,
            Arc::new(parent.finish()) as ArrayRef,
            Arc::new(parent_code.finish()) as ArrayRef,
            Arc::new(pcn.finish()) as ArrayRef,
            Arc::new(pcn_code.finish()) as ArrayRef,
            Arc::new(trust.finish()) as ArrayRef,
            Arc::new(trust_code.finish()) as ArrayRef,
            Arc::new(icb.finish()) as ArrayRef,
            Arc::new(icb_code.finish()) as ArrayRef,
            Arc::new(region.finish()) as ArrayRef,
            Arc::new(region_code.finish()) as ArrayRef,
            Arc::new(legal_start.finish()) as ArrayRef,
            Arc::new(legal_end.finish()) as ArrayRef,
            Arc::new(operational_start.finish()) as ArrayRef,
            Arc::new(operational_end.finish()) as ArrayRef,
            Arc::new(last_change_date.finish()) as ArrayRef,
        ],
    )
    .context("building Arrow orgs batch")?;

    Ok(batch)
}

pub fn export_orgs(output_dir: &Path, records: &[OdsRecord], provenance: Option<&crate::provenance::OdsProvenance>) -> Result<()> {
    let mut active_records: Vec<&OdsRecord> = records.iter()
        .filter(|r| r.status == "active")
        .collect();
    active_records.sort_by_key(|r| &r.ods_code);

    let schema = embed_metadata(&orgs_schema(), provenance);
    let output_file = File::create(output_dir.join("orgs.parquet"))
        .context("creating orgs.parquet")?;
    let props = writer_properties(provenance);
    let mut writer = ArrowWriter::try_new(output_file, schema.clone(), Some(props))
        .context("creating orgs ArrowWriter")?;

    for chunk in active_records.chunks(BATCH_SIZE) {
        let batch = build_orgs_batch(&schema, chunk)?;
        writer.write(&batch).context("writing orgs batch")?;
    }
    writer.close().context("finalising orgs writer")?;
    println!("Exported {} active records to orgs.parquet.", active_records.len());
    Ok(())
}

pub fn export_orgs_all(output_dir: &Path, records: &[OdsRecord], provenance: Option<&crate::provenance::OdsProvenance>) -> Result<()> {
    let mut sorted_records: Vec<&OdsRecord> = records.iter().collect();
    sorted_records.sort_by(|a, b| {
        let a_active = a.status == "active";
        let b_active = b.status == "active";
        match b_active.cmp(&a_active) {
            std::cmp::Ordering::Equal => a.ods_code.cmp(&b.ods_code),
            other => other,
        }
    });

    let schema = embed_metadata(&orgs_schema(), provenance);
    let output_file = File::create(output_dir.join("orgs_all.parquet"))
        .context("creating orgs_all.parquet")?;
    let props = writer_properties(provenance);
    let mut writer = ArrowWriter::try_new(output_file, schema.clone(), Some(props))
        .context("creating orgs_all ArrowWriter")?;

    for chunk in sorted_records.chunks(BATCH_SIZE) {
        let batch = build_orgs_batch(&schema, chunk)?;
        writer.write(&batch).context("writing orgs_all batch")?;
    }
    writer.close().context("finalising orgs_all writer")?;
    println!("Exported {} historical records to orgs_all.parquet.", sorted_records.len());
    Ok(())
}

// ==========================================
// roles.parquet
// ==========================================

#[derive(Clone)]
struct RoleRow {
    role: String,
    is_primary: bool,
    status: String,
    ods_code: String,
    role_code: String,
    legal_start: Option<String>,
    legal_end: Option<String>,
    operational_start: Option<String>,
    operational_end: Option<String>,
}

fn roles_schema() -> Schema {
    Schema::new(vec![
        Field::new("role", DataType::Utf8, false),
        Field::new("is_primary", DataType::Boolean, false),
        Field::new("status", DataType::Utf8, false),
        Field::new("ods_code", DataType::Utf8, false),
        Field::new("role_code", DataType::Utf8, false),
        Field::new("legal_start", DataType::Date32, true),
        Field::new("legal_end", DataType::Date32, true),
        Field::new("operational_start", DataType::Date32, true),
        Field::new("operational_end", DataType::Date32, true),
    ])
}

fn build_roles_batch(schema: &Arc<Schema>, rows: &[RoleRow]) -> Result<RecordBatch> {
    let mut role = StringBuilder::new();
    let mut is_primary = BooleanBuilder::new();
    let mut status = StringBuilder::new();
    let mut ods_code = StringBuilder::new();
    let mut role_code = StringBuilder::new();
    let mut legal_start = Date32Builder::new();
    let mut legal_end = Date32Builder::new();
    let mut operational_start = Date32Builder::new();
    let mut operational_end = Date32Builder::new();

    for r in rows {
        role.append_value(&r.role);
        is_primary.append_value(r.is_primary);
        status.append_value(&r.status);
        ods_code.append_value(&r.ods_code);
        role_code.append_value(&r.role_code);
        append_date(&mut legal_start, r.legal_start.as_deref());
        append_date(&mut legal_end, r.legal_end.as_deref());
        append_date(&mut operational_start, r.operational_start.as_deref());
        append_date(&mut operational_end, r.operational_end.as_deref());
    }

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(role.finish()) as ArrayRef,
            Arc::new(is_primary.finish()) as ArrayRef,
            Arc::new(status.finish()) as ArrayRef,
            Arc::new(ods_code.finish()) as ArrayRef,
            Arc::new(role_code.finish()) as ArrayRef,
            Arc::new(legal_start.finish()) as ArrayRef,
            Arc::new(legal_end.finish()) as ArrayRef,
            Arc::new(operational_start.finish()) as ArrayRef,
            Arc::new(operational_end.finish()) as ArrayRef,
        ],
    )
    .context("building Arrow roles batch")?;

    Ok(batch)
}

pub fn export_roles(output_dir: &Path, records: &[OdsRecord], provenance: Option<&crate::provenance::OdsProvenance>) -> Result<()> {
    let mut rows = Vec::new();
    for r in records {
        for role_record in &r.roles {
            let (l_start, l_end, o_start, o_end) = extract_dates(&role_record.dates);

            rows.push(RoleRow {
                role: role_record.display_name.clone().unwrap_or_else(|| role_record.id.clone()),
                is_primary: role_record.primary_role,
                status: role_record.status.clone(),
                ods_code: r.ods_code.clone(),
                role_code: role_record.id.clone(),
                legal_start: l_start,
                legal_end: l_end,
                operational_start: o_start,
                operational_end: o_end,
            });
        }
    }

    // Sort: ods_code ASC, operational_start DESC
    rows.sort_by(|a, b| {
        match a.ods_code.cmp(&b.ods_code) {
            std::cmp::Ordering::Equal => {
                b.operational_start.cmp(&a.operational_start)
            }
            other => other,
        }
    });

    let schema = embed_metadata(&roles_schema(), provenance);
    let output_file = File::create(output_dir.join("roles.parquet"))
        .context("creating roles.parquet")?;
    let props = writer_properties(provenance);
    let mut writer = ArrowWriter::try_new(output_file, schema.clone(), Some(props))
        .context("creating roles ArrowWriter")?;

    for chunk in rows.chunks(BATCH_SIZE) {
        let batch = build_roles_batch(&schema, chunk)?;
        writer.write(&batch).context("writing roles batch")?;
    }
    writer.close().context("finalising roles writer")?;
    println!("Exported {} records to roles.parquet.", rows.len());
    Ok(())
}

// ==========================================
// rels.parquet
// ==========================================

#[derive(Clone)]
struct RelRow {
    rel_type: String,
    status: String,
    target: Option<String>,
    source: String,
    target_code: String,
    source_code: String,
    rel_type_code: String,
    legal_start: Option<String>,
    legal_end: Option<String>,
    operational_start: Option<String>,
    operational_end: Option<String>,
}

fn rels_schema() -> Schema {
    Schema::new(vec![
        Field::new("rel_type", DataType::Utf8, false),
        Field::new("status", DataType::Utf8, false),
        Field::new("target", DataType::Utf8, true),
        Field::new("source", DataType::Utf8, false),
        Field::new("target_code", DataType::Utf8, false),
        Field::new("source_code", DataType::Utf8, false),
        Field::new("rel_type_code", DataType::Utf8, false),
        Field::new("legal_start", DataType::Date32, true),
        Field::new("legal_end", DataType::Date32, true),
        Field::new("operational_start", DataType::Date32, true),
        Field::new("operational_end", DataType::Date32, true),
    ])
}

fn build_rels_batch(schema: &Arc<Schema>, rows: &[RelRow]) -> Result<RecordBatch> {
    let mut rel_type = StringBuilder::new();
    let mut status = StringBuilder::new();
    let mut target = StringBuilder::new();
    let mut source = StringBuilder::new();
    let mut target_code = StringBuilder::new();
    let mut source_code = StringBuilder::new();
    let mut rel_type_code = StringBuilder::new();
    let mut legal_start = Date32Builder::new();
    let mut legal_end = Date32Builder::new();
    let mut operational_start = Date32Builder::new();
    let mut operational_end = Date32Builder::new();

    for r in rows {
        rel_type.append_value(&r.rel_type);
        status.append_value(&r.status);
        append_opt(&mut target, r.target.as_deref());
        source.append_value(&r.source);
        target_code.append_value(&r.target_code);
        source_code.append_value(&r.source_code);
        rel_type_code.append_value(&r.rel_type_code);
        append_date(&mut legal_start, r.legal_start.as_deref());
        append_date(&mut legal_end, r.legal_end.as_deref());
        append_date(&mut operational_start, r.operational_start.as_deref());
        append_date(&mut operational_end, r.operational_end.as_deref());
    }

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(rel_type.finish()) as ArrayRef,
            Arc::new(status.finish()) as ArrayRef,
            Arc::new(target.finish()) as ArrayRef,
            Arc::new(source.finish()) as ArrayRef,
            Arc::new(target_code.finish()) as ArrayRef,
            Arc::new(source_code.finish()) as ArrayRef,
            Arc::new(rel_type_code.finish()) as ArrayRef,
            Arc::new(legal_start.finish()) as ArrayRef,
            Arc::new(legal_end.finish()) as ArrayRef,
            Arc::new(operational_start.finish()) as ArrayRef,
            Arc::new(operational_end.finish()) as ArrayRef,
        ],
    )
    .context("building Arrow rels batch")?;

    Ok(batch)
}

pub fn export_rels(output_dir: &Path, records: &[OdsRecord], provenance: Option<&crate::provenance::OdsProvenance>) -> Result<()> {
    let mut rows = Vec::new();
    for r in records {
        for rel in &r.relationships {
            let (l_start, l_end, o_start, o_end) = extract_dates(&rel.dates);

            rows.push(RelRow {
                rel_type: rel.display_name.clone().unwrap_or_else(|| rel.id.clone()),
                status: rel.status.clone(),
                target: rel.target.name.clone(),
                source: r.name.clone(),
                target_code: rel.target.ods_code.clone(),
                source_code: r.ods_code.clone(),
                rel_type_code: rel.id.clone(),
                legal_start: l_start,
                legal_end: l_end,
                operational_start: o_start,
                operational_end: o_end,
            });
        }
        for succ in &r.successors {
            let (l_start, l_end, o_start, o_end) = extract_dates(&succ.dates);

            rows.push(RelRow {
                rel_type: succ.succ_type.clone(),
                status: "active".to_string(),
                target: succ.target.name.clone(),
                source: r.name.clone(),
                target_code: succ.target.ods_code.clone(),
                source_code: r.ods_code.clone(),
                rel_type_code: "SUCCESSOR".to_string(),
                legal_start: l_start,
                legal_end: l_end,
                operational_start: o_start,
                operational_end: o_end,
            });
        }
    }

    // Sort: target_code ASC, rel_type ASC
    rows.sort_by(|a, b| {
        match a.target_code.cmp(&b.target_code) {
            std::cmp::Ordering::Equal => {
                a.rel_type.cmp(&b.rel_type)
            }
            other => other,
        }
    });

    let schema = embed_metadata(&rels_schema(), provenance);
    let output_file = File::create(output_dir.join("rels.parquet"))
        .context("creating rels.parquet")?;
    let props = writer_properties(provenance);
    let mut writer = ArrowWriter::try_new(output_file, schema.clone(), Some(props))
        .context("creating rels ArrowWriter")?;

    for chunk in rows.chunks(BATCH_SIZE) {
        let batch = build_rels_batch(&schema, chunk)?;
        writer.write(&batch).context("writing rels batch")?;
    }
    writer.close().context("finalising rels writer")?;
    println!("Exported {} records to rels.parquet.", rows.len());
    Ok(())
}

fn walk_successors(
    start_code: &str,
    adj: &HashMap<String, Vec<String>>,
    status_map: &HashMap<String, String>,
    name_map: &HashMap<String, String>,
    visited: &mut HashSet<String>,
    current_path: &mut Vec<String>,
    results: &mut Vec<(Option<String>, Option<String>, String)>,
) {
    if !visited.insert(start_code.to_string()) {
        // Cycle detected! Terminate the path.
        let mut path_copy = current_path.clone();
        path_copy.push(start_code.to_string());
        let chain = path_copy.join(" -> ");
        results.push((None, None, chain));
        return;
    }

    current_path.push(start_code.to_string());

    let successors = adj.get(start_code);
    if let Some(succs) = successors {
        if succs.is_empty() {
            // Dead end
            let chain = current_path.join(" -> ");
            results.push((None, None, chain));
        } else {
            for next_code in succs {
                let status = status_map.get(next_code).map(|s| s.as_str()).unwrap_or("inactive");
                if status == "active" {
                    // Reached active successor!
                    let mut path_copy = current_path.clone();
                    path_copy.push(next_code.to_string());
                    let chain = path_copy.join(" -> ");
                    let name = name_map.get(next_code).cloned();
                    results.push((Some(next_code.clone()), name, chain));
                } else {
                    // Recursively walk inactive successor
                    walk_successors(next_code, adj, status_map, name_map, visited, current_path, results);
                }
            }
        }
    } else {
        // Dead end (no successor links)
        let chain = current_path.join(" -> ");
        results.push((None, None, chain));
    }

    current_path.pop();
    visited.remove(start_code);
}

struct SuccessorRow {
    ods_code: String,
    name: String,
    successor_code: Option<String>,
    successor: Option<String>,
    succession_chain: String,
}

fn successors_schema() -> Schema {
    Schema::new(vec![
        Field::new("ods_code", DataType::Utf8, false),
        Field::new("name", DataType::Utf8, false),
        Field::new("successor_code", DataType::Utf8, true),
        Field::new("successor", DataType::Utf8, true),
        Field::new("succession_chain", DataType::Utf8, false),
    ])
}

fn build_successors_batch(schema: &Arc<Schema>, rows: &[SuccessorRow]) -> Result<RecordBatch> {
    let mut ods_code = StringBuilder::new();
    let mut name = StringBuilder::new();
    let mut successor_code = StringBuilder::new();
    let mut successor = StringBuilder::new();
    let mut succession_chain = StringBuilder::new();

    for r in rows {
        ods_code.append_value(&r.ods_code);
        name.append_value(&r.name);
        append_opt(&mut successor_code, r.successor_code.as_deref());
        append_opt(&mut successor, r.successor.as_deref());
        succession_chain.append_value(&r.succession_chain);
    }

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(ods_code.finish()) as ArrayRef,
            Arc::new(name.finish()) as ArrayRef,
            Arc::new(successor_code.finish()) as ArrayRef,
            Arc::new(successor.finish()) as ArrayRef,
            Arc::new(succession_chain.finish()) as ArrayRef,
        ],
    )
    .context("building Arrow successors batch")?;

    Ok(batch)
}

pub fn export_successors(output_dir: &Path, records: &[OdsRecord], provenance: Option<&crate::provenance::OdsProvenance>) -> Result<()> {
    // 1. Build adjacency map, status map, and name map
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();
    let mut status_map = HashMap::new();
    let mut name_map = HashMap::new();

    for r in records {
        status_map.insert(r.ods_code.clone(), r.status.clone());
        name_map.insert(r.ods_code.clone(), r.name.clone());

        for succ in &r.successors {
            let succ_type = succ.succ_type.to_lowercase();
            if succ_type.contains("successor") {
                adj.entry(r.ods_code.clone())
                    .or_default()
                    .push(succ.target.ods_code.clone());
            } else if succ_type.contains("predecessor") {
                adj.entry(succ.target.ods_code.clone()).or_default().push(r.ods_code.clone());
            }
        }
    }

    // 2. Walk all inactive records
    let mut rows = Vec::new();
    for r in records {
        if r.status != "active" {
            let mut visited = std::collections::HashSet::new();
            let mut current_path = Vec::new();
            let mut results = Vec::new();

            walk_successors(
                &r.ods_code,
                &adj,
                &status_map,
                &name_map,
                &mut visited,
                &mut current_path,
                &mut results,
            );

            for (succ_code, succ_name, chain) in results {
                rows.push(SuccessorRow {
                    ods_code: r.ods_code.clone(),
                    name: r.name.clone(),
                    successor_code: succ_code,
                    successor: succ_name,
                    succession_chain: chain,
                });
            }
        }
    }

    // Sort: ods_code ASC, succession_chain ASC
    rows.sort_by(|a, b| {
        match a.ods_code.cmp(&b.ods_code) {
            std::cmp::Ordering::Equal => a.succession_chain.cmp(&b.succession_chain),
            other => other,
        }
    });

    let schema = embed_metadata(&successors_schema(), provenance);
    let output_file = File::create(output_dir.join("successors.parquet"))
        .context("creating successors.parquet")?;
    let props = writer_properties(provenance);
    let mut writer = ArrowWriter::try_new(output_file, schema.clone(), Some(props))
        .context("creating successors ArrowWriter")?;

    for chunk in rows.chunks(BATCH_SIZE) {
        let batch = build_successors_batch(&schema, chunk)?;
        writer.write(&batch).context("writing successors batch")?;
    }
    writer.close().context("finalising successors writer")?;
    println!("Exported {} records to successors.parquet.", rows.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::compile::Location;

    #[test]
    fn test_address_consolidation() {
        let record = OdsRecord {
            ods_code: "Y01234".to_string(),
            name: "Test Practice".to_string(),
            status: "active".to_string(),
            role: "gp practice".to_string(),
            parent_organisation: None,
            region_code: None,
            root: None,
            assigning_authority_name: None,
            record_class: "site".to_string(),
            last_change_date: None,
            dates: vec![
                crate::commands::compile::OdsDate {
                    date_type: "Legal".to_string(),
                    start: Some("2006-10-01".to_string()),
                    end: Some("2013-03-31".to_string()),
                },
                crate::commands::compile::OdsDate {
                    date_type: "Operational".to_string(),
                    start: Some("2006-10-01".to_string()),
                    end: Some("2022-09-30".to_string()),
                },
            ],
            geo_loc: Some(Location {
                address_lines: vec![
                    "Suite 4".to_string(),
                    "Albert House".to_string(),
                    "12 Gresham Road".to_string(),
                ],
                town: Some("London".to_string()),
                county: Some("Greater London".to_string()),
                postcode: Some("SW9 7AY".to_string()),
                country: Some("England".to_string()),
                uprn: None,
            }),
            contacts: vec![],
            roles: vec![],
            relationships: vec![],
            successors: vec![
                crate::commands::compile::OdsSuccessor {
                    unique_succ_id: "777".to_string(),
                    succ_type: "Predecessor".to_string(),
                    dates: vec![
                        crate::commands::compile::OdsDate {
                            date_type: "Legal".to_string(),
                            start: Some("2006-10-01".to_string()),
                            end: None,
                        }
                    ],
                    target: crate::commands::compile::OdsRelationshipTarget {
                        ods_code: "5FD51".to_string(),
                        name: Some("NHS Predecessor Org".to_string()),
                        root: None,
                        assigning_authority_name: None,
                        primary_role_id: None,
                        primary_role_display_name: None,
                        primary_role_unique_role_id: None,
                    }
                }
            ],
            commissioner: None,
            commissioner_code: None,
            parent: None,
            parent_code: None,
            start_date: None,
            end_date: None,
            ..Default::default()
        };

        let schema = Arc::new(orgs_schema());
        let batch = build_orgs_batch(&schema, &[&record]).unwrap();

        // 1. Verify schema has "address" and does not have "address_line_1/2/3"
        assert!(schema.column_with_name("address").is_some());
        assert!(schema.column_with_name("address_line_1").is_none());

        // 2. Verify record_class field
        assert!(schema.column_with_name("record_class").is_some());

        // 3. Verify commissioner / parent field presence
        assert!(schema.column_with_name("commissioner").is_some());
        assert!(schema.column_with_name("commissioner_code").is_some());
        assert!(schema.column_with_name("parent").is_some());
        assert!(schema.column_with_name("parent_code").is_some());

        // 4. Verify PCN/Trust/ICB hierarchy fields are present
        assert!(schema.column_with_name("pcn").is_some());
        assert!(schema.column_with_name("pcn_code").is_some());
        assert!(schema.column_with_name("trust").is_some());
        assert!(schema.column_with_name("trust_code").is_some());
        assert!(schema.column_with_name("icb").is_some());
        assert!(schema.column_with_name("icb_code").is_some());

        // 5. Verify postcode renamed
        assert!(schema.column_with_name("postcode").is_some());

        // 6. Verify the value of "address" field
        let address_col = batch
            .column(schema.index_of("address").unwrap())
            .as_any()
            .downcast_ref::<arrow::array::StringArray>()
            .unwrap();

        assert_eq!(
            address_col.value(0),
            "Suite 4, Albert House, 12 Gresham Road, London, SW9 7AY"
        );

        // 7. Verify new date columns exist
        assert!(schema.column_with_name("legal_start").is_some());
        assert!(schema.column_with_name("legal_end").is_some());
    }

    #[test]
    fn test_successor_resolution() {
        // Mock a set of records for:
        // A1 (Inactive) -> successor B2
        // B2 (Inactive) -> successor C3
        // C3 (Active)
        // D4 (Inactive) -> successor E5 (dead-end inactive)
        // F6 (Inactive) -> split to G7 and H8
        // G7 (Active)
        // H8 (Active)
        // Loop: L1 -> L2 -> L1

        let mut records = vec![
            OdsRecord {
                ods_code: "A1".to_string(),
                name: "Old A1".to_string(),
                status: "Inactive".to_string(),
                role: "GP".to_string(),
                record_class: "org".to_string(),
                successors: vec![
                    crate::commands::compile::OdsSuccessor {
                        unique_succ_id: "1".to_string(),
                        succ_type: "Successor".to_string(),
                        dates: vec![],
                        target: crate::commands::compile::OdsRelationshipTarget {
                            ods_code: "B2".to_string(),
                            name: Some("Old B2".to_string()),
                            root: None,
                            assigning_authority_name: None,
                            primary_role_id: None,
                            primary_role_display_name: None,
                            primary_role_unique_role_id: None,
                        }
                    }
                ],
                ..Default::default()
            },
            OdsRecord {
                ods_code: "B2".to_string(),
                name: "Old B2".to_string(),
                status: "Inactive".to_string(),
                role: "GP".to_string(),
                record_class: "org".to_string(),
                successors: vec![
                    crate::commands::compile::OdsSuccessor {
                        unique_succ_id: "2".to_string(),
                        succ_type: "Successor".to_string(),
                        dates: vec![],
                        target: crate::commands::compile::OdsRelationshipTarget {
                            ods_code: "C3".to_string(),
                            name: Some("Active C3".to_string()),
                            root: None,
                            assigning_authority_name: None,
                            primary_role_id: None,
                            primary_role_display_name: None,
                            primary_role_unique_role_id: None,
                        }
                    }
                ],
                ..Default::default()
            },
            OdsRecord {
                ods_code: "C3".to_string(),
                name: "Active C3".to_string(),
                status: "Active".to_string(),
                role: "GP".to_string(),
                record_class: "Organisation".to_string(),
                ..Default::default()
            },
            OdsRecord {
                ods_code: "D4".to_string(),
                name: "Old D4".to_string(),
                status: "Inactive".to_string(),
                role: "GP".to_string(),
                record_class: "Organisation".to_string(),
                successors: vec![
                    crate::commands::compile::OdsSuccessor {
                        unique_succ_id: "3".to_string(),
                        succ_type: "Successor".to_string(),
                        dates: vec![],
                        target: crate::commands::compile::OdsRelationshipTarget {
                            ods_code: "E5".to_string(),
                            name: Some("Dead End E5".to_string()),
                            root: None,
                            assigning_authority_name: None,
                            primary_role_id: None,
                            primary_role_display_name: None,
                            primary_role_unique_role_id: None,
                        }
                    }
                ],
                ..Default::default()
            },
            OdsRecord {
                ods_code: "F6".to_string(),
                name: "Old F6".to_string(),
                status: "Inactive".to_string(),
                role: "GP".to_string(),
                record_class: "Organisation".to_string(),
                successors: vec![
                    crate::commands::compile::OdsSuccessor {
                        unique_succ_id: "4".to_string(),
                        succ_type: "Successor".to_string(),
                        dates: vec![],
                        target: crate::commands::compile::OdsRelationshipTarget {
                            ods_code: "G7".to_string(),
                            name: Some("Active G7".to_string()),
                            root: None,
                            assigning_authority_name: None,
                            primary_role_id: None,
                            primary_role_display_name: None,
                            primary_role_unique_role_id: None,
                        }
                    },
                    crate::commands::compile::OdsSuccessor {
                        unique_succ_id: "5".to_string(),
                        succ_type: "Successor".to_string(),
                        dates: vec![],
                        target: crate::commands::compile::OdsRelationshipTarget {
                            ods_code: "H8".to_string(),
                            name: Some("Active H8".to_string()),
                            root: None,
                            assigning_authority_name: None,
                            primary_role_id: None,
                            primary_role_display_name: None,
                            primary_role_unique_role_id: None,
                        }
                    }
                ],
                ..Default::default()
            },
            OdsRecord {
                ods_code: "G7".to_string(),
                name: "Active G7".to_string(),
                status: "Active".to_string(),
                role: "GP".to_string(),
                record_class: "Organisation".to_string(),
                ..Default::default()
            },
            OdsRecord {
                ods_code: "H8".to_string(),
                name: "Active H8".to_string(),
                status: "Active".to_string(),
                role: "GP".to_string(),
                record_class: "Organisation".to_string(),
                ..Default::default()
            },
            OdsRecord {
                ods_code: "L1".to_string(),
                name: "Loop L1".to_string(),
                status: "Inactive".to_string(),
                role: "GP".to_string(),
                record_class: "Organisation".to_string(),
                successors: vec![
                    crate::commands::compile::OdsSuccessor {
                        unique_succ_id: "6".to_string(),
                        succ_type: "Successor".to_string(),
                        dates: vec![],
                        target: crate::commands::compile::OdsRelationshipTarget {
                            ods_code: "L2".to_string(),
                            name: Some("Loop L2".to_string()),
                            root: None,
                            assigning_authority_name: None,
                            primary_role_id: None,
                            primary_role_display_name: None,
                            primary_role_unique_role_id: None,
                        }
                    }
                ],
                ..Default::default()
            },
            OdsRecord {
                ods_code: "L2".to_string(),
                name: "Loop L2".to_string(),
                status: "Inactive".to_string(),
                role: "GP".to_string(),
                record_class: "Organisation".to_string(),
                successors: vec![
                    crate::commands::compile::OdsSuccessor {
                        unique_succ_id: "7".to_string(),
                        succ_type: "Successor".to_string(),
                        dates: vec![],
                        target: crate::commands::compile::OdsRelationshipTarget {
                            ods_code: "L1".to_string(),
                            name: Some("Loop L1".to_string()),
                            root: None,
                            assigning_authority_name: None,
                            primary_role_id: None,
                            primary_role_display_name: None,
                            primary_role_unique_role_id: None,
                        }
                    }
                ],
                ..Default::default()
            },
        ];

        // Normalize mock records to lowercase status/role/record_class to match Option B
        for r in &mut records {
            r.status = r.status.to_lowercase();
            r.role = r.role.to_lowercase();
            r.record_class = r.record_class.to_lowercase();
            for succ in &mut r.successors {
                succ.succ_type = succ.succ_type.to_lowercase();
            }
        }

        // Setup adjacency/status/names
        let mut adj: HashMap<String, Vec<String>> = HashMap::new();
        let mut status_map = HashMap::new();
        let mut name_map = HashMap::new();

        for r in &records {
            status_map.insert(r.ods_code.clone(), r.status.clone());
            name_map.insert(r.ods_code.clone(), r.name.clone());
            for succ in &r.successors {
                if succ.succ_type == "successor" {
                    adj.entry(r.ods_code.clone()).or_default().push(succ.target.ods_code.clone());
                }
            }
        }

        // Test walks
        let mut rows = Vec::new();
        for r in &records {
            if r.status != "active" {
                let mut visited = std::collections::HashSet::new();
                let mut current_path = Vec::new();
                let mut results = Vec::new();

                walk_successors(
                    &r.ods_code,
                    &adj,
                    &status_map,
                    &name_map,
                    &mut visited,
                    &mut current_path,
                    &mut results,
                );

                for (succ_code, succ_name, chain) in results {
                    rows.push(SuccessorRow {
                        ods_code: r.ods_code.clone(),
                        name: r.name.clone(),
                        successor_code: succ_code,
                        successor: succ_name,
                        succession_chain: chain,
                    });
                }
            }
        }

        // 1. Verify A1 resolves to C3
        let row_a1 = rows.iter().find(|row| row.ods_code == "A1").unwrap();
        assert_eq!(row_a1.successor_code.as_deref(), Some("C3"));
        assert_eq!(row_a1.succession_chain, "A1 -> B2 -> C3");

        // 2. Verify D4 dead-ends with NULL
        let row_d4 = rows.iter().find(|row| row.ods_code == "D4").unwrap();
        assert_eq!(row_d4.successor_code, None);
        assert_eq!(row_d4.succession_chain, "D4 -> E5");

        // 3. Verify F6 splits into two rows
        let rows_f6: Vec<&SuccessorRow> = rows.iter().filter(|row| row.ods_code == "F6").collect();
        assert_eq!(rows_f6.len(), 2);
        assert_eq!(rows_f6[0].successor_code.as_deref(), Some("G7"));
        assert_eq!(rows_f6[0].succession_chain, "F6 -> G7");
        assert_eq!(rows_f6[1].successor_code.as_deref(), Some("H8"));
        assert_eq!(rows_f6[1].succession_chain, "F6 -> H8");

        // 4. Verify L1 loops terminate safely
        let row_l1 = rows.iter().find(|row| row.ods_code == "L1").unwrap();
        assert_eq!(row_l1.successor_code, None);
        assert_eq!(row_l1.succession_chain, "L1 -> L2 -> L1");
    }
}

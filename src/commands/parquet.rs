use anyhow::{Context, Result};
use arrow::array::{ArrayRef, BooleanBuilder, StringBuilder};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use clap::Parser;
use parquet::arrow::arrow_writer::ArrowWriter;
use parquet::file::properties::WriterProperties;
use std::fs::File;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::ndjson::OdsRecord;

const BATCH_SIZE: usize = 50_000;

#[derive(Parser, Debug)]
pub struct Args {
    /// NDJSON input file
    #[arg(long, short)]
    pub input: PathBuf,

    /// Output Parquet directory path
    #[arg(long, short)]
    pub output: PathBuf,
}

pub fn run(args: Args) -> Result<()> {
    let records: Vec<OdsRecord> = if args.input.extension().map_or(false, |ext| ext == "ndjson") {
        let input_file = File::open(&args.input)
            .with_context(|| format!("opening NDJSON input: {}", args.input.display()))?;
        let reader = std::io::BufReader::new(input_file);

        let mut recs = Vec::new();
        let mut _prov = None;
        for line in reader.lines() {
            let line = line.context("reading line from NDJSON")?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if _prov.is_none() {
                if let Some(p) = crate::provenance::try_parse_provenance_line(trimmed) {
                    _prov = Some(p);
                    continue;
                }
            }
            let record: OdsRecord = serde_json::from_str(trimmed)
                .with_context(|| format!("parsing JSON line: {}", trimmed))?;
            recs.push(record);
        }
        recs
    } else {
        let xml_path = super::ndjson::find_xml_file(&args.input)?;
        let (_prov, _concept_map, parsed) = super::ndjson::parse_single_pass(&xml_path)?;
        let resolved = super::ndjson::resolve_hierarchies(parsed);
        resolved.into_values().collect()
    };

    std::fs::create_dir_all(&args.output)
        .with_context(|| format!("creating output directory: {}", args.output.display()))?;

    // 1. Export orgs.parquet
    export_orgs(&args.output, &records)?;

    // 2. Export roles.parquet
    export_roles(&args.output, &records)?;

    // 3. Export rels.parquet
    export_rels(&args.output, &records)?;

    Ok(())
}

fn append_opt(builder: &mut StringBuilder, val: Option<&str>) {
    if let Some(v) = val {
        builder.append_value(v);
    } else {
        builder.append_null();
    }
}

fn extract_dates(dates: &[crate::commands::ndjson::OdsDate]) -> (Option<String>, Option<String>, Option<String>, Option<String>) {
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

fn orgs_schema() -> Schema {
    Schema::new(vec![
        Field::new("ods_code", DataType::Utf8, false),
        Field::new("record_class", DataType::Utf8, false),
        Field::new("status", DataType::Utf8, false),
        Field::new("role", DataType::Utf8, false),
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
        Field::new("pcn", DataType::Utf8, true),
        Field::new("trust", DataType::Utf8, true),
        Field::new("icb", DataType::Utf8, true),
        Field::new("role_code", DataType::Utf8, false),
        Field::new("parent_code", DataType::Utf8, true),
        Field::new("pcn_code", DataType::Utf8, true),
        Field::new("trust_code", DataType::Utf8, true),
        Field::new("icb_code", DataType::Utf8, true),
        Field::new("legal_start", DataType::Utf8, true),
        Field::new("legal_end", DataType::Utf8, true),
        Field::new("operational_start", DataType::Utf8, true),
        Field::new("operational_end", DataType::Utf8, true),
    ])
}

fn build_orgs_batch(schema: &Arc<Schema>, records: &[OdsRecord]) -> Result<RecordBatch> {
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
    let mut pcn = StringBuilder::new();
    let mut trust = StringBuilder::new();
    let mut icb = StringBuilder::new();
    let mut role_code = StringBuilder::new();
    let mut parent_code = StringBuilder::new();
    let mut pcn_code = StringBuilder::new();
    let mut trust_code = StringBuilder::new();
    let mut icb_code = StringBuilder::new();
    let mut legal_start = StringBuilder::new();
    let mut legal_end = StringBuilder::new();
    let mut operational_start = StringBuilder::new();
    let mut operational_end = StringBuilder::new();

    for r in records {
        ods_code.append_value(&r.ods_code);
        record_class.append_value(&r.record_class);
        status.append_value(&r.status);
        role.append_value(&r.role);
        name.append_value(&r.name);

        if let Some(ref loc) = r.geo_loc {
            let mut parts = Vec::new();
            for line in &loc.address_lines {
                let trimmed: &str = line.trim();
                if !trimmed.is_empty() {
                    parts.push(trimmed.to_string());
                }
            }
            if let Some(ref t) = loc.town {
                let trimmed: &str = t.trim();
                if !trimmed.is_empty() {
                    parts.push(trimmed.to_string());
                }
            }
            if let Some(ref p) = loc.postcode {
                let trimmed: &str = p.trim();
                if !trimmed.is_empty() {
                    parts.push(trimmed.to_string());
                }
            }

            if parts.is_empty() {
                address.append_null();
            } else {
                address.append_value(&parts.join(", "));
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
        append_opt(&mut telephone, tel_val.map(|s: &String| s.as_str()));
        append_opt(&mut website, http_val.map(|s: &String| s.as_str()));

        append_opt(&mut commissioner, r.commissioner.as_deref());
        append_opt(&mut commissioner_code, r.commissioner_code.as_deref());
        append_opt(&mut parent, r.parent.as_deref());
        append_opt(&mut pcn, r.pcn.as_deref());
        append_opt(&mut trust, r.trust.as_deref());
        append_opt(&mut icb, r.icb.as_deref());

        let primary_role_id = r.roles.iter()
            .find(|role| role.primary_role)
            .map(|role| role.id.as_str())
            .unwrap_or("");
        role_code.append_value(primary_role_id);

        append_opt(&mut parent_code, r.parent_organisation.as_ref().map(|p| p.ods_code.as_str()));
        append_opt(&mut pcn_code, r.pcn_code.as_deref());
        append_opt(&mut trust_code, r.trust_code.as_deref());
        append_opt(&mut icb_code, r.icb_code.as_deref());

        let (l_start, l_end, o_start, o_end) = extract_dates(&r.dates);
        append_opt(&mut legal_start, l_start.as_deref());
        append_opt(&mut legal_end, l_end.as_deref());
        append_opt(&mut operational_start, o_start.as_deref());
        append_opt(&mut operational_end, o_end.as_deref());
    }

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(ods_code.finish()) as ArrayRef,
            Arc::new(record_class.finish()) as ArrayRef,
            Arc::new(status.finish()) as ArrayRef,
            Arc::new(role.finish()) as ArrayRef,
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
            Arc::new(pcn.finish()) as ArrayRef,
            Arc::new(trust.finish()) as ArrayRef,
            Arc::new(icb.finish()) as ArrayRef,
            Arc::new(role_code.finish()) as ArrayRef,
            Arc::new(parent_code.finish()) as ArrayRef,
            Arc::new(pcn_code.finish()) as ArrayRef,
            Arc::new(trust_code.finish()) as ArrayRef,
            Arc::new(icb_code.finish()) as ArrayRef,
            Arc::new(legal_start.finish()) as ArrayRef,
            Arc::new(legal_end.finish()) as ArrayRef,
            Arc::new(operational_start.finish()) as ArrayRef,
            Arc::new(operational_end.finish()) as ArrayRef,
        ],
    )
    .context("building Arrow orgs batch")?;

    Ok(batch)
}

fn export_orgs(output_dir: &Path, records: &[OdsRecord]) -> Result<()> {
    let mut sorted_records = records.to_vec();
    sorted_records.sort_by(|a, b| {
        let a_active = a.status == "Active";
        let b_active = b.status == "Active";
        match b_active.cmp(&a_active) {
            std::cmp::Ordering::Equal => a.ods_code.cmp(&b.ods_code),
            other => other,
        }
    });

    let schema = Arc::new(orgs_schema());
    let output_file = File::create(output_dir.join("orgs.parquet"))
        .context("creating orgs.parquet")?;
    let props = WriterProperties::builder().build();
    let mut writer = ArrowWriter::try_new(output_file, schema.clone(), Some(props))
        .context("creating orgs ArrowWriter")?;

    for chunk in sorted_records.chunks(BATCH_SIZE) {
        let batch = build_orgs_batch(&schema, chunk)?;
        writer.write(&batch).context("writing orgs batch")?;
    }
    writer.close().context("finalising orgs writer")?;
    println!("Exported {} records to orgs.parquet.", sorted_records.len());
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
        Field::new("legal_start", DataType::Utf8, true),
        Field::new("legal_end", DataType::Utf8, true),
        Field::new("operational_start", DataType::Utf8, true),
        Field::new("operational_end", DataType::Utf8, true),
    ])
}

fn build_roles_batch(schema: &Arc<Schema>, rows: &[RoleRow]) -> Result<RecordBatch> {
    let mut role = StringBuilder::new();
    let mut is_primary = BooleanBuilder::new();
    let mut status = StringBuilder::new();
    let mut ods_code = StringBuilder::new();
    let mut role_code = StringBuilder::new();
    let mut legal_start = StringBuilder::new();
    let mut legal_end = StringBuilder::new();
    let mut operational_start = StringBuilder::new();
    let mut operational_end = StringBuilder::new();

    for r in rows {
        role.append_value(&r.role);
        is_primary.append_value(r.is_primary);
        status.append_value(&r.status);
        ods_code.append_value(&r.ods_code);
        role_code.append_value(&r.role_code);
        append_opt(&mut legal_start, r.legal_start.as_deref());
        append_opt(&mut legal_end, r.legal_end.as_deref());
        append_opt(&mut operational_start, r.operational_start.as_deref());
        append_opt(&mut operational_end, r.operational_end.as_deref());
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

fn export_roles(output_dir: &Path, records: &[OdsRecord]) -> Result<()> {
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

    let schema = Arc::new(roles_schema());
    let output_file = File::create(output_dir.join("roles.parquet"))
        .context("creating roles.parquet")?;
    let props = WriterProperties::builder().build();
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
        Field::new("legal_start", DataType::Utf8, true),
        Field::new("legal_end", DataType::Utf8, true),
        Field::new("operational_start", DataType::Utf8, true),
        Field::new("operational_end", DataType::Utf8, true),
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
    let mut legal_start = StringBuilder::new();
    let mut legal_end = StringBuilder::new();
    let mut operational_start = StringBuilder::new();
    let mut operational_end = StringBuilder::new();

    for r in rows {
        rel_type.append_value(&r.rel_type);
        status.append_value(&r.status);
        append_opt(&mut target, r.target.as_deref());
        source.append_value(&r.source);
        target_code.append_value(&r.target_code);
        source_code.append_value(&r.source_code);
        rel_type_code.append_value(&r.rel_type_code);
        append_opt(&mut legal_start, r.legal_start.as_deref());
        append_opt(&mut legal_end, r.legal_end.as_deref());
        append_opt(&mut operational_start, r.operational_start.as_deref());
        append_opt(&mut operational_end, r.operational_end.as_deref());
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

fn export_rels(output_dir: &Path, records: &[OdsRecord]) -> Result<()> {
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
                status: "Active".to_string(),
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

    let schema = Arc::new(rels_schema());
    let output_file = File::create(output_dir.join("rels.parquet"))
        .context("creating rels.parquet")?;
    let props = WriterProperties::builder().build();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::ndjson::Location;

    #[test]
    fn test_address_consolidation() {
        let record = OdsRecord {
            ods_code: "Y01234".to_string(),
            name: "Test Practice".to_string(),
            status: "Active".to_string(),
            role: "GP Practice".to_string(),
            parent_organisation: None,
            region_code: None,
            root: None,
            assigning_authority_name: None,
            record_class: "org".to_string(),
            last_change_date: None,
            dates: vec![
                crate::commands::ndjson::OdsDate {
                    date_type: "Legal".to_string(),
                    start: Some("2006-10-01".to_string()),
                    end: Some("2013-03-31".to_string()),
                },
                crate::commands::ndjson::OdsDate {
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
                crate::commands::ndjson::OdsSuccessor {
                    unique_succ_id: "777".to_string(),
                    succ_type: "Predecessor".to_string(),
                    dates: vec![
                        crate::commands::ndjson::OdsDate {
                            date_type: "Legal".to_string(),
                            start: Some("2006-10-01".to_string()),
                            end: None,
                        }
                    ],
                    target: crate::commands::ndjson::OdsRelationshipTarget {
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
            pcn: None,
            pcn_code: None,
            trust: None,
            trust_code: None,
            icb: None,
            icb_code: None,
            commissioner: None,
            commissioner_code: None,
            parent: None,
            parent_code: None,
            region: None,
            start_date: None,
            end_date: None,
        };

        let schema = Arc::new(orgs_schema());
        let batch = build_orgs_batch(&schema, &[record.clone()]).unwrap();

        // 1. Verify schema has "address" and does not have "address_line_1/2/3"
        assert!(schema.column_with_name("address").is_some());
        assert!(schema.column_with_name("address_line_1").is_none());
        assert!(schema.column_with_name("address_line_2").is_none());
        assert!(schema.column_with_name("address_line_3").is_none());

        // 2. Verify country is still in its own field
        assert!(schema.column_with_name("country").is_some());

        // 3. Verify fax is removed
        assert!(schema.column_with_name("fax").is_none());

        // 4. Verify postcode renamed
        assert!(schema.column_with_name("postcode").is_some());
        assert!(schema.column_with_name("post_code").is_none());

        // 5. Verify _name suffixes removed in orgs.parquet
        assert!(schema.column_with_name("role").is_some());
        assert!(schema.column_with_name("role_name").is_none());
        assert!(schema.column_with_name("parent").is_some());
        assert!(schema.column_with_name("parent_name").is_none());
        assert!(schema.column_with_name("pcn").is_some());
        assert!(schema.column_with_name("pcn_name").is_none());
        assert!(schema.column_with_name("trust").is_some());
        assert!(schema.column_with_name("trust_name").is_none());
        assert!(schema.column_with_name("icb").is_some());
        assert!(schema.column_with_name("icb_name").is_none());

        // 6. Verify rels.parquet schema updates
        let rels = Arc::new(rels_schema());
        assert!(rels.column_with_name("rel_type").is_some());
        assert!(rels.column_with_name("rel_type_name").is_none());
        assert!(rels.column_with_name("target").is_some());
        assert!(rels.column_with_name("target_name").is_none());
        assert!(rels.column_with_name("source").is_some());
        assert!(rels.column_with_name("source_name").is_none());
        assert!(rels.column_with_name("target_code").is_some());
        assert!(rels.column_with_name("target_ods_code").is_none());
        assert!(rels.column_with_name("source_code").is_some());
        assert!(rels.column_with_name("source_ods_code").is_none());

        // 3. Verify the value of "address" field (excluding country)
        let address_col = batch
            .column(schema.index_of("address").unwrap())
            .as_any()
            .downcast_ref::<arrow::array::StringArray>()
            .unwrap();

        assert_eq!(
            address_col.value(0),
            "Suite 4, Albert House, 12 Gresham Road, London, SW9 7AY"
        );

        // 7. Verify new date columns exist and end_date/start_date are gone
        assert!(schema.column_with_name("start_date").is_none());
        assert!(schema.column_with_name("end_date").is_none());
        assert!(schema.column_with_name("legal_start").is_some());
        assert!(schema.column_with_name("legal_end").is_some());
        assert!(schema.column_with_name("operational_start").is_some());
        assert!(schema.column_with_name("operational_end").is_some());

        // 8. Verify the extracted date values
        let legal_start_col = batch.column(schema.index_of("legal_start").unwrap()).as_any().downcast_ref::<arrow::array::StringArray>().unwrap();
        let legal_end_col = batch.column(schema.index_of("legal_end").unwrap()).as_any().downcast_ref::<arrow::array::StringArray>().unwrap();
        let operational_start_col = batch.column(schema.index_of("operational_start").unwrap()).as_any().downcast_ref::<arrow::array::StringArray>().unwrap();
        let operational_end_col = batch.column(schema.index_of("operational_end").unwrap()).as_any().downcast_ref::<arrow::array::StringArray>().unwrap();

        assert_eq!(legal_start_col.value(0), "2006-10-01");
        assert_eq!(legal_end_col.value(0), "2013-03-31");
        assert_eq!(operational_start_col.value(0), "2006-10-01");
        assert_eq!(operational_end_col.value(0), "2022-09-30");

        // 9. Verify successor is exported in rels
        let mut rel_rows = Vec::new();
        for succ in &record.successors {
            let (l_start, l_end, o_start, o_end) = extract_dates(&succ.dates);
            rel_rows.push(RelRow {
                rel_type: succ.succ_type.clone(),
                status: "Active".to_string(),
                target: succ.target.name.clone(),
                source: record.name.clone(),
                target_code: succ.target.ods_code.clone(),
                source_code: record.ods_code.clone(),
                rel_type_code: "SUCCESSOR".to_string(),
                legal_start: l_start,
                legal_end: l_end,
                operational_start: o_start,
                operational_end: o_end,
            });
        }
        let rel_batch = build_rels_batch(&rels, &rel_rows).unwrap();
        let rel_type_col = rel_batch.column(rels.index_of("rel_type").unwrap()).as_any().downcast_ref::<arrow::array::StringArray>().unwrap();
        let rel_type_code_col = rel_batch.column(rels.index_of("rel_type_code").unwrap()).as_any().downcast_ref::<arrow::array::StringArray>().unwrap();
        let target_code_col = rel_batch.column(rels.index_of("target_code").unwrap()).as_any().downcast_ref::<arrow::array::StringArray>().unwrap();
        let l_start_col = rel_batch.column(rels.index_of("legal_start").unwrap()).as_any().downcast_ref::<arrow::array::StringArray>().unwrap();

        assert_eq!(rel_type_col.value(0), "Predecessor");
        assert_eq!(rel_type_code_col.value(0), "SUCCESSOR");
        assert_eq!(target_code_col.value(0), "5FD51");
        assert_eq!(l_start_col.value(0), "2006-10-01");
    }
}


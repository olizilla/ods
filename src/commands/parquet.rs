use anyhow::{Context, Result};
use arrow::array::{ArrayRef, BooleanBuilder, Date32Builder, ListBuilder, StringBuilder};
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

use crate::commands::ndjson::OdsRecord;

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

fn embed_metadata(
    schema: &Schema,
    _prov: Option<&crate::provenance::OdsProvenance>,
) -> Arc<Schema> {
    Arc::new(schema.clone())
}

fn writer_properties(prov: Option<&crate::provenance::OdsProvenance>) -> WriterProperties {
    let meta_kv = if let Some(p) = prov {
        p.to_parquet_metadata()
            .into_iter()
            .map(|(k, v)| parquet::file::metadata::KeyValue {
                key: k,
                value: Some(v),
            })
            .collect()
    } else {
        vec![parquet::file::metadata::KeyValue {
            key: "ods.tool_version".to_string(),
            value: Some(env!("CARGO_PKG_VERSION").to_string()),
        }]
    };

    // Parquet Encoding Rationale:
    // 1. ZSTD at level 3 is explicitly pinned for deterministic cross-build compression and byte stability.
    // 2. Max row group size is set to 64,000. This aligns with our 50,000 BATCH_SIZE and splits large
    //    tables (orgs: 216k, orgs_all: 305k, rels: 662k) into multiple row groups to enable HTTP range-request
    //    pruning in DuckDB.
    // 3. Rows are pre-sorted by `ods_code` before export, giving non-overlapping min/max ranges per row group.
    //    Because min/max stats provide 100% selective pruning for `ods_code` lookups, bloom filters are omitted
    //    to avoid inflating file size without adding pruning benefit.
    WriterProperties::builder()
        .set_key_value_metadata(Some(meta_kv))
        .set_compression(parquet::basic::Compression::ZSTD(
            parquet::basic::ZstdLevel::try_new(3).expect("valid zstd level 3"),
        ))
        .set_max_row_group_size(64_000)
        .build()
}

pub fn run(args: Args) -> Result<()> {
    let (provenance, records): (Option<crate::provenance::OdsProvenance>, Vec<OdsRecord>) = if args
        .input
        .extension()
        .is_some_and(|ext| ext == "ndjson")
    {
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
        if let Some(ref mut p) = _prov {
            if p.dataset_version.is_none() {
                p.dataset_version = Some(crate::datapackage::dataset_version().to_string());
            }
        }
        (_prov, recs)
    } else {
        let archive_info = crate::archive::resolve_trud_archive(&args.input)?;

        let parent_prov = crate::provenance::OdsProvenance::load_from_dir(&args.input)
            .or_else(|| crate::provenance::OdsProvenance::load_from_dir(&archive_info.archive_path))
            .or_else(|| {
                crate::provenance::OdsProvenance::try_extract_trud_zip_provenance(
                    &archive_info.archive_path,
                )
            });

        if args.input.is_dir() {
            if let Some(ref prov) = parent_prov {
                if let Err(e) = prov.validate_baseline() {
                    anyhow::bail!("Invalid baseline _provenance.json in input '{}': {}. Did you run 'ods trud pull' first?", args.input.display(), e);
                }
            } else {
                anyhow::bail!("Missing _provenance.json in input directory '{}'. Did you run 'ods trud pull' first?", args.input.display());
            }
        }

        let xml_path = crate::commands::ndjson::find_xml_file(&archive_info.archive_path)?;
        let (mut prov, _concept_map, parsed) =
            crate::commands::ndjson::parse_single_pass(&xml_path)?;

        let actual_parsed_count = parsed.len();
        if let Some(declared_count) = prov.publication_record_count {
            if declared_count != actual_parsed_count {
                anyhow::bail!(
                    "✖ Manifest record count mismatch: declared {} != parsed {}",
                    declared_count,
                    actual_parsed_count
                );
            }
        }

        if let Some(parent) = parent_prov {
            if let Some(ref parent_date) = parent.trud_release_date {
                if parent_date != &archive_info.release_date {
                    anyhow::bail!(
                        "✖ Release date mismatch: _provenance.json specifies '{}' but archive filename specifies '{}'",
                        parent_date,
                        archive_info.release_date
                    );
                }
            }
            prov.trud_release_date = Some(archive_info.release_date);
            prov.trud_release_name = parent.trud_release_name.or(Some(archive_info.release_name));
            prov.trud_release_file = parent.trud_release_file.or(Some(archive_info.filename));
            if parent.trud_release_sha256.is_some() {
                prov.trud_release_sha256 = parent.trud_release_sha256;
            }
            if parent.trud_release_sha256_verified.is_some() {
                prov.trud_release_sha256_verified = parent.trud_release_sha256_verified;
            }
            if parent.trud_release_filesize_bytes.is_some() {
                prov.trud_release_filesize_bytes = parent.trud_release_filesize_bytes;
            }
            prov.dataset_version = parent.dataset_version.or_else(|| Some(crate::datapackage::dataset_version().to_string()));
        } else {
            prov.trud_release_date = Some(archive_info.release_date);
            prov.trud_release_name = Some(archive_info.release_name);
            prov.trud_release_file = Some(archive_info.filename);
            prov.dataset_version = Some(crate::datapackage::dataset_version().to_string());
            if let Ok(meta) = std::fs::metadata(&archive_info.archive_path) {
                prov.trud_release_filesize_bytes = Some(meta.len());
            }
            if let Ok(hash) = crate::provenance::compute_file_sha256(&archive_info.archive_path) {
                prov.trud_release_sha256 = Some(hash);
                prov.trud_release_sha256_verified =
                    Some(crate::provenance::TrudVerificationSource::TrudApi);
            }
        }
        let resolved = crate::commands::ndjson::convert_parsed_orgs(parsed);
        (Some(prov), resolved.into_values().collect())
    };

    std::fs::create_dir_all(&args.output)
        .with_context(|| format!("creating output directory: {}", args.output.display()))?;

    let edges = build_succession_edges(&records);
    let (successor_closures, predecessor_closures) = compute_transitive_closures(&records, &edges);

    // 1. Export orgs.parquet (Active only)
    export_orgs(
        &args.output,
        &records,
        &successor_closures,
        &predecessor_closures,
        provenance.as_ref(),
    )?;

    // 2. Export orgs_all.parquet (All records)
    export_orgs_all(
        &args.output,
        &records,
        &successor_closures,
        &predecessor_closures,
        provenance.as_ref(),
    )?;

    // 3. Export roles.parquet (one per organisation per role holding)
    export_roles(&args.output, &records, provenance.as_ref())?;

    // 4. Export relationships.parquet
    export_relationships(&args.output, &records, provenance.as_ref())?;

    // 5. Export successions.parquet
    export_successions(&args.output, &records, provenance.as_ref())?;

    // 6. Ship the datapackage.json alongside the data so the schema and metadata
    //    are reproducible from a release alone, without the tool.
    let release_pkg =
        crate::datapackage::generate_release_datapackage(&args.output, provenance.as_ref(), None, None);
    let pkg_json = serde_json::to_string_pretty(&release_pkg)?;
    std::fs::write(args.output.join("datapackage.json"), pkg_json)
        .context("writing datapackage.json")?;

    // 7. Write updated _provenance.json to output directory
    if let Some(ref p) = provenance {
        if let Ok(prov_json) = serde_json::to_string_pretty(p) {
            let _ = std::fs::write(
                args.output.join(crate::provenance::PROVENANCE_FILENAME),
                prov_json,
            );
        }
    }

    warn_unexpected_files(&args.output);

    Ok(())
}

pub fn get_unexpected_files(output_dir: &Path) -> Vec<String> {
    let known_files: HashSet<&str> = [
        "orgs.parquet",
        "orgs_all.parquet",
        "roles.parquet",
        "relationships.parquet",
        "successions.parquet",
        "datapackage.json",
        crate::provenance::PROVENANCE_FILENAME,
        "provenance.json",
        "NOTES.md",
    ]
    .into_iter()
    .collect();

    let mut unexpected = Vec::new();
    if let Ok(entries) = std::fs::read_dir(output_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if ext == "parquet" || ext == "json" {
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            if !known_files.contains(name) {
                                unexpected.push(name.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    unexpected.sort();
    unexpected
}

pub fn warn_unexpected_files(output_dir: &Path) {
    let unexpected = get_unexpected_files(output_dir);
    if !unexpected.is_empty() {
        let n = unexpected.len();
        let file_word = if n == 1 { "file" } else { "files" };
        eprintln!(
            "* {} unexpected {} in {}, not part of the release:",
            n,
            file_word,
            output_dir.display()
        );
        for f in unexpected {
            eprintln!("    {}", f);
        }
    }
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

fn extract_dates(
    dates: &[crate::commands::ndjson::OdsDate],
) -> (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    let mut legal_start = None;
    let mut legal_end = None;
    let mut operational_start = None;
    let mut operational_end = None;

    for d in dates {
        if d.date_type.eq_ignore_ascii_case("Legal") {
            legal_start = d.start.clone();
            legal_end = d.end.clone();
        } else if d.date_type.eq_ignore_ascii_case("Operational") {
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
        Field::new("name", DataType::Utf8, false),
        Field::new("record_class", DataType::Utf8, false),
        Field::new(
            "role_codes",
            DataType::List(Arc::new(Field::new("item", DataType::Utf8, true))),
            false,
        ),
        Field::new(
            "role_names",
            DataType::List(Arc::new(Field::new("item", DataType::Utf8, true))),
            false,
        ),
        Field::new("primary_role_code", DataType::Utf8, false),
        Field::new("address", DataType::Utf8, true),
        Field::new("town", DataType::Utf8, true),
        Field::new("county", DataType::Utf8, true),
        Field::new("postcode", DataType::Utf8, true),
        Field::new("country", DataType::Utf8, true),
        Field::new("uprn", DataType::Utf8, true),
        Field::new("telephone", DataType::Utf8, true),
        Field::new("website", DataType::Utf8, true),
        Field::new(
            "predecessor_codes",
            DataType::List(Arc::new(Field::new("item", DataType::Utf8, true))),
            false,
        ),
        Field::new(
            "successor_codes",
            DataType::List(Arc::new(Field::new("item", DataType::Utf8, true))),
            false,
        ),
        Field::new("status", DataType::Utf8, false),
        Field::new("legal_start", DataType::Date32, true),
        Field::new("legal_end", DataType::Date32, true),
        Field::new("operational_start", DataType::Date32, true),
        Field::new("operational_end", DataType::Date32, true),
        Field::new("last_changed", DataType::Date32, true),
        Field::new("trud_release_date", DataType::Date32, false),
    ])
}

fn build_orgs_batch(
    schema: &Arc<Schema>,
    records: &[&OdsRecord],
    successor_closures: &HashMap<String, Vec<String>>,
    predecessor_closures: &HashMap<String, Vec<String>>,
    release_days: i32,
) -> Result<RecordBatch> {
    let mut ods_code = StringBuilder::new();
    let mut name = StringBuilder::new();
    let mut record_class = StringBuilder::new();
    let mut roles_list = ListBuilder::new(StringBuilder::new());
    let mut role_names_list = ListBuilder::new(StringBuilder::new());
    let mut primary_role = StringBuilder::new();
    let mut address = StringBuilder::new();
    let mut town = StringBuilder::new();
    let mut county = StringBuilder::new();
    let mut postcode = StringBuilder::new();
    let mut country = StringBuilder::new();
    let mut uprn = StringBuilder::new();
    let mut telephone = StringBuilder::new();
    let mut website = StringBuilder::new();
    let mut predecessor_codes_list = ListBuilder::new(StringBuilder::new());
    let mut successor_codes_list = ListBuilder::new(StringBuilder::new());
    let mut status = StringBuilder::new();
    let mut legal_start = Date32Builder::new();
    let mut legal_end = Date32Builder::new();
    let mut operational_start = Date32Builder::new();
    let mut operational_end = Date32Builder::new();
    let mut last_change_date = Date32Builder::new();
    let mut trud_release_date = Date32Builder::new();

    let empty_vec = Vec::new();

    for r in records {
        ods_code.append_value(&r.ods_code);
        name.append_value(&r.name);
        record_class.append_value(&r.record_class);

        let org_is_active = r.status.eq_ignore_ascii_case("active");
        let mut codes: Vec<&str> = r
            .roles
            .iter()
            .filter(|role| !org_is_active || role.status.eq_ignore_ascii_case("active"))
            .map(|role| role.id.as_str())
            .collect();
        codes.sort_unstable();
        codes.dedup();
        for code in &codes {
            roles_list.values().append_value(code);
            let r_name = crate::roles::role_names().role_name(code)?;
            role_names_list.values().append_value(r_name);
        }
        roles_list.append(true);
        role_names_list.append(true);

        let primary_role_id = r
            .roles
            .iter()
            .find(|role| role.primary_role)
            .map(|role| role.id.as_str())
            .unwrap_or("");
        primary_role.append_value(primary_role_id);

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
                "tel" => {
                    if tel_val.is_none() {
                        tel_val = Some(&c.value);
                    }
                }
                "http" => {
                    if http_val.is_none() {
                        http_val = Some(&c.value);
                    }
                }
                _ => {}
            }
        }
        append_opt(&mut telephone, tel_val.map(|s| s.as_str()));
        append_opt(&mut website, http_val.map(|s| s.as_str()));

        let preds = predecessor_closures.get(&r.ods_code).unwrap_or(&empty_vec);
        for p in preds {
            predecessor_codes_list.values().append_value(p);
        }
        predecessor_codes_list.append(true);

        let succs = successor_closures.get(&r.ods_code).unwrap_or(&empty_vec);
        for s in succs {
            successor_codes_list.values().append_value(s);
        }
        successor_codes_list.append(true);

        status.append_value(&r.status);

        let (l_start, l_end, o_start, o_end) = extract_dates(&r.dates);
        append_date(&mut legal_start, l_start.as_deref());
        append_date(&mut legal_end, l_end.as_deref());
        append_date(&mut operational_start, o_start.as_deref());
        append_date(&mut operational_end, o_end.as_deref());
        append_date(&mut last_change_date, r.last_change_date.as_deref());
        trud_release_date.append_value(release_days);
    }

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(ods_code.finish()) as ArrayRef,
            Arc::new(name.finish()) as ArrayRef,
            Arc::new(record_class.finish()) as ArrayRef,
            Arc::new(roles_list.finish()) as ArrayRef,
            Arc::new(role_names_list.finish()) as ArrayRef,
            Arc::new(primary_role.finish()) as ArrayRef,
            Arc::new(address.finish()) as ArrayRef,
            Arc::new(town.finish()) as ArrayRef,
            Arc::new(county.finish()) as ArrayRef,
            Arc::new(postcode.finish()) as ArrayRef,
            Arc::new(country.finish()) as ArrayRef,
            Arc::new(uprn.finish()) as ArrayRef,
            Arc::new(telephone.finish()) as ArrayRef,
            Arc::new(website.finish()) as ArrayRef,
            Arc::new(predecessor_codes_list.finish()) as ArrayRef,
            Arc::new(successor_codes_list.finish()) as ArrayRef,
            Arc::new(status.finish()) as ArrayRef,
            Arc::new(legal_start.finish()) as ArrayRef,
            Arc::new(legal_end.finish()) as ArrayRef,
            Arc::new(operational_start.finish()) as ArrayRef,
            Arc::new(operational_end.finish()) as ArrayRef,
            Arc::new(last_change_date.finish()) as ArrayRef,
            Arc::new(trud_release_date.finish()) as ArrayRef,
        ],
    )
    .context("building Arrow orgs batch")?;

    Ok(batch)
}

pub fn export_orgs(
    output_dir: &Path,
    records: &[OdsRecord],
    successor_closures: &HashMap<String, Vec<String>>,
    predecessor_closures: &HashMap<String, Vec<String>>,
    provenance: Option<&crate::provenance::OdsProvenance>,
) -> Result<()> {
    let mut active_records: Vec<&OdsRecord> =
        records.iter().filter(|r| r.status == "active").collect();
    active_records.sort_by_key(|r| &r.ods_code);

    let release_date_str = provenance
        .and_then(|p| p.trud_release_date.as_deref())
        .ok_or_else(|| anyhow::anyhow!("Missing trud_release_date in provenance"))?;
    let release_days = parse_date_to_days(release_date_str)
        .ok_or_else(|| anyhow::anyhow!("Invalid trud_release_date: {}", release_date_str))?;

    let schema = embed_metadata(&orgs_schema(), provenance);
    let output_file =
        File::create(output_dir.join("orgs.parquet")).context("creating orgs.parquet")?;
    let props = writer_properties(provenance);
    let mut writer = ArrowWriter::try_new(output_file, schema.clone(), Some(props))
        .context("creating orgs ArrowWriter")?;

    for chunk in active_records.chunks(BATCH_SIZE) {
        let batch = build_orgs_batch(
            &schema,
            chunk,
            successor_closures,
            predecessor_closures,
            release_days,
        )?;
        writer.write(&batch).context("writing orgs batch")?;
    }
    writer.close().context("finalising orgs writer")?;
    println!("Exported {} records to orgs.parquet.", active_records.len());
    Ok(())
}

pub fn export_orgs_all(
    output_dir: &Path,
    records: &[OdsRecord],
    successor_closures: &HashMap<String, Vec<String>>,
    predecessor_closures: &HashMap<String, Vec<String>>,
    provenance: Option<&crate::provenance::OdsProvenance>,
) -> Result<()> {
    let mut all_records: Vec<&OdsRecord> = records.iter().collect();
    all_records.sort_by_key(|r| &r.ods_code);

    let release_date_str = provenance
        .and_then(|p| p.trud_release_date.as_deref())
        .ok_or_else(|| anyhow::anyhow!("Missing trud_release_date in provenance"))?;
    let release_days = parse_date_to_days(release_date_str)
        .ok_or_else(|| anyhow::anyhow!("Invalid trud_release_date: {}", release_date_str))?;

    let schema = embed_metadata(&orgs_schema(), provenance);
    let output_file =
        File::create(output_dir.join("orgs_all.parquet")).context("creating orgs_all.parquet")?;
    let props = writer_properties(provenance);
    let mut writer = ArrowWriter::try_new(output_file, schema.clone(), Some(props))
        .context("creating orgs_all ArrowWriter")?;

    for chunk in all_records.chunks(BATCH_SIZE) {
        let batch = build_orgs_batch(
            &schema,
            chunk,
            successor_closures,
            predecessor_closures,
            release_days,
        )?;
        writer.write(&batch).context("writing orgs_all batch")?;
    }
    writer.close().context("finalising orgs_all writer")?;
    println!(
        "Exported {} records to orgs_all.parquet.",
        all_records.len()
    );
    Ok(())
}

// ==========================================
// roles.parquet — one per organisation per role holding.
// ==========================================

#[derive(Clone)]
struct RoleRow {
    ods_code: String,
    role_code: String,
    role_name: String,
    role_id: String,
    is_primary: bool,
    status: String,
    legal_start: Option<String>,
    legal_end: Option<String>,
    operational_start: Option<String>,
    operational_end: Option<String>,
}

pub fn roles_schema() -> Schema {
    Schema::new(vec![
        Field::new("ods_code", DataType::Utf8, false),
        Field::new("role_code", DataType::Utf8, false),
        Field::new("role_name", DataType::Utf8, false),
        Field::new("is_primary", DataType::Boolean, false),
        Field::new("role_status", DataType::Utf8, false),
        Field::new("legal_start", DataType::Date32, true),
        Field::new("legal_end", DataType::Date32, true),
        Field::new("operational_start", DataType::Date32, true),
        Field::new("operational_end", DataType::Date32, true),
        Field::new("role_id", DataType::Utf8, false),
        Field::new("trud_release_date", DataType::Date32, false),
    ])
}

fn build_roles_batch(
    schema: &Arc<Schema>,
    rows: &[RoleRow],
    release_days: i32,
) -> Result<RecordBatch> {
    let mut ods_code = StringBuilder::new();
    let mut role_code = StringBuilder::new();
    let mut role_name = StringBuilder::new();
    let mut is_primary = BooleanBuilder::new();
    let mut status = StringBuilder::new();
    let mut legal_start = Date32Builder::new();
    let mut legal_end = Date32Builder::new();
    let mut operational_start = Date32Builder::new();
    let mut operational_end = Date32Builder::new();
    let mut role_id = StringBuilder::new();
    let mut trud_release_date = Date32Builder::new();

    for r in rows {
        ods_code.append_value(&r.ods_code);
        role_code.append_value(&r.role_code);
        role_name.append_value(&r.role_name);
        is_primary.append_value(r.is_primary);
        status.append_value(&r.status);
        append_date(&mut legal_start, r.legal_start.as_deref());
        append_date(&mut legal_end, r.legal_end.as_deref());
        append_date(&mut operational_start, r.operational_start.as_deref());
        append_date(&mut operational_end, r.operational_end.as_deref());
        role_id.append_value(&r.role_id);
        trud_release_date.append_value(release_days);
    }

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(ods_code.finish()) as ArrayRef,
            Arc::new(role_code.finish()) as ArrayRef,
            Arc::new(role_name.finish()) as ArrayRef,
            Arc::new(is_primary.finish()) as ArrayRef,
            Arc::new(status.finish()) as ArrayRef,
            Arc::new(legal_start.finish()) as ArrayRef,
            Arc::new(legal_end.finish()) as ArrayRef,
            Arc::new(operational_start.finish()) as ArrayRef,
            Arc::new(operational_end.finish()) as ArrayRef,
            Arc::new(role_id.finish()) as ArrayRef,
            Arc::new(trud_release_date.finish()) as ArrayRef,
        ],
    )
    .context("building Arrow roles batch")?;

    Ok(batch)
}

pub fn export_roles(
    output_dir: &Path,
    records: &[OdsRecord],
    provenance: Option<&crate::provenance::OdsProvenance>,
) -> Result<()> {
    let mut rows = Vec::new();
    for r in records {
        for role_record in &r.roles {
            let (l_start, l_end, o_start, o_end) = extract_dates(&role_record.dates);
            let r_name = crate::roles::role_names().role_name(&role_record.id)?;

            rows.push(RoleRow {
                ods_code: r.ods_code.clone(),
                role_code: role_record.id.clone(),
                role_name: r_name.to_string(),
                role_id: role_record.unique_role_id.clone(),
                is_primary: role_record.primary_role,
                status: role_record.status.clone(),
                legal_start: l_start,
                legal_end: l_end,
                operational_start: o_start,
                operational_end: o_end,
            });
        }
    }

    // Sort: ods_code ASC, role_code ASC, role_id ASC. role_id breaks ties
    // deterministically where one organisation holds the same code twice.
    rows.sort_by(|a, b| {
        a.ods_code
            .cmp(&b.ods_code)
            .then_with(|| a.role_code.cmp(&b.role_code))
            .then_with(|| a.role_id.cmp(&b.role_id))
    });

    let release_date_str = provenance
        .and_then(|p| p.trud_release_date.as_deref())
        .ok_or_else(|| anyhow::anyhow!("Missing trud_release_date in provenance"))?;
    let release_days = parse_date_to_days(release_date_str)
        .ok_or_else(|| anyhow::anyhow!("Invalid trud_release_date: {}", release_date_str))?;

    let schema = embed_metadata(&roles_schema(), provenance);
    let output_file =
        File::create(output_dir.join("roles.parquet")).context("creating roles.parquet")?;
    let props = writer_properties(provenance);
    let mut writer = ArrowWriter::try_new(output_file, schema.clone(), Some(props))
        .context("creating roles ArrowWriter")?;

    for chunk in rows.chunks(BATCH_SIZE) {
        let batch = build_roles_batch(&schema, chunk, release_days)?;
        writer.write(&batch).context("writing roles batch")?;
    }
    writer.close().context("finalising roles writer")?;
    println!("Exported {} records to roles.parquet.", rows.len());
    Ok(())
}

// ==========================================
// relationships.parquet
// ==========================================

#[derive(Clone)]
struct RelationshipRow {
    rel_id: String,
    source_code: String,
    target_code: String,
    rel_code: String,
    rel_name: String,
    rel_status: String,
    legal_start: Option<String>,
    legal_end: Option<String>,
    operational_start: Option<String>,
    operational_end: Option<String>,
}

pub fn relationships_schema() -> Schema {
    Schema::new(vec![
        Field::new("source_code", DataType::Utf8, false),
        Field::new("target_code", DataType::Utf8, false),
        Field::new("rel_code", DataType::Utf8, false),
        Field::new("rel_name", DataType::Utf8, false),
        Field::new("rel_status", DataType::Utf8, false),
        Field::new("legal_start", DataType::Date32, true),
        Field::new("legal_end", DataType::Date32, true),
        Field::new("operational_start", DataType::Date32, true),
        Field::new("operational_end", DataType::Date32, true),
        Field::new("rel_id", DataType::Utf8, false),
        Field::new("trud_release_date", DataType::Date32, false),
    ])
}

fn build_relationships_batch(
    schema: &Arc<Schema>,
    rows: &[RelationshipRow],
    release_days: i32,
) -> Result<RecordBatch> {
    let mut source_code = StringBuilder::new();
    let mut target_code = StringBuilder::new();
    let mut rel_code = StringBuilder::new();
    let mut rel_name = StringBuilder::new();
    let mut rel_status = StringBuilder::new();
    let mut legal_start = Date32Builder::new();
    let mut legal_end = Date32Builder::new();
    let mut operational_start = Date32Builder::new();
    let mut operational_end = Date32Builder::new();
    let mut rel_id = StringBuilder::new();
    let mut trud_release_date = Date32Builder::new();

    for r in rows {
        source_code.append_value(&r.source_code);
        target_code.append_value(&r.target_code);
        rel_code.append_value(&r.rel_code);
        rel_name.append_value(&r.rel_name);
        rel_status.append_value(&r.rel_status);
        append_date(&mut legal_start, r.legal_start.as_deref());
        append_date(&mut legal_end, r.legal_end.as_deref());
        append_date(&mut operational_start, r.operational_start.as_deref());
        append_date(&mut operational_end, r.operational_end.as_deref());
        rel_id.append_value(&r.rel_id);
        trud_release_date.append_value(release_days);
    }

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(source_code.finish()) as ArrayRef,
            Arc::new(target_code.finish()) as ArrayRef,
            Arc::new(rel_code.finish()) as ArrayRef,
            Arc::new(rel_name.finish()) as ArrayRef,
            Arc::new(rel_status.finish()) as ArrayRef,
            Arc::new(legal_start.finish()) as ArrayRef,
            Arc::new(legal_end.finish()) as ArrayRef,
            Arc::new(operational_start.finish()) as ArrayRef,
            Arc::new(operational_end.finish()) as ArrayRef,
            Arc::new(rel_id.finish()) as ArrayRef,
            Arc::new(trud_release_date.finish()) as ArrayRef,
        ],
    )
    .context("building Arrow relationships batch")?;

    Ok(batch)
}

pub fn export_relationships(
    output_dir: &Path,
    records: &[OdsRecord],
    provenance: Option<&crate::provenance::OdsProvenance>,
) -> Result<()> {
    let mut rows = Vec::new();
    for r in records {
        for rel in &r.relationships {
            let (l_start, l_end, o_start, o_end) = extract_dates(&rel.dates);

            rows.push(RelationshipRow {
                rel_id: rel.unique_rel_id.clone(),
                source_code: r.ods_code.clone(),
                target_code: rel.target.ods_code.clone(),
                rel_code: rel.id.clone(),
                rel_name: rel.display_name.clone().unwrap_or_else(|| rel.id.clone()),
                rel_status: rel.status.clone(),
                legal_start: l_start,
                legal_end: l_end,
                operational_start: o_start,
                operational_end: o_end,
            });
        }
    }

    // Sort: source_code ASC, target_code ASC, rel_code ASC, rel_id ASC
    rows.sort_by(|a, b| {
        a.source_code
            .cmp(&b.source_code)
            .then_with(|| a.target_code.cmp(&b.target_code))
            .then_with(|| a.rel_code.cmp(&b.rel_code))
            .then_with(|| a.rel_id.cmp(&b.rel_id))
    });

    let release_date_str = provenance
        .and_then(|p| p.trud_release_date.as_deref())
        .ok_or_else(|| anyhow::anyhow!("Missing trud_release_date in provenance"))?;
    let release_days = parse_date_to_days(release_date_str)
        .ok_or_else(|| anyhow::anyhow!("Invalid trud_release_date: {}", release_date_str))?;

    let schema = embed_metadata(&relationships_schema(), provenance);
    let output_file = File::create(output_dir.join("relationships.parquet"))
        .context("creating relationships.parquet")?;
    let props = writer_properties(provenance);
    let mut writer = ArrowWriter::try_new(output_file, schema.clone(), Some(props))
        .context("creating relationships ArrowWriter")?;

    for chunk in rows.chunks(BATCH_SIZE) {
        let batch = build_relationships_batch(&schema, chunk, release_days)?;
        writer
            .write(&batch)
            .context("writing relationships batch")?;
    }
    writer.close().context("finalising relationships writer")?;
    println!("Exported {} records to relationships.parquet.", rows.len());
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuccessionEdge {
    pub succession_id: String,
    pub predecessor_code: String,
    pub successor_code: String,
    pub legal_start: Option<String>,
}

pub fn build_succession_edges(records: &[OdsRecord]) -> Vec<SuccessionEdge> {
    let mut map: HashMap<String, SuccessionEdge> = HashMap::new();

    for r in records {
        for succ in &r.successors {
            let succ_type = succ.succ_type.to_lowercase();
            let legal_start = succ
                .dates
                .iter()
                .find(|d| d.date_type.eq_ignore_ascii_case("legal"))
                .and_then(|d| d.start.clone());

            let (pred, succ_code) = if succ_type.contains("predecessor") {
                (succ.target.ods_code.clone(), r.ods_code.clone())
            } else {
                (r.ods_code.clone(), succ.target.ods_code.clone())
            };

            let edge = SuccessionEdge {
                succession_id: succ.unique_succ_id.clone(),
                predecessor_code: pred,
                successor_code: succ_code,
                legal_start,
            };

            map.entry(succ.unique_succ_id.clone()).or_insert(edge);
        }
    }

    let mut edges: Vec<SuccessionEdge> = map.into_values().collect();
    edges.sort_by(|a, b| {
        a.predecessor_code
            .cmp(&b.predecessor_code)
            .then_with(|| a.successor_code.cmp(&b.successor_code))
            .then_with(|| a.succession_id.cmp(&b.succession_id))
    });

    edges
}

pub fn compute_transitive_closures(
    records: &[OdsRecord],
    edges: &[SuccessionEdge],
) -> (HashMap<String, Vec<String>>, HashMap<String, Vec<String>>) {
    let mut fwd_adj: HashMap<String, HashSet<String>> = HashMap::new();
    let mut rev_adj: HashMap<String, HashSet<String>> = HashMap::new();

    for edge in edges {
        fwd_adj
            .entry(edge.predecessor_code.clone())
            .or_default()
            .insert(edge.successor_code.clone());
        rev_adj
            .entry(edge.successor_code.clone())
            .or_default()
            .insert(edge.predecessor_code.clone());
    }

    let mut successor_closures: HashMap<String, Vec<String>> = HashMap::new();
    let mut predecessor_closures: HashMap<String, Vec<String>> = HashMap::new();

    for r in records {
        // Forward closure (successor_codes)
        let mut visited = HashSet::new();
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(r.ods_code.clone());
        visited.insert(r.ods_code.clone());

        while let Some(curr) = queue.pop_front() {
            if let Some(succs) = fwd_adj.get(&curr) {
                for s in succs {
                    if visited.insert(s.clone()) {
                        queue.push_back(s.clone());
                    }
                }
            }
        }
        visited.remove(&r.ods_code);
        let mut succs: Vec<String> = visited.into_iter().collect();
        succs.sort();
        successor_closures.insert(r.ods_code.clone(), succs);

        // Reverse closure (predecessor_codes)
        let mut rev_visited = HashSet::new();
        let mut rev_queue = std::collections::VecDeque::new();
        rev_queue.push_back(r.ods_code.clone());
        rev_visited.insert(r.ods_code.clone());

        while let Some(curr) = rev_queue.pop_front() {
            if let Some(preds) = rev_adj.get(&curr) {
                for p in preds {
                    if rev_visited.insert(p.clone()) {
                        rev_queue.push_back(p.clone());
                    }
                }
            }
        }
        rev_visited.remove(&r.ods_code);
        let mut preds: Vec<String> = rev_visited.into_iter().collect();
        preds.sort();
        predecessor_closures.insert(r.ods_code.clone(), preds);
    }

    (successor_closures, predecessor_closures)
}

pub fn successions_schema() -> Schema {
    Schema::new(vec![
        Field::new("predecessor_code", DataType::Utf8, false),
        Field::new("successor_code", DataType::Utf8, false),
        Field::new("legal_start", DataType::Date32, true),
        Field::new("succession_id", DataType::Utf8, false),
        Field::new("trud_release_date", DataType::Date32, false),
    ])
}

fn build_successions_batch(
    schema: &Arc<Schema>,
    edges: &[SuccessionEdge],
    release_days: i32,
) -> Result<RecordBatch> {
    let mut predecessor_code = StringBuilder::new();
    let mut successor_code = StringBuilder::new();
    let mut legal_start = Date32Builder::new();
    let mut succession_id = StringBuilder::new();
    let mut trud_release_date = Date32Builder::new();

    for edge in edges {
        predecessor_code.append_value(&edge.predecessor_code);
        successor_code.append_value(&edge.successor_code);
        append_date(&mut legal_start, edge.legal_start.as_deref());
        succession_id.append_value(&edge.succession_id);
        trud_release_date.append_value(release_days);
    }

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(predecessor_code.finish()) as ArrayRef,
            Arc::new(successor_code.finish()) as ArrayRef,
            Arc::new(legal_start.finish()) as ArrayRef,
            Arc::new(succession_id.finish()) as ArrayRef,
            Arc::new(trud_release_date.finish()) as ArrayRef,
        ],
    )
    .context("building Arrow successions batch")?;

    Ok(batch)
}

pub fn export_successions(
    output_dir: &Path,
    records: &[OdsRecord],
    provenance: Option<&crate::provenance::OdsProvenance>,
) -> Result<()> {
    let edges = build_succession_edges(records);
    let release_date_str = provenance
        .and_then(|p| p.trud_release_date.as_deref())
        .ok_or_else(|| anyhow::anyhow!("Missing trud_release_date in provenance"))?;
    let release_days = parse_date_to_days(release_date_str)
        .ok_or_else(|| anyhow::anyhow!("Invalid trud_release_date: {}", release_date_str))?;

    let schema = embed_metadata(&successions_schema(), provenance);
    let output_file = File::create(output_dir.join("successions.parquet"))
        .context("creating successions.parquet")?;
    let props = writer_properties(provenance);
    let mut writer = ArrowWriter::try_new(output_file, schema.clone(), Some(props))
        .context("creating successions ArrowWriter")?;

    for chunk in edges.chunks(BATCH_SIZE) {
        let batch = build_successions_batch(&schema, chunk, release_days)?;
        writer.write(&batch).context("writing successions batch")?;
    }
    writer.close().context("finalising successions writer")?;
    println!("Exported {} records to successions.parquet.", edges.len());
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
            status: "active".to_string(),
            role: "gp practice".to_string(),
            parent_organisation: None,
            region_code: None,
            root: None,
            assigning_authority_name: None,
            record_class: "site".to_string(),
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
            successors: vec![crate::commands::ndjson::OdsSuccessor {
                unique_succ_id: "777".to_string(),
                succ_type: "Predecessor".to_string(),
                dates: vec![crate::commands::ndjson::OdsDate {
                    date_type: "Legal".to_string(),
                    start: Some("2006-10-01".to_string()),
                    end: None,
                }],
                target: crate::commands::ndjson::OdsRelationshipTarget {
                    ods_code: "5FD51".to_string(),
                    name: Some("NHS Predecessor Org".to_string()),
                    root: None,
                    assigning_authority_name: None,
                    primary_role_id: None,
                    primary_role_display_name: None,
                    primary_role_unique_role_id: None,
                },
            }],
            ..Default::default()
        };

        let schema = Arc::new(orgs_schema());
        let release_days = parse_date_to_days("2026-07-31").unwrap();
        let batch = build_orgs_batch(
            &schema,
            &[&record],
            &HashMap::new(),
            &HashMap::new(),
            release_days,
        )
        .unwrap();

        // 1. Verify schema has "address" and does not have "address_line_1/2/3"
        assert!(schema.column_with_name("address").is_some());
        assert!(schema.column_with_name("address_line_1").is_none());

        // 2. Verify record_class field
        assert!(schema.column_with_name("record_class").is_some());

        // 3. Verify hierarchy fields are dropped
        assert!(schema.column_with_name("commissioner_name").is_none());
        assert!(schema.column_with_name("commissioner_code").is_none());
        assert!(schema.column_with_name("parent_name").is_none());
        assert!(schema.column_with_name("parent_code").is_none());
        assert!(schema.column_with_name("pcn_name").is_none());
        assert!(schema.column_with_name("pcn_code").is_none());
        assert!(schema.column_with_name("trust_name").is_none());
        assert!(schema.column_with_name("trust_code").is_none());
        assert!(schema.column_with_name("icb_name").is_none());
        assert!(schema.column_with_name("icb_code").is_none());
        assert!(schema.column_with_name("region_name").is_none());
        assert!(schema.column_with_name("region_code").is_none());

        // 4. Verify postcode renamed
        assert!(schema.column_with_name("postcode").is_some());

        // 5. Verify successor_codes and predecessor_codes present
        assert!(schema.column_with_name("successor_codes").is_some());
        assert!(schema.column_with_name("predecessor_codes").is_some());

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

        // 7. Verify date columns exist
        assert!(schema.column_with_name("legal_start").is_some());
        assert!(schema.column_with_name("legal_end").is_some());
        assert!(schema.column_with_name("operational_start").is_some());
        assert!(schema.column_with_name("operational_end").is_some());
        assert!(schema.column_with_name("last_changed").is_some());
        assert!(schema.column_with_name("trud_release_date").is_some());
    }

    #[test]
    fn test_successions_and_transitive_closures() {
        // Chain: 0AF -> 0CE -> 0CY -> YDDTR
        // 0AN -> 0CE
        // 0AJ -> 0CY
        let records = vec![
            OdsRecord {
                ods_code: "0AF".to_string(),
                status: "inactive".to_string(),
                successors: vec![crate::commands::ndjson::OdsSuccessor {
                    unique_succ_id: "101".to_string(),
                    succ_type: "Successor".to_string(),
                    dates: vec![crate::commands::ndjson::OdsDate {
                        date_type: "Legal".to_string(),
                        start: Some("2002-04-01".to_string()),
                        end: None,
                    }],
                    target: crate::commands::ndjson::OdsRelationshipTarget {
                        ods_code: "0CE".to_string(),
                        ..Default::default()
                    },
                }],
                ..Default::default()
            },
            OdsRecord {
                ods_code: "0AN".to_string(),
                status: "inactive".to_string(),
                successors: vec![crate::commands::ndjson::OdsSuccessor {
                    unique_succ_id: "102".to_string(),
                    succ_type: "Successor".to_string(),
                    dates: vec![],
                    target: crate::commands::ndjson::OdsRelationshipTarget {
                        ods_code: "0CE".to_string(),
                        ..Default::default()
                    },
                }],
                ..Default::default()
            },
            OdsRecord {
                ods_code: "0CE".to_string(),
                status: "inactive".to_string(),
                successors: vec![
                    // Stated from both ends! Same unique_succ_id "101"
                    crate::commands::ndjson::OdsSuccessor {
                        unique_succ_id: "101".to_string(),
                        succ_type: "Predecessor".to_string(),
                        dates: vec![crate::commands::ndjson::OdsDate {
                            date_type: "Legal".to_string(),
                            start: Some("2002-04-01".to_string()),
                            end: None,
                        }],
                        target: crate::commands::ndjson::OdsRelationshipTarget {
                            ods_code: "0AF".to_string(),
                            ..Default::default()
                        },
                    },
                    crate::commands::ndjson::OdsSuccessor {
                        unique_succ_id: "103".to_string(),
                        succ_type: "Successor".to_string(),
                        dates: vec![],
                        target: crate::commands::ndjson::OdsRelationshipTarget {
                            ods_code: "0CY".to_string(),
                            ..Default::default()
                        },
                    },
                ],
                ..Default::default()
            },
            OdsRecord {
                ods_code: "0AJ".to_string(),
                status: "inactive".to_string(),
                successors: vec![crate::commands::ndjson::OdsSuccessor {
                    unique_succ_id: "104".to_string(),
                    succ_type: "Successor".to_string(),
                    dates: vec![],
                    target: crate::commands::ndjson::OdsRelationshipTarget {
                        ods_code: "0CY".to_string(),
                        ..Default::default()
                    },
                }],
                ..Default::default()
            },
            OdsRecord {
                ods_code: "0CY".to_string(),
                status: "inactive".to_string(),
                successors: vec![crate::commands::ndjson::OdsSuccessor {
                    unique_succ_id: "105".to_string(),
                    succ_type: "Successor".to_string(),
                    dates: vec![],
                    target: crate::commands::ndjson::OdsRelationshipTarget {
                        ods_code: "YDDTR".to_string(),
                        ..Default::default()
                    },
                }],
                ..Default::default()
            },
            OdsRecord {
                ods_code: "YDDTR".to_string(),
                status: "active".to_string(),
                ..Default::default()
            },
        ];

        let edges = build_succession_edges(&records);
        // Deduplicated: 5 unique edges (unique_succ_id 101, 102, 103, 104, 105)
        assert_eq!(edges.len(), 5);

        let (succ_closures, pred_closures) = compute_transitive_closures(&records, &edges);

        assert_eq!(
            succ_closures.get("0AF").unwrap(),
            &vec!["0CE", "0CY", "YDDTR"]
        );
        assert_eq!(pred_closures.get("0AF").unwrap(), &Vec::<String>::new());

        assert_eq!(succ_closures.get("0CE").unwrap(), &vec!["0CY", "YDDTR"]);
        assert_eq!(pred_closures.get("0CE").unwrap(), &vec!["0AF", "0AN"]);

        assert_eq!(succ_closures.get("0CY").unwrap(), &vec!["YDDTR"]);
        assert_eq!(
            pred_closures.get("0CY").unwrap(),
            &vec!["0AF", "0AJ", "0AN", "0CE"]
        );

        assert_eq!(succ_closures.get("YDDTR").unwrap(), &Vec::<String>::new());
        assert_eq!(
            pred_closures.get("YDDTR").unwrap(),
            &vec!["0AF", "0AJ", "0AN", "0CE", "0CY"]
        );
    }

    #[test]
    fn test_relationships_schema_and_export() {
        let schema = relationships_schema();
        assert!(schema.column_with_name("rel_id").is_some());
        assert!(schema.column_with_name("source_code").is_some());
        assert!(schema.column_with_name("target_code").is_some());
        assert!(schema.column_with_name("rel_code").is_some());
        assert!(schema.column_with_name("rel_name").is_some());
        assert!(schema.column_with_name("rel_status").is_some());
        assert!(schema.column_with_name("legal_start").is_some());
        assert!(schema.column_with_name("trud_release_date").is_some());

        // Assert dropped columns do not exist
        assert!(schema.column_with_name("rel_type_code").is_none());
        assert!(schema.column_with_name("rel_type_name").is_none());
        assert!(schema.column_with_name("source").is_none());
        assert!(schema.column_with_name("target").is_none());
        assert!(schema.column_with_name("rel_type").is_none());
        assert!(schema.column_with_name("status").is_none());
        assert!(schema.column_with_name("publication_date").is_none());

        let record = OdsRecord {
            ods_code: "0AF".to_string(),
            name: "Bury HA".to_string(),
            relationships: vec![crate::commands::ndjson::OdsRelationship {
                id: "RE4".to_string(),
                display_name: Some("IS COMMISSIONED BY".to_string()),
                unique_rel_id: "999".to_string(),
                status: "active".to_string(),
                dates: vec![],
                target: crate::commands::ndjson::OdsRelationshipTarget {
                    ods_code: "QE1".to_string(),
                    ..Default::default()
                },
            }],
            ..Default::default()
        };

        let mut prov = crate::provenance::OdsProvenance::default();
        prov.trud_release_date = Some("2026-07-31".to_string());

        let temp_dir = tempfile::tempdir().unwrap();
        export_relationships(temp_dir.path(), &[record], Some(&prov)).unwrap();
        assert!(temp_dir.path().join("relationships.parquet").exists());
    }

    #[test]
    fn test_task_6_column_renames() {
        let orgs_s = orgs_schema();
        assert!(orgs_s.column_with_name("record_class").is_some());
        assert!(orgs_s.column_with_name("primary_role_code").is_some());
        assert!(orgs_s.column_with_name("role_codes").is_some());
        assert!(orgs_s.column_with_name("role_names").is_some());
        assert!(orgs_s.column_with_name("last_changed").is_some());
        assert!(orgs_s.column_with_name("commissioner_name").is_none());
        assert!(orgs_s.column_with_name("parent_name").is_none());
        assert!(orgs_s.column_with_name("pcn_name").is_none());
        assert!(orgs_s.column_with_name("trust_name").is_none());
        assert!(orgs_s.column_with_name("icb_name").is_none());
        assert!(orgs_s.column_with_name("region_name").is_none());

        assert!(orgs_s.column_with_name("primary_role").is_none());
        assert!(orgs_s.column_with_name("roles").is_none());
        assert!(orgs_s.column_with_name("last_change_date").is_none());
        assert!(orgs_s.column_with_name("commissioner").is_none());
        assert!(orgs_s.column_with_name("parent").is_none());
        assert!(orgs_s.column_with_name("pcn").is_none());
        assert!(orgs_s.column_with_name("trust").is_none());
        assert!(orgs_s.column_with_name("icb").is_none());
        assert!(orgs_s.column_with_name("region").is_none());

        let roles_s = roles_schema();
        assert!(roles_s.column_with_name("ods_code").is_some());
        assert!(roles_s.column_with_name("role_code").is_some());
        assert!(roles_s.column_with_name("role_name").is_some());
        assert!(roles_s.column_with_name("role_id").is_some());
        assert!(roles_s.column_with_name("is_primary").is_some());
        assert!(roles_s.column_with_name("role_status").is_some());
        assert!(roles_s.column_with_name("status").is_none());
        assert!(roles_s.column_with_name("can_be_primary").is_none());
    }

    #[test]
    fn test_task_7_trud_release_date_on_all_tables() {
        assert!(orgs_schema()
            .column_with_name("trud_release_date")
            .is_some());
        assert!(roles_schema()
            .column_with_name("trud_release_date")
            .is_some());
        assert!(relationships_schema()
            .column_with_name("trud_release_date")
            .is_some());
        assert!(successions_schema()
            .column_with_name("trud_release_date")
            .is_some());

        assert!(orgs_schema().column_with_name("publication_date").is_none());
        assert!(roles_schema()
            .column_with_name("publication_date")
            .is_none());
        assert!(relationships_schema()
            .column_with_name("publication_date")
            .is_none());
        assert!(successions_schema()
            .column_with_name("publication_date")
            .is_none());
    }
}

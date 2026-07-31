use anyhow::{Context, Result};
use clap::Parser;
use quick_xml::events::{Event, BytesStart};
use quick_xml::reader::Reader;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

#[derive(Parser, Debug)]
pub struct Args {
    /// Input directory or XML file containing HSCOrgRefData
    #[arg(long, short)]
    pub input: PathBuf,

    /// Output NDJSON file path
    #[arg(long, short, default_value = "./ods.ndjson")]
    pub output: PathBuf,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct OdsDate {
    #[serde(rename = "type")]
    pub date_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Location {
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub address_lines: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub town: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub county: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub postcode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uprn: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct OdsRole {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub unique_role_id: String,
    pub primary_role: bool,
    pub status: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub dates: Vec<OdsDate>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct OdsRelationshipTarget {
    pub ods_code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assigning_authority_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_role_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_role_display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_role_unique_role_id: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct OdsRelationship {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub unique_rel_id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub dates: Vec<OdsDate>,
    pub target: OdsRelationshipTarget,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct OdsSuccessor {
    pub unique_succ_id: String,
    #[serde(rename = "type")]
    pub succ_type: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub dates: Vec<OdsDate>,
    pub target: OdsRelationshipTarget,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct OdsContact {
    #[serde(rename = "type")]
    pub contact_type: String,
    pub value: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ParentOrganisation {
    pub ods_code: String,
    pub name: String,
}

/// Raw organisation data as parsed directly from the TRUD XML.
/// Fields that require cross-record graph resolution (pcn/trust/icb hierarchy,
/// flattened dates) are absent — they live only in `OdsRecord` post-resolution.
#[derive(Debug, Clone)]
pub struct ParsedOrg {
    pub ods_code: String,
    pub name: String,
    pub status: String,
    pub role: String,
    pub parent_organisation: Option<ParentOrganisation>,
    pub region_code: Option<String>,
    pub root: Option<String>,
    pub assigning_authority_name: Option<String>,
    pub org_record_class: Option<String>,
    pub last_change_date: Option<String>,
    pub dates: Vec<OdsDate>,
    pub geo_loc: Option<Location>,
    pub contacts: Vec<OdsContact>,
    pub roles: Vec<OdsRole>,
    pub relationships: Vec<OdsRelationship>,
    pub successors: Vec<OdsSuccessor>,
}

/// Fully-resolved organisation record: all `ParsedOrg` fields plus the
/// denormalized hierarchy lookups and flattened dates produced by
/// `resolve_hierarchies`. This is the canonical NDJSON / Parquet shape.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct OdsRecord {
    pub ods_code: String,
    pub name: String,
    pub status: String,
    pub role: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_organisation: Option<ParentOrganisation>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub region_code: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub assigning_authority_name: Option<String>,

    pub record_class: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_change_date: Option<String>,

    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub dates: Vec<OdsDate>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub geo_loc: Option<Location>,

    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub contacts: Vec<OdsContact>,

    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub roles: Vec<OdsRole>,

    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub relationships: Vec<OdsRelationship>,

    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub successors: Vec<OdsSuccessor>,

    // --- resolved / denormalized fields (populated by resolve_hierarchies) ---
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commissioner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commissioner_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pcn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pcn_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trust: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trust_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icb: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icb_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_date: Option<String>,
}

pub fn find_xml_file(input_path: &Path) -> Result<PathBuf> {
    if input_path.is_file() {
        return Ok(input_path.to_path_buf());
    }
    for entry in walkdir::WalkDir::new(input_path) {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|ext| ext == "xml") {
            return Ok(path.to_path_buf());
        }
    }
    anyhow::bail!("No XML file found in {}", input_path.display())
}

pub fn run(args: Args) -> Result<()> {
    // 1. Fail early: check output directory and file
    if let Some(parent) = args.output.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating directory for output: {}", parent.display()))?;
        }
    }
    let output_file = File::create(&args.output)
        .with_context(|| format!("cannot create or write to output file: {}", args.output.display()))?;

    // 2. Find and parse XML input
    let xml_path = find_xml_file(&args.input)?;
    eprintln!("Found XML file: {}", xml_path.display());

    eprintln!("Compiling ODS database (single pass)...");
    let start_compile = std::time::Instant::now();
    let (provenance, concept_map, records) = parse_single_pass(&xml_path)?;
    eprintln!(
        "Parsing complete. Found {} concept mappings and {} organisations. Took {:?}",
        concept_map.len(),
        records.len(),
        start_compile.elapsed()
    );

    // Hierarchy and Date Resolution Pass
    eprintln!("Resolving parent hierarchies and dates...");
    let start_resolve = std::time::Instant::now();
    let resolved_records = resolve_hierarchies(records);
    eprintln!("Hierarchy resolution complete. Took {:?}", start_resolve.elapsed());

    // Write to NDJSON
    eprintln!("Writing compiled NDJSON to {}...", args.output.display());
    let start_write = std::time::Instant::now();
    let mut writer = BufWriter::new(output_file);

    // Line 1: Dataset & Build Provenance Header
    let prov_json = serde_json::to_string(&provenance)?;
    writer.write_all(prov_json.as_bytes())?;
    writer.write_all(b"\n")?;

    for record in resolved_records.values() {
        let serialized = serde_json::to_string(record)?;
        writer.write_all(serialized.as_bytes())?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    eprintln!("NDJSON compiled successfully. Took {:?}", start_write.elapsed());

    Ok(())
}

fn resolve_commissioner(code: &str, parsed: &HashMap<String, ParsedOrg>) -> Option<(String, String)> {
    let mut current = code.to_string();
    let mut visited = std::collections::HashSet::new();
    while visited.insert(current.clone()) {
        let Some(rec) = parsed.get(&current) else {
            return None;
        };
        // Check if this entity itself is an ICB
        let is_icb = rec.roles.iter().any(|r| r.id == crate::ods_codes::ROLE_ICB && r.status.eq_ignore_ascii_case("active"));
        if is_icb {
            return Some((rec.ods_code.clone(), rec.name.clone()));
        }
        // Follow REL_COMMISSIONED_BY
        let mut next = None;
        for rel in &rec.relationships {
            if rel.status.eq_ignore_ascii_case("active") && rel.id == crate::ods_codes::REL_COMMISSIONED_BY {
                next = Some(rel.target.ods_code.clone());
                break;
            }
        }
        // Fallback: If no direct commissioning relationship exists, check if this is a Sub ICB Location (RO319)
        // and follow its regional geography link (RE5) to the parent ICB.
        if next.is_none() {
            let is_sub_icb = rec.roles.iter().any(|r| r.id == "RO319" && r.status.eq_ignore_ascii_case("active"));
            if is_sub_icb {
                for rel in &rec.relationships {
                    if rel.status.eq_ignore_ascii_case("active") && rel.id == crate::ods_codes::REL_REGION {
                        next = Some(rel.target.ods_code.clone());
                        break;
                    }
                }
            }
        }
        if let Some(n) = next {
            current = n;
        } else {
            break;
        }
    }
    None
}

fn resolve_parent(code: &str, parsed: &HashMap<String, ParsedOrg>) -> Option<(String, String)> {
    let mut current = code.to_string();
    let mut visited = std::collections::HashSet::new();
    while visited.insert(current.clone()) {
        let Some(rec) = parsed.get(&current) else {
            return None;
        };
        // Check if this entity itself is an NHS Trust
        let is_trust = rec.roles.iter().any(|r| r.id == crate::ods_codes::ROLE_NHS_TRUST && r.status.eq_ignore_ascii_case("active"));
        if is_trust {
            return Some((rec.ods_code.clone(), rec.name.clone()));
        }
        // Follow REL_MANAGED_BY
        let mut next = None;
        for rel in &rec.relationships {
            if rel.status.eq_ignore_ascii_case("active") && rel.id == crate::ods_codes::REL_MANAGED_BY {
                next = Some(rel.target.ods_code.clone());
                break;
            }
        }
        if let Some(n) = next {
            current = n;
        } else if let Some(ref parent) = rec.parent_organisation {
            current = parent.ods_code.clone();
        } else {
            if current != code {
                return Some((rec.ods_code.clone(), rec.name.clone()));
            } else {
                return None;
            }
        }
    }
    None
}

fn resolve_pcn(code: &str, parsed: &HashMap<String, ParsedOrg>) -> Option<(String, String)> {
    let rec = parsed.get(code)?;
    if rec.roles.iter().any(|r| r.id == crate::ods_codes::ROLE_PCN && r.status.eq_ignore_ascii_case("active")) {
        return Some((rec.ods_code.clone(), rec.name.clone()));
    }
    for rel in &rec.relationships {
        if rel.status.eq_ignore_ascii_case("active") {
            if let Some(target_rec) = parsed.get(&rel.target.ods_code) {
                if target_rec.roles.iter().any(|r| r.id == crate::ods_codes::ROLE_PCN && r.status.eq_ignore_ascii_case("active")) {
                    return Some((target_rec.ods_code.clone(), target_rec.name.clone()));
                }
            }
        }
    }
    None
}

fn resolve_trust(code: &str, parsed: &HashMap<String, ParsedOrg>) -> Option<(String, String)> {
    let mut current = code.to_string();
    let mut visited = std::collections::HashSet::new();
    while visited.insert(current.clone()) {
        let rec = parsed.get(&current)?;
        if rec.roles.iter().any(|r| r.id == crate::ods_codes::ROLE_NHS_TRUST && r.status.eq_ignore_ascii_case("active")) {
            return Some((rec.ods_code.clone(), rec.name.clone()));
        }
        let mut next = None;
        for rel in &rec.relationships {
            if rel.status.eq_ignore_ascii_case("active") && rel.id == crate::ods_codes::REL_MANAGED_BY {
                next = Some(rel.target.ods_code.clone());
                break;
            }
        }
        if let Some(n) = next {
            current = n;
        } else if let Some(ref parent) = rec.parent_organisation {
            current = parent.ods_code.clone();
        } else {
            break;
        }
    }
    None
}

fn resolve_icb(code: &str, parsed: &HashMap<String, ParsedOrg>) -> Option<(String, String)> {
    let mut current = code.to_string();
    let mut visited = std::collections::HashSet::new();
    while visited.insert(current.clone()) {
        let rec = parsed.get(&current)?;
        let is_icb = rec.roles.iter().any(|r| (r.id == crate::ods_codes::ROLE_ICB || r.id == "RO98" || r.id == "RO319") && r.status.eq_ignore_ascii_case("active"));
        if is_icb {
            return Some((rec.ods_code.clone(), rec.name.clone()));
        }
        if let Some((comm_code, comm_name)) = resolve_commissioner(&current, parsed) {
            if comm_code != current {
                current = comm_code;
            } else {
                return Some((comm_code, comm_name));
            }
        } else if let Some(ref parent) = rec.parent_organisation {
            if parent.ods_code != current {
                current = parent.ods_code.clone();
            } else {
                break;
            }
        } else {
            break;
        }
    }
    None
}

fn resolve_region(code: &str, parsed: &HashMap<String, ParsedOrg>) -> Option<(String, String)> {
    let rec = parsed.get(code)?;
    if let Some(ref reg_code) = rec.region_code {
        let reg_name = parsed.get(reg_code).map(|r| r.name.clone()).unwrap_or_else(|| reg_code.clone());
        return Some((reg_code.clone(), reg_name));
    }
    for rel in &rec.relationships {
        if rel.status.eq_ignore_ascii_case("active") && rel.id == crate::ods_codes::REL_REGION {
            let target_name = parsed.get(&rel.target.ods_code).map(|r| r.name.clone()).unwrap_or_else(|| rel.target.ods_code.clone());
            return Some((rel.target.ods_code.clone(), target_name));
        }
    }
    None
}

/// Resolves cross-record hierarchy (commissioner / parent / pcn / trust / icb / region), flattens dates, and
/// resolves human-readable names for relationship targets.
pub fn resolve_hierarchies(parsed: HashMap<String, ParsedOrg>) -> HashMap<String, OdsRecord> {
    // Collected per-org resolved properties before consuming `parsed`.
    struct HierarchyProps {
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
        start_date: Option<String>,
        end_date: Option<String>,
    }

    let mut resolved_props: HashMap<String, HierarchyProps> = parsed
        .par_iter()
        .map(|(code, rec)| {
            let comm = resolve_commissioner(code, &parsed);
            let (commissioner_code, commissioner) = match comm {
                Some((c, n)) => (Some(c), Some(n)),
                None => (None, None),
            };

            let par = resolve_parent(code, &parsed);
            let (parent_code, parent) = match par {
                Some((c, n)) => (Some(c), Some(n)),
                None => (None, None),
            };

            let pcn_res = resolve_pcn(code, &parsed);
            let (pcn_code, pcn) = match pcn_res {
                Some((c, n)) => (Some(c), Some(n)),
                None => (None, None),
            };

            let trust_res = resolve_trust(code, &parsed);
            let (trust_code, trust) = match trust_res {
                Some((c, n)) => (Some(c), Some(n)),
                None => (None, None),
            };

            let icb_res = resolve_icb(code, &parsed);
            let (icb_code, icb) = match icb_res {
                Some((c, n)) => (Some(c), Some(n)),
                None => (None, None),
            };

            let reg_res = resolve_region(code, &parsed);
            let (region_code, region) = match reg_res {
                Some((c, n)) => (Some(c), Some(n)),
                None => (None, None),
            };

            // Resolve start/end dates (prefer Legal, fallback to Operational)
            let start_date = rec.dates.iter()
                .find(|d| d.date_type == "Legal")
                .or_else(|| rec.dates.iter().find(|d| d.date_type == "Operational"))
                .and_then(|d| d.start.clone());

            let end_date = rec.dates.iter()
                .find(|d| d.date_type == "Legal")
                .or_else(|| rec.dates.iter().find(|d| d.date_type == "Operational"))
                .and_then(|d| d.end.clone());

            (
                code.clone(),
                HierarchyProps {
                    commissioner,
                    commissioner_code,
                    parent,
                    parent_code,
                    pcn,
                    pcn_code,
                    trust,
                    trust_code,
                    icb,
                    icb_code,
                    region,
                    region_code,
                    start_date,
                    end_date,
                },
            )
        })
        .collect();

    // Build name map before consuming `parsed`.
    let name_map: HashMap<String, String> =
        parsed.iter().map(|(k, v)| (k.clone(), v.name.clone())).collect();

    let mut records: HashMap<String, OdsRecord> = HashMap::with_capacity(parsed.len());

    for (code, mut org) in parsed {
        let props = resolved_props
            .remove(&code)
            .expect("every parsed code must have resolved props");

        // Resolve human-readable names for linked records.
        if let Some(ref mut parent) = org.parent_organisation {
            if let Some(name) = name_map.get(&parent.ods_code) {
                parent.name = name.clone();
            }
        }
        for rel in &mut org.relationships {
            rel.target.name = name_map.get(&rel.target.ods_code).cloned();
        }
        for succ in &mut org.successors {
            succ.target.name = name_map.get(&succ.target.ods_code).cloned();
        }

        let record_class = match org.org_record_class.as_deref() {
            Some("RC1") => "org".to_string(),
            Some("RC2") => "site".to_string(),
            _ => "org".to_string(),
        };

        // Normalize roles and relationships inside org to lowercase
        for r in &mut org.roles {
            r.status = r.status.to_lowercase();
            if let Some(ref mut d) = r.display_name {
                *d = d.to_lowercase();
            }
        }
        for rel in &mut org.relationships {
            rel.status = rel.status.to_lowercase();
            if let Some(ref mut d) = rel.display_name {
                *d = d.to_lowercase();
            }
            if let Some(ref mut prd) = rel.target.primary_role_display_name {
                *prd = prd.to_lowercase();
            }
        }
        for succ in &mut org.successors {
            succ.succ_type = succ.succ_type.to_lowercase();
            if let Some(ref mut prd) = succ.target.primary_role_display_name {
                *prd = prd.to_lowercase();
            }
        }

        records.insert(code, OdsRecord {
            ods_code: org.ods_code,
            name: org.name,
            status: org.status.to_lowercase(),
            role: org.role.to_lowercase(),
            parent_organisation: org.parent_organisation,
            region_code: props.region_code.clone(),
            root: org.root,
            assigning_authority_name: org.assigning_authority_name,
            record_class,
            last_change_date: org.last_change_date,
            dates: org.dates,
            geo_loc: org.geo_loc,
            contacts: org.contacts,
            roles: org.roles,
            relationships: org.relationships,
            successors: org.successors,
            // Resolved hierarchy fields.
            commissioner: props.commissioner,
            commissioner_code: props.commissioner_code,
            parent: props.parent,
            parent_code: props.parent_code,
            pcn: props.pcn,
            pcn_code: props.pcn_code,
            trust: props.trust,
            trust_code: props.trust_code,
            icb: props.icb,
            icb_code: props.icb_code,
            region: props.region,
            start_date: props.start_date,
            end_date: props.end_date,
        });
    }

    records
}

fn parse_concept_attrs<B: std::io::BufRead>(e: &BytesStart, reader: &Reader<B>, concept_map: &mut HashMap<String, String>) -> Result<()> {
    let mut attr_id = None;
    let mut attr_code = None;
    let mut display_name = None;
    for attr in e.attributes() {
        let attr = attr?;
        let key = attr.key.as_ref();
        if key == b"id" {
            attr_id = Some(attr.decode_and_unescape_value(reader)?.into_owned());
        } else if key == b"code" {
            attr_code = Some(attr.decode_and_unescape_value(reader)?.into_owned());
        } else if key == b"displayName" {
            display_name = Some(attr.decode_and_unescape_value(reader)?.into_owned());
        }
    }
    let key = attr_id.or(attr_code);
    if let (Some(k), Some(v)) = (key, display_name) {
        concept_map.insert(k, v);
    }
    Ok(())
}

fn get_manifest_attr<B: std::io::BufRead>(e: &BytesStart, reader: &Reader<B>) -> Option<String> {
    for attr in e.attributes().flatten() {
        if attr.key.as_ref() == b"value" {
            return attr.decode_and_unescape_value(reader).ok().map(|s| s.into_owned());
        }
    }
    None
}

pub fn parse_single_pass(
    xml_path: &Path,
) -> Result<(crate::provenance::OdsProvenance, HashMap<String, String>, HashMap<String, ParsedOrg>)> {
    let file = File::open(xml_path)?;
    let buf_reader = BufReader::with_capacity(128 * 1024, file);
    let mut reader = Reader::from_reader(buf_reader);
    reader.trim_text(true);
 
    let mut concept_map = HashMap::new();
    let mut parsed: HashMap<String, ParsedOrg> = HashMap::new();
    let mut parser_state = ParserState::new();
    let mut buf = Vec::new();

    let mut pub_date = None;
    let mut pub_seq = None;
    let mut pub_type = None;
    let mut pub_source = None;
    let mut xml_version = None;
    let mut xml_creation = None;
 
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(ref e) => {
                let name = e.name();
                let name_ref = name.as_ref();
                if name_ref == b"concept" || name_ref == b"Concept" {
                    parse_concept_attrs(e, &reader, &mut concept_map)?;
                } else {
                    parser_state.handle_start_or_empty(name_ref, e, false, &reader, &concept_map)?;
                }
            }
            Event::Empty(ref e) => {
                let name = e.name();
                let name_ref = name.as_ref();
                if name_ref == b"concept" || name_ref == b"Concept" {
                    parse_concept_attrs(e, &reader, &mut concept_map)?;
                } else if name_ref == b"PublicationDate" {
                    pub_date = get_manifest_attr(e, &reader);
                } else if name_ref == b"PublicationSeqNum" {
                    pub_seq = get_manifest_attr(e, &reader);
                } else if name_ref == b"PublicationType" {
                    pub_type = get_manifest_attr(e, &reader);
                } else if name_ref == b"PublicationSource" {
                    pub_source = get_manifest_attr(e, &reader);
                } else if name_ref == b"Version" {
                    xml_version = get_manifest_attr(e, &reader);
                } else if name_ref == b"FileCreationDateTime" {
                    xml_creation = get_manifest_attr(e, &reader);
                } else {
                    parser_state.handle_start_or_empty(name_ref, e, true, &reader, &concept_map)?;
                }
            }
            Event::End(ref e) => {
                let name = e.name();
                parser_state.handle_end(name.as_ref(), &mut parsed)?;
            }
            Event::Text(ref e) => {
                if parser_state.current_text_target != TextTarget::None {
                    let text = e.unescape()?.into_owned();
                    parser_state.handle_text(text)?;
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    let provenance = crate::provenance::OdsProvenance::new(
        pub_date,
        pub_seq,
        pub_type,
        pub_source,
        xml_version,
        xml_creation,
        Some(xml_path),
    );

    Ok((provenance, concept_map, parsed))
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum TextTarget {
    None,
    OrgName,
    AddrLine,
    Town,
    County,
    PostCode,
    Country,
    Uprn,
    SuccType,
}

#[derive(Default)]
struct OrgState {
    code: Option<String>,
    name: Option<String>,
    status: Option<String>,
    root: Option<String>,
    assigning_authority_name: Option<String>,
    record_class: Option<String>,
    last_change_date: Option<String>,
    dates: Vec<OdsDate>,
    location: Option<Location>,
    contacts: Vec<OdsContact>,
    roles: Vec<OdsRole>,
    relationships: Vec<OdsRelationship>,
    successors: Vec<OdsSuccessor>,
    role: Option<OdsRole>,
    rel: Option<OdsRelationship>,
    succ: Option<OdsSuccessor>,
    date: Option<OdsDate>,
    target: Option<OdsRelationshipTarget>,
}

struct ParserState {
    in_organisation: bool,
    in_rel: bool,
    in_succ: bool,
    in_geoloc: bool,
    in_location: bool,
    current_text_target: TextTarget,
    org: OrgState,
}

impl ParserState {
    fn new() -> Self {
        Self {
            in_organisation: false,
            in_rel: false,
            in_succ: false,
            in_geoloc: false,
            in_location: false,
            current_text_target: TextTarget::None,
            org: OrgState::default(),
        }
    }

    fn reset_org(&mut self) {
        self.in_organisation = true;
        self.in_rel = false;
        self.in_succ = false;
        self.in_geoloc = false;
        self.in_location = false;
        self.current_text_target = TextTarget::None;
        self.org = OrgState::default();
    }

    fn handle_start_or_empty<B: std::io::BufRead>(
        &mut self,
        name_ref: &[u8],
        e: &BytesStart,
        is_empty: bool,
        reader: &Reader<B>,
        concept_map: &HashMap<String, String>,
    ) -> Result<()> {
        match name_ref {
            b"Organisation" => {
                self.reset_org();
                for attr in e.attributes() {
                    let attr = attr?;
                    if attr.key.as_ref() == b"orgRecordClass" {
                        self.org.record_class = Some(attr.decode_and_unescape_value(reader)?.into_owned());
                    }
                }
            }
            b"Name" => {
                if self.in_organisation && !self.in_rel && !self.in_succ && !self.in_geoloc {
                    self.current_text_target = TextTarget::OrgName;
                }
            }
            b"Date" => {
                self.org.date = Some(OdsDate {
                    date_type: String::new(),
                    start: None,
                    end: None,
                });
            }
            b"Type" => {
                if self.org.date.is_some() {
                    for attr in e.attributes() {
                        let attr = attr?;
                        if attr.key.as_ref() == b"value" {
                            if let Some(ref mut d) = self.org.date {
                                d.date_type = attr.decode_and_unescape_value(reader)?.into_owned();
                            }
                        }
                    }
                } else if self.org.succ.is_some() {
                    self.current_text_target = TextTarget::SuccType;
                }
            }
            b"Start" => {
                if let Some(ref mut d) = self.org.date {
                    for attr in e.attributes() {
                        let attr = attr?;
                        if attr.key.as_ref() == b"value" {
                            d.start = Some(attr.decode_and_unescape_value(reader)?.into_owned());
                        }
                    }
                }
            }
            b"End" => {
                if let Some(ref mut d) = self.org.date {
                    for attr in e.attributes() {
                        let attr = attr?;
                        if attr.key.as_ref() == b"value" {
                            d.end = Some(attr.decode_and_unescape_value(reader)?.into_owned());
                        }
                    }
                }
            }
            b"Status" => {
                let mut status_val = String::new();
                for attr in e.attributes() {
                    let attr = attr?;
                    if attr.key.as_ref() == b"value" {
                        status_val = attr.decode_and_unescape_value(reader)?.into_owned();
                    }
                }
                if let Some(ref mut r) = self.org.role {
                    r.status = status_val;
                } else if let Some(ref mut rel) = self.org.rel {
                    rel.status = status_val;
                } else if self.in_organisation {
                    self.org.status = Some(status_val);
                }
            }
            b"LastChangeDate" => {
                for attr in e.attributes() {
                    let attr = attr?;
                    if attr.key.as_ref() == b"value" {
                        self.org.last_change_date = Some(attr.decode_and_unescape_value(reader)?.into_owned());
                    }
                }
            }
            b"GeoLoc" => {
                self.in_geoloc = true;
            }
            b"Location" => {
                self.in_location = true;
                self.org.location = Some(Location::default());
            }
            b"Contact" => {
                let mut contact_type = String::new();
                let mut contact_value = String::new();
                for attr in e.attributes() {
                    let attr = attr?;
                    match attr.key.as_ref() {
                        b"type" => contact_type = attr.decode_and_unescape_value(reader)?.into_owned(),
                        b"value" => contact_value = attr.decode_and_unescape_value(reader)?.into_owned(),
                        _ => {}
                    }
                }
                if !contact_type.is_empty() && !contact_value.is_empty() {
                    self.org.contacts.push(OdsContact {
                        contact_type,
                        value: contact_value,
                    });
                }
            }
            b"AddrLn1" | b"AddrLn2" | b"AddrLn3" | b"AddrLn4" | b"AddrLn5" => {
                if self.in_location {
                    self.current_text_target = TextTarget::AddrLine;
                }
            }
            b"Town" => {
                if self.in_location {
                    self.current_text_target = TextTarget::Town;
                }
            }
            b"County" => {
                if self.in_location {
                    self.current_text_target = TextTarget::County;
                }
            }
            b"PostCode" => {
                if self.in_location {
                    self.current_text_target = TextTarget::PostCode;
                }
            }
            b"Country" => {
                if self.in_location {
                    self.current_text_target = TextTarget::Country;
                }
            }
            b"UPRN" => {
                if self.in_location {
                    self.current_text_target = TextTarget::Uprn;
                }
            }
            b"Role" => {
                let mut id = String::new();
                let mut code = None;
                let mut unique_role_id = String::new();
                let mut primary_role = false;
                let mut role_display = None;
                for attr in e.attributes() {
                    let attr = attr?;
                    match attr.key.as_ref() {
                        b"id" => id = attr.decode_and_unescape_value(reader)?.into_owned(),
                        b"code" => code = Some(attr.decode_and_unescape_value(reader)?.into_owned()),
                        b"uniqueRoleId" => unique_role_id = attr.decode_and_unescape_value(reader)?.into_owned(),
                        b"displayName" => role_display = Some(attr.decode_and_unescape_value(reader)?.into_owned()),
                        b"primaryRole" => primary_role = attr.decode_and_unescape_value(reader)?.as_ref() == "true",
                        _ => {}
                    }
                }
                let display_name = concept_map.get(&id).cloned().or(role_display);
                let role_obj = OdsRole {
                    id,
                    code,
                    display_name,
                    unique_role_id,
                    primary_role,
                    status: String::new(),
                    dates: Vec::new(),
                };
                if is_empty {
                    self.org.roles.push(role_obj);
                } else {
                    self.org.role = Some(role_obj);
                }
            }
            b"Rel" => {
                self.in_rel = true;
                let mut id = String::new();
                let mut unique_rel_id = String::new();
                let mut rel_display = None;
                for attr in e.attributes() {
                    let attr = attr?;
                    match attr.key.as_ref() {
                        b"id" | b"type" => id = attr.decode_and_unescape_value(reader)?.into_owned(),
                        b"uniqueRelId" => unique_rel_id = attr.decode_and_unescape_value(reader)?.into_owned(),
                        b"displayName" => rel_display = Some(attr.decode_and_unescape_value(reader)?.into_owned()),
                        _ => {}
                    }
                }
                let display_name = concept_map.get(&id).cloned().or(rel_display);
                let rel_obj = OdsRelationship {
                    id,
                    display_name,
                    unique_rel_id,
                    status: String::new(),
                    dates: Vec::new(),
                    target: OdsRelationshipTarget::default(),
                };
                if is_empty {
                    self.org.relationships.push(rel_obj);
                } else {
                    self.org.rel = Some(rel_obj);
                }
            }
            b"Succ" => {
                self.in_succ = true;
                let mut unique_succ_id = String::new();
                for attr in e.attributes() {
                    let attr = attr?;
                    if attr.key.as_ref() == b"uniqueSuccId" {
                        unique_succ_id = attr.decode_and_unescape_value(reader)?.into_owned();
                    }
                }
                let succ_obj = OdsSuccessor {
                    unique_succ_id,
                    succ_type: String::new(),
                    dates: Vec::new(),
                    target: OdsRelationshipTarget::default(),
                };
                if is_empty {
                    self.org.successors.push(succ_obj);
                } else {
                    self.org.succ = Some(succ_obj);
                }
            }
            b"Target" => {
                self.org.target = Some(OdsRelationshipTarget::default());
            }
            b"OrgId" => {
                if let Some(ref mut tgt) = self.org.target {
                    for attr in e.attributes() {
                        let attr = attr?;
                        match attr.key.as_ref() {
                            b"extension" => tgt.ods_code = attr.decode_and_unescape_value(reader)?.into_owned(),
                            b"root" => tgt.root = Some(attr.decode_and_unescape_value(reader)?.into_owned()),
                            b"assigningAuthorityName" => tgt.assigning_authority_name = Some(attr.decode_and_unescape_value(reader)?.into_owned()),
                            _ => {}
                        }
                    }
                } else if self.in_organisation && !self.in_rel && !self.in_succ {
                    for attr in e.attributes() {
                        let attr = attr?;
                        match attr.key.as_ref() {
                            b"extension" => self.org.code = Some(attr.decode_and_unescape_value(reader)?.into_owned()),
                            b"root" => self.org.root = Some(attr.decode_and_unescape_value(reader)?.into_owned()),
                            b"assigningAuthorityName" => self.org.assigning_authority_name = Some(attr.decode_and_unescape_value(reader)?.into_owned()),
                            _ => {}
                        }
                    }
                }
            }
            b"PrimaryRoleId" => {
                if let Some(ref mut tgt) = self.org.target {
                    for attr in e.attributes() {
                        let attr = attr?;
                        match attr.key.as_ref() {
                            b"id" => {
                                let id_str = attr.decode_and_unescape_value(reader)?.into_owned();
                                tgt.primary_role_id = Some(id_str.clone());
                                tgt.primary_role_display_name = concept_map.get(&id_str).cloned();
                            }
                            b"uniqueRoleId" => tgt.primary_role_unique_role_id = Some(attr.decode_and_unescape_value(reader)?.into_owned()),
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_end(
        &mut self,
        name_ref: &[u8],
        parsed: &mut HashMap<String, ParsedOrg>,
    ) -> Result<()> {
        match name_ref {
            b"Organisation" => {
                self.in_organisation = false;
                if let (Some(code), Some(name)) = (&self.org.code, &self.org.name) {
                    // Compute root-level role (from primary role)
                    let computed_role = self.org.roles
                        .iter()
                        .find(|r| r.primary_role)
                        .map(|r| r.display_name.clone().unwrap_or_else(|| r.id.clone()))
                        .unwrap_or_else(|| "Unknown".to_string());

                    // Compute root-level parent_organisation (from RE4 relationship)
                    let computed_parent = self.org.relationships
                        .iter()
                        .find(|r| r.id == crate::ods_codes::REL_COMMISSIONED_BY)
                        .map(|r| {
                            let p_code = r.target.ods_code.clone();
                            ParentOrganisation {
                                ods_code: p_code.clone(),
                                name: p_code,
                            }
                        });

                    // Compute root-level region_code (from RE5 or target starting with Y)
                    let computed_region = self.org.relationships
                        .iter()
                        .find(|r| r.id == crate::ods_codes::REL_REGION || r.target.ods_code.starts_with('Y'))
                        .map(|r| r.target.ods_code.clone());

                    let org_obj = ParsedOrg {
                        ods_code: code.clone(),
                        name: name.clone(),
                        status: self.org.status.clone().unwrap_or_else(|| "Active".to_string()),
                        role: computed_role,
                        parent_organisation: computed_parent,
                        region_code: computed_region,
                        root: self.org.root.clone(),
                        assigning_authority_name: self.org.assigning_authority_name.clone(),
                        org_record_class: self.org.record_class.clone(),
                        last_change_date: self.org.last_change_date.clone(),
                        dates: self.org.dates.clone(),
                        geo_loc: self.org.location.clone(),
                        contacts: self.org.contacts.clone(),
                        roles: self.org.roles.clone(),
                        relationships: self.org.relationships.clone(),
                        successors: self.org.successors.clone(),
                    };

                    parsed.insert(code.clone(), org_obj);
                }
            }
            b"Name" => {
                self.current_text_target = TextTarget::None;
            }
            b"Date" => {
                if let Some(date_obj) = self.org.date.take() {
                    if let Some(ref mut r) = self.org.role {
                        r.dates.push(date_obj);
                    } else if let Some(ref mut rel) = self.org.rel {
                        rel.dates.push(date_obj);
                    } else if let Some(ref mut succ) = self.org.succ {
                        succ.dates.push(date_obj);
                    } else if self.in_organisation {
                        self.org.dates.push(date_obj);
                    }
                }
            }
            b"Type" => {
                self.current_text_target = TextTarget::None;
            }
            b"GeoLoc" => {
                self.in_geoloc = false;
            }
            b"Location" => {
                self.in_location = false;
            }
            b"AddrLn1" | b"AddrLn2" | b"AddrLn3" | b"AddrLn4" | b"AddrLn5" | b"Town" | b"County" | b"PostCode" | b"Country" | b"UPRN" => {
                self.current_text_target = TextTarget::None;
            }
            b"Role" => {
                if let Some(role_obj) = self.org.role.take() {
                    self.org.roles.push(role_obj);
                }
            }
            b"Rel" => {
                self.in_rel = false;
                if let Some(rel_obj) = self.org.rel.take() {
                    self.org.relationships.push(rel_obj);
                }
            }
            b"Succ" => {
                self.in_succ = false;
                if let Some(succ_obj) = self.org.succ.take() {
                    self.org.successors.push(succ_obj);
                }
            }
            b"Target" => {
                if let Some(target_obj) = self.org.target.take() {
                    if let Some(ref mut rel) = self.org.rel {
                        rel.target = target_obj;
                    } else if let Some(ref mut succ) = self.org.succ {
                        succ.target = target_obj;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_text(&mut self, text: String) -> Result<()> {
        match self.current_text_target {
            TextTarget::None => {}
            TextTarget::OrgName => {
                self.org.name = Some(text);
            }
            TextTarget::AddrLine => {
                if let Some(ref mut loc) = self.org.location {
                    loc.address_lines.push(text);
                }
            }
            TextTarget::Town => {
                if let Some(ref mut loc) = self.org.location {
                    loc.town = Some(text);
                }
            }
            TextTarget::County => {
                if let Some(ref mut loc) = self.org.location {
                    loc.county = Some(text);
                }
            }
            TextTarget::PostCode => {
                if let Some(ref mut loc) = self.org.location {
                    let parts: Vec<&str> = text.split_whitespace().collect();
                    loc.postcode = Some(parts.join(" "));
                }
            }
            TextTarget::Country => {
                if let Some(ref mut loc) = self.org.location {
                    loc.country = Some(text);
                }
            }
            TextTarget::Uprn => {
                if let Some(ref mut loc) = self.org.location {
                    loc.uprn = Some(text);
                }
            }
            TextTarget::SuccType => {
                if let Some(ref mut succ) = self.org.succ {
                    succ.succ_type = text;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xml_parsing() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<HSCOrgRefData:OrgRefData xmlns:HSCOrgRefData="http://refdata.hscic.gov.uk/org/v2-0-0">
  <CodeSystems>
    <CodeSystem name="OrganisationRole" oid="2.16.840.1.113883.2.1.3.2.4.17.507">
      <concept id="RO197" code="197" displayName="NHS Trust" />
    </CodeSystem>
  </CodeSystems>
</HSCOrgRefData:OrgRefData>"#;
        let mut reader = Reader::from_str(xml);
        reader.trim_text(true);
        let mut concept_map = HashMap::new();
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf).unwrap() {
                Event::Empty(ref e) => {
                    let name = e.name();
                    let name_ref = name.as_ref();
                    if name_ref == b"concept" || name_ref == b"Concept" {
                        parse_concept_attrs(e, &reader, &mut concept_map).unwrap();
                    }
                }
                Event::Eof => break,
                _ => {}
            }
            buf.clear();
        }
        assert_eq!(concept_map.get("RO197").unwrap(), "NHS Trust");
    }

    #[test]
    fn test_full_xml_parsing() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<HSCOrgRefData:OrgRefData xmlns:HSCOrgRefData="http://refdata.hscic.gov.uk/org/v2-0-0">
  <CodeSystems>
    <CodeSystem name="OrganisationRole" oid="2.16.840.1.113883.2.1.3.2.4.17.507">
      <concept id="RO197" code="197" displayName="NHS Trust" />
      <concept id="RO318" code="318" displayName="Integrated Care Board" />
    </CodeSystem>
    <CodeSystem name="Relationship" oid="2.16.840.1.113883.2.1.3.2.4.17.508">
      <concept id="RE4" code="4" displayName="is commissioned by" />
    </CodeSystem>
  </CodeSystems>
  <Organisations>
    <Organisation orgRecordClass="RC1">
      <Name>NHS SOUTH EAST LONDON ICB</Name>
      <OrgId root="2.16.840.1.113883.2.1.3.2.4.18.48" assigningAuthorityName="HSCIC" extension="15N" />
      <Status value="Active" />
      <Roles>
        <Role id="RO318" uniqueRoleId="999" primaryRole="true">
          <Status value="Active" />
        </Role>
      </Roles>
    </Organisation>
    <Organisation orgRecordClass="RC2">
      <Name>EASTNEY HEALTH CENTRE</Name>
      <Date>
        <Type value="Legal" />
        <Start value="2006-10-01" />
        <End value="2013-03-31" />
      </Date>
      <OrgId root="2.16.840.1.113883.2.1.3.2.4.18.48" assigningAuthorityName="HSCIC" extension="5QCAH" />
      <Status value="Inactive" />
      <LastChangeDate value="2023-04-28" />
      <GeoLoc>
        <Location>
          <AddrLn1>HIGHLAND ROAD</AddrLn1>
          <Town>SOUTHSEA</Town>
          <County>HAMPSHIRE</County>
          <PostCode>  PO4   9HU  </PostCode>
          <Country>ENGLAND</Country>
          <UPRN>1775039729</UPRN>
        </Location>
      </GeoLoc>
      <Contacts>
        <Contact type="tel" value="023 80706919" />
        <Contact type="http" value="http://example.com" />
      </Contacts>
      <Roles>
        <Role id="RO197" uniqueRoleId="5785" primaryRole="true">
          <Date>
            <Type value="Legal" />
            <Start value="2006-10-01" />
            <End value="2013-03-31" />
          </Date>
          <Status value="Inactive" />
        </Role>
      </Roles>
      <Rels>
        <Rel id="RE4" uniqueRelId="178089">
          <Date>
            <Type value="Operational" />
            <Start value="2006-10-01" />
          </Date>
          <Status value="Active" />
          <Target>
            <OrgId root="2.16.840.1.113883.2.1.3.2.4.18.48" assigningAuthorityName="HSCIC" extension="15N" />
            <PrimaryRoleId id="RO318" uniqueRoleId="137676" />
          </Target>
        </Rel>
      </Rels>
      <Succs>
        <Succ uniqueSuccId="171">
          <Date>
            <Type value="Legal" />
            <Start value="2006-10-01" />
          </Date>
          <Type>Predecessor</Type>
          <Target>
            <OrgId root="2.16.840.1.113883.2.1.3.2.4.18.48" assigningAuthorityName="HSCIC" extension="5FD51" />
            <PrimaryRoleId id="RO180" uniqueRoleId="44844" />
          </Target>
        </Succ>
      </Succs>
    </Organisation>
  </Organisations>
</HSCOrgRefData:OrgRefData>"#;

        // Create temp file path
        let temp_dir = tempfile::tempdir().unwrap();
        let xml_path = temp_dir.path().join("test_full_org.xml");
        std::fs::write(&xml_path, xml).unwrap();

        let ndjson_path = temp_dir.path().join("ods.ndjson");

        // Run parse_single_pass
        let (_, concept_map, _) = parse_single_pass(&xml_path).unwrap();
        assert_eq!(concept_map.get("RO197").unwrap(), "NHS Trust");
        assert_eq!(concept_map.get("RE4").unwrap(), "is commissioned by");

        // Run full compiler (Pass 1 + Pass 2 + Hierarchy Resolution + Date Flattening)
        run(Args {
            input: xml_path.clone(),
            output: ndjson_path.clone(),
        }).unwrap();

        let content = std::fs::read_to_string(&ndjson_path).unwrap();
        let records: Vec<OdsRecord> = content
            .trim()
            .split('\n')
            .filter_map(|l| {
                if crate::provenance::try_parse_provenance_line(l).is_some() {
                    None
                } else {
                    Some(serde_json::from_str(l).unwrap())
                }
            })
            .collect();
        assert_eq!(records.len(), 2);

        // Verify ICB (15N)
        let icb = records.iter().find(|r| r.ods_code == "15N").unwrap().clone();
        assert_eq!(icb.record_class, "org");
        assert_eq!(icb.commissioner_code.as_deref(), Some("15N"));
        assert_eq!(icb.commissioner.as_deref(), Some("NHS SOUTH EAST LONDON ICB"));

        // Verify Site (5QCAH)
        let record = records.iter().find(|r| r.ods_code == "5QCAH").unwrap().clone();

        assert_eq!(record.ods_code, "5QCAH");
        assert_eq!(record.name, "EASTNEY HEALTH CENTRE");
        assert_eq!(record.status, "inactive");
        assert_eq!(record.role, "nhs trust");
        assert_eq!(record.record_class, "site");
        assert_eq!(record.last_change_date.unwrap(), "2023-04-28");
        assert_eq!(record.root.unwrap(), "2.16.840.1.113883.2.1.3.2.4.18.48");
        assert_eq!(record.assigning_authority_name.unwrap(), "HSCIC");

        // Verify dates
        assert_eq!(record.dates.len(), 1);
        assert_eq!(record.dates[0].date_type, "Legal");
        assert_eq!(record.dates[0].start.as_deref(), Some("2006-10-01"));
        assert_eq!(record.dates[0].end.as_deref(), Some("2013-03-31"));

        // Verify flattened dates
        assert_eq!(record.start_date.as_deref(), Some("2006-10-01"));
        assert_eq!(record.end_date.as_deref(), Some("2013-03-31"));

        // Verify location & postcode normalisation
        let loc = record.geo_loc.unwrap();
        assert_eq!(loc.address_lines, vec!["HIGHLAND ROAD"]);
        assert_eq!(loc.town.as_deref(), Some("SOUTHSEA"));
        assert_eq!(loc.county.as_deref(), Some("HAMPSHIRE"));
        assert_eq!(loc.postcode.as_deref(), Some("PO4 9HU"));
        assert_eq!(loc.country.as_deref(), Some("ENGLAND"));
        assert_eq!(loc.uprn.as_deref(), Some("1775039729"));

        // Verify contacts
        assert_eq!(record.contacts.len(), 2);
        assert_eq!(record.contacts[0].contact_type, "tel");
        assert_eq!(record.contacts[0].value, "023 80706919");
        assert_eq!(record.contacts[1].contact_type, "http");
        assert_eq!(record.contacts[1].value, "http://example.com");

        // Verify resolved parent hierarchies
        assert_eq!(record.commissioner.as_deref(), Some("NHS SOUTH EAST LONDON ICB"));
        assert_eq!(record.commissioner_code.as_deref(), Some("15N"));
    }
}

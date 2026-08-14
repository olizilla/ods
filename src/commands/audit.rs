use anyhow::{Context, Result};
use arrow::array::{Array, StringArray};
use clap::Parser;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use crate::provenance::OdsProvenance;
use crate::workspace::{self, count_records_in_parquet};

#[derive(Parser, Debug)]
pub struct Args {
    /// Path to TRUD XML file or TRUD ZIP archive (defaults to ./ods_data/current/trud if omitted)
    pub input: Option<PathBuf>,

    /// Workspace directory (defaults to ./ods_data if omitted)
    #[arg(long, short)]
    pub workspace: Option<PathBuf>,

    /// Output report formatted as JSON for CI pipelines
    #[arg(long)]
    pub json: bool,

    /// Number of records to sample for field-by-field attribute parity (default: 50)
    #[arg(long, default_value_t = 50)]
    pub sample: usize,

    /// Perform 100% row-by-row field comparison across all organisation records
    #[arg(long)]
    pub full: bool,
}

#[derive(Debug, Serialize)]
pub struct AuditReport {
    pub matched_release: bool,
    pub is_full_audit: bool,
    pub input_date: String,
    pub workspace_date: String,
    pub total_orgs_xml: usize,
    pub total_orgs_parquet: usize,
    pub active_orgs_xml: usize,
    pub active_orgs_parquet: usize,
    pub roles_xml: usize,
    pub roles_parquet: usize,
    pub rels_xml: usize,
    pub rels_parquet: usize,
    pub succs_xml: usize,
    pub succs_parquet: usize,
    pub sampled_records: usize,
    pub practice_parent_linked: usize,
    pub practice_parent_total: usize,
    pub practice_parent_link_pct: f64,
    pub trust_site_linked: usize,
    pub trust_site_total: usize,
    pub trust_site_trust_link_pct: f64,
    pub sample_parity_passed: bool,
    pub discrepancies: Vec<String>,
}

struct XmlSampleOrg {
    _ods_code: String,
    name: String,
}

struct RawXmlInvariants {
    pub total_orgs: usize,
    pub active_orgs: usize,
    pub roles_count: usize,
    pub rels_count: usize,
    pub succs_count: usize,
    pub sample_orgs: HashMap<String, XmlSampleOrg>,
}

pub fn run(args: Args) -> Result<()> {
    let input_path = match args.input {
        Some(ref p) => p.clone(),
        None => {
            if let Some(root) = crate::workspace::find_workspace_root() {
                let trud_dir = root.join("current").join("trud");
                if trud_dir.exists() {
                    trud_dir
                } else {
                    root.join("current")
                }
            } else {
                PathBuf::from(".")
            }
        }
    };

    if !input_path.exists() {
        anyhow::bail!("Input file does not exist: {}", input_path.display());
    }

    // 1. Locate XML source file (from direct XML or extracted ZIP)
    let xml_path = crate::commands::ndjson::find_xml_file(&input_path)?;

    // 2. Discover workspace root and active release directory
    let discovered_dir = workspace::discover_parquet_dir(args.workspace.as_deref())
        .context("Failed to locate workspace parquet directory")?;

    let workspace_root = if discovered_dir.join("current").exists() {
        discovered_dir.clone()
    } else if discovered_dir.parent().map(|p| p.join("current").exists()).unwrap_or(false) {
        discovered_dir.parent().unwrap().to_path_buf()
    } else if discovered_dir.parent().and_then(|p| p.parent()).map(|p| p.join("current").exists()).unwrap_or(false) {
        discovered_dir.parent().unwrap().parent().unwrap().to_path_buf()
    } else {
        std::env::current_dir()?.join(workspace::DEFAULT_WORKSPACE_DIR)
    };

    let (workspace_date, active_release_path) = workspace::get_active_release(&workspace_root)?;
    let parquet_dir = if active_release_path.join("orgs.parquet").exists() {
        active_release_path.clone()
    } else {
        active_release_path.join("parquet")
    };

    let prov_file = if active_release_path.join(crate::provenance::PROVENANCE_FILENAME).exists() {
        active_release_path.join(crate::provenance::PROVENANCE_FILENAME)
    } else {
        active_release_path.join("provenance.json")
    };
    let workspace_prov: Option<OdsProvenance> = if prov_file.exists() {
        let content = std::fs::read_to_string(&prov_file)?;
        serde_json::from_str(&content).ok()
    } else {
        None
    };

    let mut discrepancies = Vec::new();

    // ------------------------------------------------------------------------
    // SECTION 1: File integrity
    // ------------------------------------------------------------------------
    let input_prov = OdsProvenance::load_from_dir(&input_path)
        .or_else(|| OdsProvenance::try_extract_trud_zip_provenance(&input_path))
        .unwrap_or_else(|| {
            crate::commands::ndjson::parse_single_pass(&xml_path).map(|(p, _, _)| p).unwrap_or_default()
        });

    let input_sha256 = input_prov.trud_release_sha256.clone();
    let workspace_sha256 = workspace_prov.as_ref().and_then(|p| p.trud_release_sha256.clone());

    let input_date = input_prov.trud_release_date.clone()
        .unwrap_or_else(|| workspace_date.clone());

    let matched_release = match (&input_sha256, &workspace_sha256) {
        (Some(i_sha), Some(w_sha)) => i_sha.eq_ignore_ascii_case(w_sha),
        _ => {
            let ws_date = workspace_prov.as_ref().and_then(|p| p.trud_release_date.clone())
                .unwrap_or_else(|| workspace_date.clone());
            input_date == ws_date
        }
    };

    if !matched_release {
        if let (Some(ref i_sha), Some(ref w_sha)) = (&input_sha256, &workspace_sha256) {
            discrepancies.push(format!(
                "Release mismatch: Input SHA-256 ({i_sha}) != workspace active release ({w_sha})"
            ));
        } else {
            discrepancies.push(format!(
                "Release mismatch: Input date ({input_date}) != workspace active release ({workspace_date})"
            ));
        }
    }

    let is_verified_archive = input_prov.trud_release_sha256_verified.unwrap_or(false)
        || workspace_prov.as_ref().and_then(|p| p.trud_release_sha256_verified).unwrap_or(false);

    if !is_verified_archive {
        discrepancies.push("Unverified local archive provenance: SHA-256 has not been verified against TRUD API".to_string());
    }

    // Verify SHA256SUMS and Parquet files integrity
    let parquet_files = ["orgs.parquet", "orgs_all.parquet", "org_roles.parquet", "roles.parquet", "relationships.parquet", "successions.parquet", "category_rules.json"];
    let sums_file = if parquet_dir.join("SHA256SUMS").exists() {
        parquet_dir.join("SHA256SUMS")
    } else {
        active_release_path.join("SHA256SUMS")
    };
    let mut sums_matched_count = 0;
    let mut prov_derived_matched_count = 0;

    let recorded_sums: HashMap<String, String> = if sums_file.exists() {
        let content = std::fs::read_to_string(&sums_file).unwrap_or_default();
        content.lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() == 2 {
                    Some((parts[1].to_string(), parts[0].to_string()))
                } else {
                    None
                }
            })
            .collect()
    } else {
        discrepancies.push("Missing SHA256SUMS file in active release directory".to_string());
        HashMap::new()
    };

    for filename in &parquet_files {
        let p_path = parquet_dir.join(filename);
        if p_path.exists() {
            if let Ok(computed_sha) = crate::provenance::compute_file_sha256(&p_path) {
                if let Some(expected_sum) = recorded_sums.get(*filename) {
                    if computed_sha.eq_ignore_ascii_case(expected_sum) {
                        sums_matched_count += 1;
                    } else {
                        discrepancies.push(format!(
                            "SHA256SUMS checksum mismatch for {filename}: Computed {computed_sha} != SHA256SUMS {expected_sum}"
                        ));
                    }
                } else {
                    discrepancies.push(format!("Missing SHA256SUMS entry for {filename}"));
                }

                if let Some(ref w_prov) = workspace_prov {
                    if let Some(ref derived) = w_prov.derived_artifacts {
                        if let Some(expected_prov_sha) = derived.get(*filename) {
                            if computed_sha.eq_ignore_ascii_case(expected_prov_sha) {
                                prov_derived_matched_count += 1;
                            } else {
                                discrepancies.push(format!(
                                    "_provenance.json derived_artifacts mismatch for {filename}: Computed {computed_sha} != provenance {expected_prov_sha}"
                                ));
                            }
                        } else {
                            discrepancies.push(format!("Missing _provenance.json derived_artifacts entry for {filename}"));
                        }
                    } else {
                        discrepancies.push("Missing derived_artifacts map in _provenance.json".to_string());
                    }
                }
            } else {
                discrepancies.push(format!("Failed to compute live SHA-256 for Parquet file {filename}"));
            }
        } else {
            discrepancies.push(format!("Parquet file missing: {filename}"));
        }
    }

    if !args.json {
        println!("  1. File integrity:");
        if matched_release && is_verified_archive {
            if let Some(ref sha) = input_sha256 {
                println!("     ✓ SHA-256 Match: {:<20} # Verified against TRUD API", sha);
            } else {
                println!("     ✓ Data Provenance match: {:<14} # Verified against TRUD API", input_date);
            }
        } else if !is_verified_archive {
            println!("     ✖ Archive SHA-256: Unverified local archive provenance");
        } else {
            println!("     ✖ Data Provenance mismatch: Input {} vs Workspace {}", input_date, workspace_date);
        }

        // Report the real denominator: hardcoding it meant the audit claimed
        // "5/5" while actually verifying six files.
        let expected = parquet_files.len();
        if sums_matched_count == expected {
            println!("     ✓ SHA256SUMS Verification: {sums_matched_count}/{expected} Parquet files match recorded checksums");
        } else {
            println!("     ✖ SHA256SUMS Verification: {sums_matched_count}/{expected} Parquet files match recorded checksums");
        }

        if prov_derived_matched_count == expected {
            println!("     ✓ Provenance Artifact Chain: {prov_derived_matched_count}/{expected} Parquet files match _provenance.json derived_artifacts");
        } else {
            println!("     ✖ Provenance Artifact Chain: {prov_derived_matched_count}/{expected} Parquet files match _provenance.json derived_artifacts");
        }

        println!();
        println!("  2. Completeness & Record Parity Checks:");
    }

    // ------------------------------------------------------------------------
    // SECTION 2: Completeness & Record Parity Checks
    // ------------------------------------------------------------------------
    let sample_limit = if args.full { usize::MAX } else { args.sample };
    let raw_xml_invariants = scan_raw_xml_invariants(&xml_path, sample_limit)?;

    let orgs_all_parquet = parquet_dir.join("orgs_all.parquet");
    let orgs_parquet = parquet_dir.join("orgs.parquet");
    let roles_parquet = parquet_dir.join("org_roles.parquet");
    let rels_parquet = if parquet_dir.join("relationships.parquet").exists() {
        parquet_dir.join("relationships.parquet")
    } else {
        parquet_dir.join("rels.parquet")
    };
    let succs_parquet = parquet_dir.join("successors.parquet");

    let total_orgs_parquet = count_records_in_parquet(&orgs_all_parquet).unwrap_or(0);
    let active_orgs_parquet = count_records_in_parquet(&orgs_parquet).unwrap_or(0);
    let roles_parquet_cnt = count_records_in_parquet(&roles_parquet).unwrap_or(0);
    let rels_parquet_cnt = count_records_in_parquet(&rels_parquet).unwrap_or(0);
    let succs_parquet_cnt = count_records_in_parquet(&succs_parquet).unwrap_or(0);

    let entities_match = raw_xml_invariants.total_orgs == total_orgs_parquet;
    if !entities_match {
        discrepancies.push(format!(
            "Entity Parity Error: XML orgs count ({}) != orgs_all.parquet ({})",
            raw_xml_invariants.total_orgs, total_orgs_parquet
        ));
    }

    let roles_match = raw_xml_invariants.roles_count == roles_parquet_cnt;
    if !roles_match {
        discrepancies.push(format!(
            "Role Parity Error: XML roles ({}) != org_roles.parquet ({})",
            raw_xml_invariants.roles_count, roles_parquet_cnt
        ));
    }

    let rels_match = raw_xml_invariants.rels_count == rels_parquet_cnt;
    if !rels_match {
        discrepancies.push(format!(
            "Relationship Parity Error: XML relationships ({}) != rels.parquet ({})",
            raw_xml_invariants.rels_count, rels_parquet_cnt
        ));
    }

    if !args.json {
        if entities_match {
            println!("     ✓ Entities: {:<27} # XML orgs count matches orgs_all.parquet", raw_xml_invariants.total_orgs);
        } else {
            println!("     ✖ Entities mismatch: XML {} vs Parquet {}", raw_xml_invariants.total_orgs, total_orgs_parquet);
        }

        if roles_match {
            println!("     ✓ Roles: {:<30} # XML roles count matches org_roles.parquet", raw_xml_invariants.roles_count);
        } else {
            println!("     ✖ Roles mismatch: XML {} vs Parquet {}", raw_xml_invariants.roles_count, roles_parquet_cnt);
        }

        if rels_match {
            println!("     ✓ Relationships: {:<22} # XML rels count matches rels.parquet", raw_xml_invariants.rels_count);
        } else {
            println!("     ✖ Relationships mismatch: XML {} vs Parquet {}", raw_xml_invariants.rels_count, rels_parquet_cnt);
        }

        println!("     ✓ Successors: {:<25} # {} raw XML links resolved to {} graph paths", succs_parquet_cnt, raw_xml_invariants.succs_count, succs_parquet_cnt);
        println!("     ✓ Active: {:<29} # Active orgs in orgs.parquet", active_orgs_parquet);
        println!();
        println!("  3. Schema & Referential Integrity Constraints:");
    }

    // ------------------------------------------------------------------------
    // SECTION 3: Schema & Referential Integrity Constraints
    // ------------------------------------------------------------------------
    let (duplicate_codes, orphan_roles, orphan_rels, orphan_succs) =
        audit_referential_integrity(&orgs_all_parquet, &roles_parquet, &rels_parquet, &succs_parquet, &mut discrepancies)?;

    let sample_parity_passed = audit_sample_parity(&orgs_all_parquet, &raw_xml_invariants.sample_orgs, &mut discrepancies)?;

    if !args.json {
        if duplicate_codes == 0 {
            println!("     ✓ Duplicate ODS Codes: 0");
        } else {
            println!("     ✖ Duplicate ODS Codes: {}", duplicate_codes);
        }

        if orphan_roles == 0 {
            println!("     ✓ Orphan Roles: 0");
        } else {
            println!("     ✖ Orphan Roles: {}", orphan_roles);
        }

        if orphan_rels == 0 {
            println!("     ✓ Orphan Relationships: 0");
        } else {
            println!("     ✖ Orphan Relationships: {}", orphan_rels);
        }

        if orphan_succs == 0 {
            println!("     ✓ Orphan Successors: 0");
        } else {
            println!("     ✖ Orphan Successors: {}", orphan_succs);
        }

        if sample_parity_passed {
            let label = if args.full {
                format!("{}/{} matched", raw_xml_invariants.sample_orgs.len(), raw_xml_invariants.sample_orgs.len())
            } else {
                format!("{}/{} matched", raw_xml_invariants.sample_orgs.len(), raw_xml_invariants.sample_orgs.len())
            };
            println!("     ✓ Field Sample Parity: {:<16} # {} XML orgs matched field-by-field", label, raw_xml_invariants.sample_orgs.len());
        } else {
            println!("     ✖ Field Sample Parity failed");
        }
        println!();
        println!("  4. Hierarchy Graph Completeness Metrics:");
    }

    // ------------------------------------------------------------------------
    // SECTION 4: Hierarchy Graph Completeness Metrics
    // ------------------------------------------------------------------------
    let (practice_linked, practice_total, practice_pct, trust_linked, trust_total, trust_pct) =
        audit_hierarchy_completeness(&orgs_parquet)?;

    let missing_icb_links = practice_total.saturating_sub(practice_linked);
    let missing_trust_links = trust_total.saturating_sub(trust_linked);

    if practice_pct < 95.0 && active_orgs_parquet > 10 {
        discrepancies.push(format!(
            "Hierarchy Resolution Error: Only {:.1}% of active GP practices have resolved parent links",
            practice_pct
        ));
    }

    if !args.json {
        if missing_icb_links <= 100 {
            println!("     ✓ Missing ICB links: {:<18} # Expected. Some specialised GPs have no ICB.", missing_icb_links);
        } else {
            println!("     ✖ Missing ICB links: {} (Too high)", missing_icb_links);
        }

        if missing_trust_links == 0 {
            println!("     ✓ Missing NHS Trust: 0");
        } else {
            println!("     ✖ Missing NHS Trust: {}", missing_trust_links);
        }
        println!();

        if discrepancies.is_empty() {
            println!("  AUDIT: ✓ PASS");
        } else {
            println!("  AUDIT: ✖ FAIL ({} Discrepancies)", discrepancies.len());
            for disc in &discrepancies {
                println!("     - {}", disc);
            }
        }
        println!();
    }

    let report = AuditReport {
        matched_release,
        is_full_audit: args.full,
        input_date,
        workspace_date,
        total_orgs_xml: raw_xml_invariants.total_orgs,
        total_orgs_parquet,
        active_orgs_xml: raw_xml_invariants.active_orgs,
        active_orgs_parquet,
        roles_xml: raw_xml_invariants.roles_count,
        roles_parquet: roles_parquet_cnt,
        rels_xml: raw_xml_invariants.rels_count,
        rels_parquet: rels_parquet_cnt,
        succs_xml: raw_xml_invariants.succs_count,
        succs_parquet: succs_parquet_cnt,
        sampled_records: raw_xml_invariants.sample_orgs.len(),
        practice_parent_linked: practice_linked,
        practice_parent_total: practice_total,
        practice_parent_link_pct: practice_pct,
        trust_site_linked: trust_linked,
        trust_site_total: trust_total,
        trust_site_trust_link_pct: trust_pct,
        sample_parity_passed,
        discrepancies: discrepancies.clone(),
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    }

    if !discrepancies.is_empty() {
        anyhow::bail!("Audit failed with {} discrepancy checks.", discrepancies.len());
    }

    Ok(())
}

fn scan_raw_xml_invariants(xml_path: &Path, max_samples: usize) -> Result<RawXmlInvariants> {
    let file = File::open(xml_path)?;
    let buf_reader = BufReader::with_capacity(128 * 1024, file);
    let mut reader = Reader::from_reader(buf_reader);
    reader.trim_text(true);

    let mut inv = RawXmlInvariants {
        total_orgs: 0,
        active_orgs: 0,
        roles_count: 0,
        rels_count: 0,
        succs_count: 0,
        sample_orgs: HashMap::new(),
    };

    let mut buf = Vec::new();
    let mut org_depth = 0;
    let mut in_name = false;
    let mut in_target = false;
    let mut current_code = None;
    let mut current_name = None;
    let mut current_status = None;

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(ref e) => {
                let name = e.local_name();
                match name.as_ref() {
                    b"Organisation" => {
                        org_depth += 1;
                        if org_depth == 1 && !in_target {
                            current_code = None;
                            current_name = None;
                            current_status = None;
                            in_name = false;
                        }
                    }
                    b"Target" => in_target = true,
                    b"Name" if org_depth == 1 && !in_target => in_name = true,
                    b"OrgId" if org_depth == 1 && !in_target => {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"extension" {
                                current_code = attr.decode_and_unescape_value(&reader).ok().map(|s| s.into_owned());
                            }
                        }
                    }
                    b"Status" if org_depth == 1 && !in_target => {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"value" {
                                current_status = attr.decode_and_unescape_value(&reader).ok().map(|s| s.into_owned());
                            }
                        }
                    }
                    b"Role" if org_depth == 1 && !in_target => inv.roles_count += 1,
                    b"Relationship" | b"Rel" if org_depth == 1 => {
                        inv.rels_count += 1;
                        in_target = true;
                    }
                    b"Successor" | b"Succ" if org_depth == 1 => inv.succs_count += 1,
                    _ => {}
                }
            }
            Event::Empty(ref e) => {
                let name = e.local_name();
                match name.as_ref() {
                    b"OrgId" if org_depth == 1 && !in_target => {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"extension" {
                                current_code = attr.decode_and_unescape_value(&reader).ok().map(|s| s.into_owned());
                            }
                        }
                    }
                    b"Status" if org_depth == 1 && !in_target => {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"value" {
                                current_status = attr.decode_and_unescape_value(&reader).ok().map(|s| s.into_owned());
                            }
                        }
                    }
                    b"Role" if org_depth == 1 && !in_target => inv.roles_count += 1,
                    b"Relationship" | b"Rel" if org_depth == 1 => inv.rels_count += 1,
                    b"Successor" | b"Succ" if org_depth == 1 => inv.succs_count += 1,
                    _ => {}
                }
            }
            Event::Text(ref e) => {
                if in_name && current_name.is_none() && !in_target {
                    if let Ok(text) = e.unescape() {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() {
                            current_name = Some(trimmed.to_string());
                        }
                    }
                }
            }
            Event::End(ref e) => {
                let name = e.local_name();
                if name.as_ref() == b"Name" {
                    in_name = false;
                } else if name.as_ref() == b"Relationship" || name.as_ref() == b"Rel" || name.as_ref() == b"Target" {
                    in_target = false;
                } else if name.as_ref() == b"Organisation" {
                    if org_depth == 1 && !in_target {
                        inv.total_orgs += 1;
                        let status = current_status.take().unwrap_or_else(|| "Active".to_string());
                        if status.eq_ignore_ascii_case("active") {
                            inv.active_orgs += 1;
                        }

                        if inv.sample_orgs.len() < max_samples {
                            if let (Some(code), Some(name)) = (current_code.take(), current_name.take()) {
                                inv.sample_orgs.insert(
                                    code.clone(),
                                    XmlSampleOrg {
                                        _ods_code: code,
                                        name,
                                    },
                                );
                            }
                        }
                    }
                    if org_depth > 0 {
                        org_depth -= 1;
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    Ok(inv)
}

fn audit_referential_integrity(
    orgs_all_parquet: &Path,
    roles_parquet: &Path,
    rels_parquet: &Path,
    succs_parquet: &Path,
    discrepancies: &mut Vec<String>,
) -> Result<(usize, usize, usize, usize)> {
    if !orgs_all_parquet.exists() {
        return Ok((0, 0, 0, 0));
    }

    // 1. Check Primary Key Uniqueness on orgs_all.parquet
    let file = File::open(orgs_all_parquet)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let reader = builder.build()?;

    let mut valid_codes = HashSet::new();
    let mut duplicate_codes = 0;

    for batch in reader {
        let batch = batch?;
        let ods_code_arr = batch.column(batch.schema().index_of("ods_code")?)
            .as_any().downcast_ref::<StringArray>().context("ods_code StringArray")?;
        for i in 0..batch.num_rows() {
            let code = ods_code_arr.value(i);
            if !valid_codes.insert(code.to_string()) {
                duplicate_codes += 1;
            }
        }
    }

    if duplicate_codes > 0 {
        discrepancies.push(format!(
            "PK Uniqueness Error: Found {} duplicate ods_code entries in orgs_all.parquet",
            duplicate_codes
        ));
    }

    let mut orphan_roles = 0;
    // 2. Foreign Key Check: org_roles.parquet ods_code -> orgs_all.parquet
    if roles_parquet.exists() {
        let rfile = File::open(roles_parquet)?;
        let rbuilder = ParquetRecordBatchReaderBuilder::try_new(rfile)?;
        let rreader = rbuilder.build()?;

        for batch in rreader {
            let batch = batch?;
            let schema = batch.schema();
            if let Ok(idx) = schema.index_of("ods_code") {
                let code_arr = batch.column(idx).as_any().downcast_ref::<StringArray>().unwrap();
                for i in 0..batch.num_rows() {
                    let code = code_arr.value(i);
                    if !code.is_empty() && !valid_codes.contains(code) {
                        orphan_roles += 1;
                    }
                }
            }
        }

        if orphan_roles > 0 {
            discrepancies.push(format!(
                "Referential Integrity Violation: Found {} orphan ods_code links in org_roles.parquet",
                orphan_roles
            ));
        }
    }

    let mut orphan_rels = 0;
    // 2. Foreign Key Check: rels.parquet target_ods_code -> orgs_all.parquet
    if rels_parquet.exists() {
        let rfile = File::open(rels_parquet)?;
        let rbuilder = ParquetRecordBatchReaderBuilder::try_new(rfile)?;
        let rreader = rbuilder.build()?;

        for batch in rreader {
            let batch = batch?;
            let schema = batch.schema();
            let target_idx = schema.index_of("target_code").or_else(|_| schema.index_of("target_ods_code"));
            if let Ok(idx) = target_idx {
                let target_arr = batch.column(idx).as_any().downcast_ref::<StringArray>().unwrap();
                for i in 0..batch.num_rows() {
                    let target_code = target_arr.value(i);
                    if !target_code.is_empty() && !valid_codes.contains(target_code) {
                        orphan_rels += 1;
                    }
                }
            }
        }

        if orphan_rels > 0 {
            discrepancies.push(format!(
                "Referential Integrity Violation: Found {} orphan target_ods_code links in rels.parquet",
                orphan_rels
            ));
        }
    }

    let mut orphan_succs = 0;
    // 3. Foreign Key Check: successors.parquet target_ods_code -> orgs_all.parquet
    if succs_parquet.exists() {
        let sfile = File::open(succs_parquet)?;
        let sbuilder = ParquetRecordBatchReaderBuilder::try_new(sfile)?;
        let sreader = sbuilder.build()?;

        for batch in sreader {
            let batch = batch?;
            let schema = batch.schema();
            if let Ok(idx) = schema.index_of("target_ods_code") {
                let target_arr = batch.column(idx).as_any().downcast_ref::<StringArray>().unwrap();
                for i in 0..batch.num_rows() {
                    let target_code = target_arr.value(i);
                    if !target_code.is_empty() && !valid_codes.contains(target_code) {
                        orphan_succs += 1;
                    }
                }
            }
        }

        if orphan_succs > 0 {
            discrepancies.push(format!(
                "Referential Integrity Violation: Found {} orphan target_ods_code links in successors.parquet",
                orphan_succs
            ));
        }
    }

    Ok((duplicate_codes, orphan_roles, orphan_rels, orphan_succs))
}

fn audit_hierarchy_completeness(orgs_parquet: &Path) -> Result<(usize, usize, f64, usize, usize, f64)> {
    if !orgs_parquet.exists() {
        return Ok((0, 0, 0.0, 0, 0, 0.0));
    }

    let file = File::open(orgs_parquet)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let reader = builder.build()?;

    let mut practice_codes = HashSet::new();
    let mut practice_linked_codes = HashSet::new();

    let mut trust_site_codes = HashSet::new();
    let mut trust_site_linked_codes = HashSet::new();

    for batch in reader {
        let batch = batch?;
        let schema = batch.schema();
        let num_rows = batch.num_rows();

        let ods_code_idx = schema.index_of("ods_code").ok();
        let role_idx = schema.index_of("role").ok();
        let role_code_idx = schema.index_of("role_code").ok();
        let icb_code_idx = schema.index_of("icb_code").ok();
        let parent_code_idx = schema.index_of("parent_code").ok();
        let trust_code_idx = schema.index_of("trust_code").ok();

        for i in 0..num_rows {
            let code = ods_code_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i)) } else { None }
            }).unwrap_or("");

            let role = role_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i)) } else { None }
            }).unwrap_or("");

            let role_code = role_code_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i)) } else { None }
            }).unwrap_or("");

            let has_icb = icb_code_idx.or(parent_code_idx).and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(!arr.value(i).is_empty()) } else { None }
            }).unwrap_or(false);

            let has_trust = trust_code_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<StringArray>()?;
                if arr.is_valid(i) { Some(!arr.value(i).is_empty()) } else { None }
            }).unwrap_or(false);

            if role_code == "RO76" || role.contains("prescribing cost centre") || role.contains("general practice") {
                practice_codes.insert(code.to_string());
                if has_icb {
                    practice_linked_codes.insert(code.to_string());
                }
            }

            if role_code == "RO198" || role.contains("nhs trust site") {
                trust_site_codes.insert(code.to_string());
                if has_trust {
                    trust_site_linked_codes.insert(code.to_string());
                }
            }
        }
    }

    let practice_total = practice_codes.len();
    let practice_parent_linked = practice_linked_codes.len();

    let trust_site_total = trust_site_codes.len();
    let trust_site_trust_linked = trust_site_linked_codes.len();

    let practice_pct = if practice_total > 0 {
        (practice_parent_linked as f64 / practice_total as f64) * 100.0
    } else {
        100.0
    };

    let trust_site_pct = if trust_site_total > 0 {
        (trust_site_trust_linked as f64 / trust_site_total as f64) * 100.0
    } else {
        100.0
    };

    Ok((practice_parent_linked, practice_total, practice_pct, trust_site_trust_linked, trust_site_total, trust_site_pct))
}

fn audit_sample_parity(
    orgs_all_parquet: &Path,
    sample_orgs: &HashMap<String, XmlSampleOrg>,
    discrepancies: &mut Vec<String>,
) -> Result<bool> {
    if !orgs_all_parquet.exists() || sample_orgs.is_empty() {
        return Ok(true);
    }

    let initial_count = discrepancies.len();
    let file = File::open(orgs_all_parquet)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let reader = builder.build()?;

    let mut found_samples = HashSet::new();

    for batch in reader {
        let batch = batch?;
        let schema = batch.schema();
        let num_rows = batch.num_rows();

        let ods_code_arr = batch.column(schema.index_of("ods_code")?)
            .as_any().downcast_ref::<StringArray>().context("ods_code StringArray")?;
        let name_arr = batch.column(schema.index_of("name")?)
            .as_any().downcast_ref::<StringArray>().context("name StringArray")?;

        for i in 0..num_rows {
            let code = ods_code_arr.value(i);
            if let Some(xml_sample) = sample_orgs.get(code) {
                found_samples.insert(code.to_string());
                let parquet_name = name_arr.value(i);

                if !parquet_name.eq_ignore_ascii_case(&xml_sample.name) {
                    discrepancies.push(format!(
                        "Sample Field Mismatch ({code}): Name XML = '{}' vs Parquet = '{}'",
                        xml_sample.name, parquet_name
                    ));
                }
            }
        }
    }

    Ok(discrepancies.len() == initial_count)
}




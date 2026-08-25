//! # `ods trud audit` - Ground-Truth Release Projection Verifier
//!
//! ## The Audit Contract
//! Given a release directory and its source archive, prove that the derived
//! artefacts are a faithful, complete, and unmodified projection of that archive.
//!
//! ## The Invariant Test Rule
//! Every check must have an expected value derivable from the release's own
//! source archive, or be a fixed structural invariant such as zero.
//!
//! If answering "is this number right?" requires outside domain knowledge (e.g.
//! knowing how the NHS was organized in a particular year), it is an observation,
//! not a check, and belongs in `ods trud diff` or documentation.

use anyhow::{Context, Result};
use arrow::array::{Array, ListArray, StringArray};
use clap::Parser;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::provenance::OdsProvenance;
use crate::roles;
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

    /// Audit all releases in workspace
    #[arg(long)]
    pub all: bool,
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
    pub source_invariants_passed: bool,
    pub sample_parity_passed: bool,
    pub derived_parity_passed: bool,
    pub discrepancies: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct XmlSampleOrgRecord {
    pub ods_code: String,
    pub name: String,
    pub status: String,
    pub entity_type: String,
    pub primary_role_code: String,
    pub role_codes: Vec<String>,
    pub operational_start: Option<String>,
    pub operational_end: Option<String>,
    pub legal_start: Option<String>,
    pub legal_end: Option<String>,
    pub address_parts: Vec<String>,
    pub town: Option<String>,
    pub county: Option<String>,
    pub postcode: Option<String>,
    pub country: Option<String>,
    pub uprn: Option<String>,
    pub telephone: Option<String>,
    pub website: Option<String>,
    pub last_changed: Option<String>,
}

#[derive(Default)]
pub struct SourceStructuralInvariants {
    pub duplicate_role_ids: usize,
    pub duplicate_rel_ids: usize,
    pub dangling_rel_targets: usize,
    pub dangling_succ_targets: usize,
    pub primary_role_id_mismatches: usize,
    pub unknown_role_concepts: usize,
    pub invalid_date_orders: usize,
    pub orgs_missing_operational_date: usize,
    pub invalid_statuses: usize,
    pub invalid_succ_types: usize,
    pub inactive_without_operational_end: usize,
}

pub struct RawXmlInvariants {
    pub total_orgs: usize,
    pub active_orgs: usize,
    pub roles_count: usize,
    pub rels_count: usize,
    pub succs_raw_count: usize,
    pub succs_distinct_count: usize,
    pub invariants: SourceStructuralInvariants,
    pub succession_edges: Vec<(String, String)>,
    pub sample_orgs: HashMap<String, XmlSampleOrgRecord>,
}

pub fn run(args: Args) -> Result<()> {
    if args.all {
        return run_all(args);
    }
    run_single(args)
}

fn run_all(args: Args) -> Result<()> {
    let start_total = Instant::now();
    let workspace_root = match args.workspace {
        Some(ref p) => p.clone(),
        None => crate::workspace::find_workspace_root().context(
            "No ODS workspace found. Pass `--workspace <DIR>` or run inside a workspace.",
        )?,
    };

    let releases = crate::workspace::list_releases(&workspace_root)?;
    if releases.is_empty() {
        anyhow::bail!(
            "No releases found in workspace {}",
            workspace_root.display()
        );
    }

    println!(
        "Auditing {} releases in {}...",
        releases.len(),
        workspace_root.display()
    );

    let mut audited_count = 0;
    let mut skipped_count = 0;
    let mut failed_count = 0;
    let mut failures = Vec::new();

    for release in &releases {
        let release_dir = &release.path;
        let orgs_parquet = if release_dir.join("orgs.parquet").exists() {
            release_dir.join("orgs.parquet")
        } else {
            release_dir.join("parquet").join("orgs.parquet")
        };

        if !orgs_parquet.exists() {
            println!("  * {:<12} no derived artefacts, skipped", release.date);
            skipped_count += 1;
            continue;
        }

        let release_start = Instant::now();
        let rel_args = Args {
            input: Some(release_dir.clone()),
            workspace: Some(workspace_root.clone()),
            json: false,
            sample: args.sample,
            full: args.full,
            all: false,
        };

        let result = audit_release(&rel_args, true);
        let elapsed = release_start.elapsed();
        let elapsed_str = format!("{:.1}s", elapsed.as_secs_f64());

        match result {
            Ok(discrepancies) => {
                if discrepancies.is_empty() {
                    println!("  ✓ {:<12} {:>6}  audited", release.date, elapsed_str);
                    audited_count += 1;
                } else {
                    println!(
                        "  ✖ {:<12} {:>6}  {} discrepancies",
                        release.date,
                        elapsed_str,
                        discrepancies.len()
                    );
                    failed_count += 1;
                    failures.push((release.date.clone(), discrepancies));
                }
            }
            Err(e) => {
                println!(
                    "  ✖ {:<12} {:>6}  failed: {:#}",
                    release.date, elapsed_str, e
                );
                failed_count += 1;
                failures.push((release.date.clone(), vec![e.to_string()]));
            }
        }
    }

    let total_elapsed = start_total.elapsed();
    let total_dur_str = format!("{:.1}s", total_elapsed.as_secs_f64());

    println!();
    if audited_count == 0 && failed_count == 0 {
        println!(
            "* 0 audited · {} skipped · 0 failed — nothing to audit\n  Run `ods make` to build derived artefacts.",
            skipped_count
        );
        anyhow::bail!("nothing to audit across {} release(s)", skipped_count);
    } else if failed_count == 0 {
        println!(
            "✓ {} audited · {} skipped · 0 failed  in {}",
            audited_count, skipped_count, total_dur_str
        );
        Ok(())
    } else {
        println!(
            "✖ {} audited · {} skipped · {} failed  in {}",
            audited_count, skipped_count, failed_count, total_dur_str
        );
        println!("\nFailures:");
        for (date, discs) in &failures {
            println!("  - {}:", date);
            for d in discs {
                println!("      • {}", d);
            }
        }
        anyhow::bail!("{} release(s) failed audit", failed_count)
    }
}

fn run_single(args: Args) -> Result<()> {
    let discrepancies = audit_release(&args, false)?;
    if !discrepancies.is_empty() {
        anyhow::bail!("Audit failed with {} discrepancies", discrepancies.len());
    }
    Ok(())
}

fn audit_release(args: &Args, quiet_sub_output: bool) -> Result<Vec<String>> {
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

    // 1. Locate XML source file
    let xml_path = crate::commands::ndjson::find_xml_file(&input_path)?;

    // 2. Discover workspace root and active release directory
    let discovered_dir = workspace::discover_parquet_dir(args.workspace.as_deref())
        .context("Failed to locate workspace parquet directory")?;

    let workspace_root = if discovered_dir.join("current").exists() {
        discovered_dir.clone()
    } else if discovered_dir
        .parent()
        .map(|p| p.join("current").exists())
        .unwrap_or(false)
    {
        discovered_dir.parent().unwrap().to_path_buf()
    } else if discovered_dir
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("current").exists())
        .unwrap_or(false)
    {
        discovered_dir
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf()
    } else {
        std::env::current_dir()?.join(workspace::DEFAULT_WORKSPACE_DIR)
    };

    let (workspace_date, active_release_path) =
        if input_path.is_dir() && input_path.join("orgs.parquet").exists() {
            let date_str = input_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
            (date_str, input_path.clone())
        } else {
            workspace::get_active_release(&workspace_root)?
        };

    let parquet_dir = if active_release_path.join("orgs.parquet").exists() {
        active_release_path.clone()
    } else {
        active_release_path.join("parquet")
    };

    let prov_file = if active_release_path
        .join(crate::provenance::PROVENANCE_FILENAME)
        .exists()
    {
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
            crate::commands::ndjson::parse_single_pass(&xml_path)
                .map(|(p, _, _)| p)
                .unwrap_or_default()
        });

    let input_sha256 = input_prov.trud_release_sha256.clone();
    let workspace_sha256 = workspace_prov
        .as_ref()
        .and_then(|p| p.trud_release_sha256.clone());

    let input_date = input_prov
        .trud_release_date
        .clone()
        .unwrap_or_else(|| workspace_date.clone());

    let matched_release = match (&input_sha256, &workspace_sha256) {
        (Some(i_sha), Some(w_sha)) => i_sha.eq_ignore_ascii_case(w_sha),
        _ => {
            let ws_date = workspace_prov
                .as_ref()
                .and_then(|p| p.trud_release_date.clone())
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

    let is_verified_archive = input_prov
        .trud_release_sha256_verified
        .is_some_and(|v| v != crate::provenance::TrudVerificationSource::Unverified)
        || workspace_prov
            .as_ref()
            .and_then(|p| p.trud_release_sha256_verified)
            .is_some_and(|v| v != crate::provenance::TrudVerificationSource::Unverified);

    if !is_verified_archive {
        discrepancies.push(
            "Unverified local archive provenance: SHA-256 has not been verified against TRUD API"
                .to_string(),
        );
    }

    // Verify SHA256SUMS and Parquet files integrity
    let parquet_files = [
        "orgs.parquet",
        "orgs_all.parquet",
        "roles.parquet",
        "relationships.parquet",
        "successions.parquet",
        "datapackage.json",
    ];
    let sums_file = if parquet_dir.join("SHA256SUMS").exists() {
        parquet_dir.join("SHA256SUMS")
    } else {
        active_release_path.join("SHA256SUMS")
    };
    let mut sums_matched_count = 0;
    let mut prov_derived_matched_count = 0;

    let recorded_sums: HashMap<String, String> = if sums_file.exists() {
        let content = std::fs::read_to_string(&sums_file).unwrap_or_default();
        content
            .lines()
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
                    if let Some(ref derived) = w_prov.dataset_file_sha256 {
                        if let Some(expected_prov_sha) = derived.get(*filename) {
                            if computed_sha.eq_ignore_ascii_case(expected_prov_sha) {
                                prov_derived_matched_count += 1;
                            } else {
                                discrepancies.push(format!(
                                    "_provenance.json dataset_file_sha256 mismatch for {filename}: Computed {computed_sha} != provenance {expected_prov_sha}"
                                ));
                            }
                        } else {
                            discrepancies.push(format!(
                                "Missing _provenance.json dataset_file_sha256 entry for {filename}"
                            ));
                        }
                    } else {
                        discrepancies.push(
                            "Missing dataset_file_sha256 map in _provenance.json".to_string(),
                        );
                    }
                }
            } else {
                discrepancies.push(format!(
                    "Failed to compute live SHA-256 for Parquet file {filename}"
                ));
            }
        } else {
            discrepancies.push(format!("Parquet file missing: {filename}"));
        }
    }

    if !args.json && !quiet_sub_output {
        println!("  1. File integrity:");
        if matched_release && is_verified_archive {
            if let Some(ref sha) = input_sha256 {
                println!(
                    "     ✓ SHA-256 Match: {:<20} # Verified against TRUD API",
                    sha
                );
            } else {
                println!(
                    "     ✓ Data Provenance match: {:<14} # Verified against TRUD API",
                    input_date
                );
            }
        } else if !is_verified_archive {
            println!("     ✖ Archive SHA-256: Unverified local archive provenance");
        } else {
            println!(
                "     ✖ Data Provenance mismatch: Input {} vs Workspace {}",
                input_date, workspace_date
            );
        }

        let expected = parquet_files.len();
        if sums_matched_count == expected {
            println!("     ✓ SHA256SUMS Verification: {sums_matched_count}/{expected} Parquet files match recorded checksums");
        } else {
            println!("     ✖ SHA256SUMS Verification: {sums_matched_count}/{expected} Parquet files match recorded checksums");
        }

        if prov_derived_matched_count == expected {
            println!("     ✓ Provenance Artifact Chain: {prov_derived_matched_count}/{expected} Parquet files match _provenance.json dataset_file_sha256");
        } else {
            println!("     ✖ Provenance Artifact Chain: {prov_derived_matched_count}/{expected} Parquet files match _provenance.json dataset_file_sha256");
        }
        println!();
    }

    // ------------------------------------------------------------------------
    // SECTION 2: Source Structural Invariants & Record Parity
    // ------------------------------------------------------------------------
    let sample_limit = if args.full { usize::MAX } else { args.sample };
    let raw_xml_invariants = scan_raw_xml_invariants(&xml_path, sample_limit)?;

    let source_invariants_passed =
        check_source_invariants(&raw_xml_invariants.invariants, &mut discrepancies);

    if !args.json && !quiet_sub_output {
        println!("  2. Source Structural Invariants:");
        let inv = &raw_xml_invariants.invariants;
        if inv.duplicate_role_ids == 0 {
            println!("     ✓ Unique Role IDs: 0 reuse");
        } else {
            println!(
                "     ✖ Unique Role IDs: {} duplicate uniqueRoleId instances",
                inv.duplicate_role_ids
            );
        }

        if inv.duplicate_rel_ids == 0 {
            println!("     ✓ Unique Rel IDs: 0 reuse");
        } else {
            println!(
                "     ✖ Unique Rel IDs: {} duplicate uniqueRelId instances",
                inv.duplicate_rel_ids
            );
        }

        if inv.dangling_rel_targets == 0 && inv.dangling_succ_targets == 0 {
            println!("     ✓ Target Resolution: 0 dangling targets in XML");
        } else {
            println!(
                "     ✖ Target Resolution: {} dangling rel and {} dangling succ targets",
                inv.dangling_rel_targets, inv.dangling_succ_targets
            );
        }

        if inv.primary_role_id_mismatches == 0 {
            println!("     ✓ PrimaryRoleId Redundancy Cross-Check: 0 mismatches across relationship targets");
        } else {
            println!(
                "     ✖ PrimaryRoleId Redundancy Cross-Check: {} target primary role mismatches",
                inv.primary_role_id_mismatches
            );
        }

        if inv.unknown_role_concepts == 0 {
            println!("     ✓ CodeSystem Concept Integrity: 0 unknown role concepts");
        } else {
            println!(
                "     ✖ CodeSystem Concept Integrity: {} unknown role concept IDs",
                inv.unknown_role_concepts
            );
        }

        if inv.invalid_date_orders == 0 && inv.orgs_missing_operational_date == 0 {
            println!("     ✓ Date Validity: 0 invalid date orders (End >= Start) and 100% operational date coverage");
        } else {
            println!(
                "     ✖ Date Validity: {} invalid date orders, {} orgs missing operational dates",
                inv.invalid_date_orders, inv.orgs_missing_operational_date
            );
        }

        if inv.invalid_statuses == 0
            && inv.invalid_succ_types == 0
            && inv.inactive_without_operational_end == 0
        {
            println!("     ✓ Status & Succession Invariants: 0 invalid values, 0 inactive orgs without operational end");
        } else {
            println!("     ✖ Status & Succession Invariants: {} invalid status, {} invalid succ type, {} inactive without end", inv.invalid_statuses, inv.invalid_succ_types, inv.inactive_without_operational_end);
        }
        println!();
        println!("  3. Completeness & Record Parity Checks:");
    }

    let orgs_all_parquet = parquet_dir.join("orgs_all.parquet");
    let orgs_parquet = parquet_dir.join("orgs.parquet");
    let roles_parquet = parquet_dir.join("roles.parquet");
    let rels_parquet = parquet_dir.join("relationships.parquet");
    let succs_parquet = parquet_dir.join("successions.parquet");

    let total_orgs_res = count_records_in_parquet(&orgs_all_parquet);
    let active_orgs_res = count_records_in_parquet(&orgs_parquet);
    let roles_res = count_records_in_parquet(&roles_parquet);
    let rels_res = count_records_in_parquet(&rels_parquet);
    let succs_res = count_records_in_parquet(&succs_parquet);

    let entities_match = total_orgs_res.as_ref().ok() == Some(&raw_xml_invariants.total_orgs);
    if !entities_match {
        let actual_str = total_orgs_res
            .as_ref()
            .map(|c| c.to_string())
            .unwrap_or_else(|e| format!("Error reading orgs_all.parquet: {:#}", e));
        discrepancies.push(format!(
            "Entity Parity Error: XML orgs count ({}) != orgs_all.parquet ({})",
            raw_xml_invariants.total_orgs, actual_str
        ));
    }

    let roles_match = roles_res.as_ref().ok() == Some(&raw_xml_invariants.roles_count);
    if !roles_match {
        let actual_str = roles_res
            .as_ref()
            .map(|c| c.to_string())
            .unwrap_or_else(|e| format!("Error reading roles.parquet: {:#}", e));
        discrepancies.push(format!(
            "Role Parity Error: XML roles ({}) != roles.parquet ({})",
            raw_xml_invariants.roles_count, actual_str
        ));
    }

    let rels_match = rels_res.as_ref().ok() == Some(&raw_xml_invariants.rels_count);
    if !rels_match {
        let actual_str = rels_res
            .as_ref()
            .map(|c| c.to_string())
            .unwrap_or_else(|e| format!("Error reading relationships.parquet: {:#}", e));
        discrepancies.push(format!(
            "Relationship Parity Error: XML relationships ({}) != relationships.parquet ({})",
            raw_xml_invariants.rels_count, actual_str
        ));
    }

    let succs_match = succs_res.as_ref().ok() == Some(&raw_xml_invariants.succs_distinct_count);
    if !succs_match {
        let actual_str = succs_res
            .as_ref()
            .map(|c| c.to_string())
            .unwrap_or_else(|e| format!("Error reading successions.parquet: {:#}", e));
        discrepancies.push(format!(
            "Successions Parity Error: XML distinct successions ({}) != successions.parquet ({})",
            raw_xml_invariants.succs_distinct_count, actual_str
        ));
    }

    let active_orgs_match = active_orgs_res.as_ref().ok() == Some(&raw_xml_invariants.active_orgs);
    if !active_orgs_match {
        let actual_str = active_orgs_res
            .as_ref()
            .map(|c| c.to_string())
            .unwrap_or_else(|e| format!("Error reading orgs.parquet: {:#}", e));
        discrepancies.push(format!(
            "Active Orgs Parity Error: XML active orgs ({}) != orgs.parquet ({})",
            raw_xml_invariants.active_orgs, actual_str
        ));
    }

    if !args.json && !quiet_sub_output {
        if entities_match {
            println!(
                "     ✓ Entities: {:<27} # XML orgs count matches orgs_all.parquet",
                raw_xml_invariants.total_orgs
            );
        } else {
            let actual_str = total_orgs_res
                .as_ref()
                .map(|c| c.to_string())
                .unwrap_or_else(|_| "missing/unreadable".to_string());
            println!(
                "     ✖ Entities mismatch: XML {} vs Parquet {}",
                raw_xml_invariants.total_orgs, actual_str
            );
        }

        if roles_match {
            println!(
                "     ✓ Roles: {:<30} # XML roles count matches roles.parquet",
                raw_xml_invariants.roles_count
            );
        } else {
            let actual_str = roles_res
                .as_ref()
                .map(|c| c.to_string())
                .unwrap_or_else(|_| "missing/unreadable".to_string());
            println!(
                "     ✖ Roles mismatch: XML {} vs Parquet {}",
                raw_xml_invariants.roles_count, actual_str
            );
        }

        if rels_match {
            println!(
                "     ✓ Relationships: {:<22} # XML rels count matches relationships.parquet",
                raw_xml_invariants.rels_count
            );
        } else {
            let actual_str = rels_res
                .as_ref()
                .map(|c| c.to_string())
                .unwrap_or_else(|_| "missing/unreadable".to_string());
            println!(
                "     ✖ Relationships mismatch: XML {} vs Parquet {}",
                raw_xml_invariants.rels_count, actual_str
            );
        }

        if succs_match {
            println!(
                "     ✓ Successors: {:<25} # XML successions count matches successions.parquet",
                raw_xml_invariants.succs_distinct_count
            );
        } else {
            let actual_str = succs_res
                .as_ref()
                .map(|c| c.to_string())
                .unwrap_or_else(|_| "missing/unreadable".to_string());
            println!(
                "     ✖ Successors mismatch: XML {} vs Parquet {}",
                raw_xml_invariants.succs_distinct_count, actual_str
            );
        }

        if active_orgs_match {
            println!(
                "     ✓ Active: {:<29} # Active orgs in orgs.parquet",
                raw_xml_invariants.active_orgs
            );
        } else {
            let actual_str = active_orgs_res
                .as_ref()
                .map(|c| c.to_string())
                .unwrap_or_else(|_| "missing/unreadable".to_string());
            println!(
                "     ✖ Active mismatch: XML {} vs Parquet {}",
                raw_xml_invariants.active_orgs, actual_str
            );
        }
        println!();
        println!("  4. Schema & Referential Integrity Constraints:");
    }

    // ------------------------------------------------------------------------
    // SECTION 4: Schema & Referential Integrity Constraints
    // ------------------------------------------------------------------------
    let (duplicate_codes, orphan_roles, orphan_rels, orphan_succs) = audit_referential_integrity(
        &orgs_all_parquet,
        &roles_parquet,
        &rels_parquet,
        &succs_parquet,
        &mut discrepancies,
    )?;

    let (inactive_in_orgs, role_list_mismatches) =
        audit_table_invariants(&orgs_parquet, &orgs_all_parquet, &mut discrepancies)?;

    // Unaccounted files check: any file in release directory not in SHA256SUMS
    let mut unaccounted_files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&parquet_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name == "SHA256SUMS"
                        || name == crate::provenance::PROVENANCE_FILENAME
                        || name == "provenance.json"
                        || name.starts_with('.')
                    {
                        continue;
                    }
                    if !recorded_sums.contains_key(name) {
                        unaccounted_files.push(name.to_string());
                    }
                }
            }
        }
    }
    unaccounted_files.sort();
    let unaccounted_count = unaccounted_files.len();
    if unaccounted_count > 0 {
        let file_list = unaccounted_files.join(", ");
        discrepancies.push(format!(
            "Unaccounted files found in release directory: {} not in SHA256SUMS ({})",
            unaccounted_count, file_list
        ));
    }

    if !args.json && !quiet_sub_output {
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

        if inactive_in_orgs == 0 && role_list_mismatches == 0 {
            println!("     ✓ Table Invariants: 100% active in orgs.parquet, 100% role_codes/role_names aligned");
        } else {
            println!(
                "     ✖ Table Invariants: {} inactive in orgs.parquet, {} role mismatches",
                inactive_in_orgs, role_list_mismatches
            );
        }

        if unaccounted_count == 0 {
            println!("     ✓ Unaccounted files: 0");
        } else {
            let file_list = unaccounted_files.join(", ");
            println!(
                "     ✖ Unaccounted files: {:<18} # {} not in SHA256SUMS",
                unaccounted_count, file_list
            );
        }
        println!();
        println!("  5. Field & Derived Column Parity:");
    }

    // ------------------------------------------------------------------------
    // SECTION 5: Field & Derived Column Parity
    // ------------------------------------------------------------------------
    let (sample_parity_passed, derived_parity_passed) = audit_sample_and_derived_parity(
        &orgs_all_parquet,
        &raw_xml_invariants.sample_orgs,
        &raw_xml_invariants.succession_edges,
        &parquet_dir,
        &input_date,
        &mut discrepancies,
    )?;

    let sample_len = raw_xml_invariants.sample_orgs.len();

    if !args.json && !quiet_sub_output {
        if sample_parity_passed {
            println!("     ✓ Verbatim Field Parity: {:<16} # {} XML orgs matched across 17 verbatim fields", format!("{}/{}", sample_len, sample_len), sample_len);
        } else {
            println!("     ✖ Verbatim Field Parity failed");
        }

        if derived_parity_passed {
            println!("     ✓ Derived Column Parity: {:<16} # Closures, hierarchies, role codes & role names verified", format!("{}/{}", sample_len, sample_len));
        } else {
            println!("     ✖ Derived Column Parity failed");
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
        total_orgs_parquet: total_orgs_res.unwrap_or(0),
        active_orgs_xml: raw_xml_invariants.active_orgs,
        active_orgs_parquet: active_orgs_res.unwrap_or(0),
        roles_xml: raw_xml_invariants.roles_count,
        roles_parquet: roles_res.unwrap_or(0),
        rels_xml: raw_xml_invariants.rels_count,
        rels_parquet: rels_res.unwrap_or(0),
        succs_xml: raw_xml_invariants.succs_distinct_count,
        succs_parquet: succs_res.unwrap_or(0),
        sampled_records: raw_xml_invariants.sample_orgs.len(),
        source_invariants_passed,
        sample_parity_passed,
        derived_parity_passed,
        discrepancies: discrepancies.clone(),
    };

    if args.json && !quiet_sub_output {
        println!("{}", serde_json::to_string_pretty(&report)?);
    }

    Ok(discrepancies)
}

fn check_source_invariants(
    inv: &SourceStructuralInvariants,
    discrepancies: &mut Vec<String>,
) -> bool {
    let initial_len = discrepancies.len();
    if inv.duplicate_role_ids > 0 {
        discrepancies.push(format!(
            "Source Invariant Violation: Found {} duplicate uniqueRoleId instances",
            inv.duplicate_role_ids
        ));
    }
    if inv.duplicate_rel_ids > 0 {
        discrepancies.push(format!(
            "Source Invariant Violation: Found {} duplicate uniqueRelId instances",
            inv.duplicate_rel_ids
        ));
    }
    if inv.dangling_rel_targets > 0 {
        discrepancies.push(format!(
            "Source Invariant Violation: Found {} dangling relationship target references",
            inv.dangling_rel_targets
        ));
    }
    if inv.dangling_succ_targets > 0 {
        discrepancies.push(format!(
            "Source Invariant Violation: Found {} dangling successor target references",
            inv.dangling_succ_targets
        ));
    }
    if inv.primary_role_id_mismatches > 0 {
        discrepancies.push(format!(
            "Source Invariant Violation: Found {} relationship target PrimaryRoleId mismatches",
            inv.primary_role_id_mismatches
        ));
    }
    if inv.unknown_role_concepts > 0 {
        discrepancies.push(format!(
            "Source Invariant Violation: Found {} role IDs not defined in CodeSystem concepts",
            inv.unknown_role_concepts
        ));
    }
    if inv.invalid_date_orders > 0 {
        discrepancies.push(format!(
            "Source Invariant Violation: Found {} date ranges with End < Start",
            inv.invalid_date_orders
        ));
    }
    if inv.orgs_missing_operational_date > 0 {
        discrepancies.push(format!(
            "Source Invariant Violation: Found {} organisations missing an Operational date block",
            inv.orgs_missing_operational_date
        ));
    }
    if inv.invalid_statuses > 0 {
        discrepancies.push(format!(
            "Source Invariant Violation: Found {} invalid status values (not Active/Inactive)",
            inv.invalid_statuses
        ));
    }
    if inv.invalid_succ_types > 0 {
        discrepancies.push(format!("Source Invariant Violation: Found {} invalid succession types (not Predecessor/Successor)", inv.invalid_succ_types));
    }
    if inv.inactive_without_operational_end > 0 {
        discrepancies.push(format!("Source Invariant Violation: Found {} Inactive organisations with no operational end date", inv.inactive_without_operational_end));
    }
    discrepancies.len() == initial_len
}

fn scan_raw_xml_invariants(xml_path: &Path, max_samples: usize) -> Result<RawXmlInvariants> {
    let file = File::open(xml_path)?;
    let buf_reader = BufReader::with_capacity(256 * 1024, file);
    let mut reader = Reader::from_reader(buf_reader);
    reader.trim_text(true);

    let mut inv = RawXmlInvariants {
        total_orgs: 0,
        active_orgs: 0,
        roles_count: 0,
        rels_count: 0,
        succs_raw_count: 0,
        succs_distinct_count: 0,
        invariants: SourceStructuralInvariants::default(),
        succession_edges: Vec::new(),
        sample_orgs: HashMap::new(),
    };

    let mut seen_role_ids = HashSet::new();
    let mut seen_rel_ids = HashSet::new();
    let mut unique_succ_ids = HashSet::new();
    let mut all_org_codes = HashSet::new();
    let mut code_system_concepts = HashSet::new();
    let mut role_concept_ids = Vec::new();
    let mut org_primary_role_ids: HashMap<String, String> = HashMap::new();
    let mut rel_primary_role_targets: Vec<(String, String)> = Vec::new();
    let mut rel_target_codes: Vec<String> = Vec::new();
    let mut succ_target_codes: Vec<String> = Vec::new();

    let mut buf = Vec::new();
    let mut org_depth = 0;
    let mut in_name = false;
    let mut in_target = false;
    let mut in_role = false;
    let mut in_rel = false;
    let mut in_succ = false;
    let mut in_date = false;
    let mut in_org_date = false;
    let mut in_addr_line = false;
    let mut in_town = false;
    let mut in_county = false;
    let mut in_postcode = false;
    let mut in_country = false;
    let mut in_uprn = false;

    let mut current_record = XmlSampleOrgRecord::default();
    let mut current_has_operational_date = false;
    let mut current_operational_start: Option<String> = None;
    let mut current_operational_end: Option<String> = None;
    let mut current_date_type: Option<String> = None;
    let mut current_target_code = String::new();
    let mut current_target_primary_role = String::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let name = e.local_name();
                match name.as_ref() {
                    b"concept" | b"Concept" => {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref().eq_ignore_ascii_case(b"id") {
                                if let Ok(val) = attr.decode_and_unescape_value(&reader) {
                                    code_system_concepts.insert(val.into_owned());
                                }
                            }
                        }
                    }
                    b"Organisation" => {
                        org_depth += 1;
                        if org_depth == 1 && !in_target {
                            current_record = XmlSampleOrgRecord::default();
                            current_has_operational_date = false;
                            current_operational_start = None;
                            current_operational_end = None;
                            current_date_type = None;
                            in_org_date = false;
                            in_role = false;
                            in_rel = false;
                            in_succ = false;
                            in_name = false;
                        }
                    }
                    b"Target" => {
                        in_target = true;
                        current_target_code.clear();
                        current_target_primary_role.clear();
                    }
                    b"Name" if org_depth == 1 && !in_target => in_name = true,
                    b"AddrLn1" | b"AddrLn2" | b"AddrLn3" | b"AddrLn4"
                        if org_depth == 1 && !in_target =>
                    {
                        in_addr_line = true
                    }
                    b"Town" if org_depth == 1 && !in_target => in_town = true,
                    b"County" if org_depth == 1 && !in_target => in_county = true,
                    b"PostCode" | b"Postcode" if org_depth == 1 && !in_target => in_postcode = true,
                    b"Country" if org_depth == 1 && !in_target => in_country = true,
                    b"UPRN" | b"Uprn" if org_depth == 1 && !in_target => in_uprn = true,
                    b"OrgId" => {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"extension" {
                                if let Ok(val) = attr.decode_and_unescape_value(&reader) {
                                    let code = val.into_owned();
                                    if org_depth == 1 && !in_target {
                                        current_record.ods_code = code.clone();
                                        all_org_codes.insert(code);
                                    } else if in_target {
                                        current_target_code = code;
                                    }
                                }
                            }
                        }
                    }
                    b"OrgRecordClass" if org_depth == 1 && !in_target => {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref().eq_ignore_ascii_case(b"value") {
                                if let Ok(val) = attr.decode_and_unescape_value(&reader) {
                                    let et = match val.as_ref() {
                                        "RC1" => "org".to_string(),
                                        "RC2" => "site".to_string(),
                                        other => other.to_string(),
                                    };
                                    current_record.entity_type = et;
                                }
                            }
                        }
                    }
                    b"Status"
                        if org_depth == 1 && !in_role && !in_rel && !in_succ && !in_target =>
                    {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"value" {
                                if let Ok(val) = attr.decode_and_unescape_value(&reader) {
                                    let status = val.into_owned();
                                    if status != "Active" && status != "Inactive" {
                                        inv.invariants.invalid_statuses += 1;
                                    }
                                    current_record.status = status;
                                }
                            }
                        }
                    }
                    b"LastChangeDate" if org_depth == 1 && !in_target => {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref().eq_ignore_ascii_case(b"value") {
                                if let Ok(val) = attr.decode_and_unescape_value(&reader) {
                                    current_record.last_changed = Some(val.into_owned());
                                }
                            }
                        }
                    }
                    b"Date" => {
                        in_date = true;
                        current_date_type = None;
                        if org_depth == 1 && !in_role && !in_rel && !in_succ && !in_target {
                            in_org_date = true;
                        }
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref().eq_ignore_ascii_case(b"type") {
                                if let Ok(val) = attr.decode_and_unescape_value(&reader) {
                                    let dt = val.into_owned();
                                    if dt.eq_ignore_ascii_case("operational") {
                                        current_date_type = Some("operational".to_string());
                                        if in_org_date {
                                            current_has_operational_date = true;
                                        }
                                    } else if dt.eq_ignore_ascii_case("legal") {
                                        current_date_type = Some("legal".to_string());
                                    }
                                }
                            }
                        }
                    }
                    b"Type" => {
                        if in_date {
                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref().eq_ignore_ascii_case(b"value") {
                                    if let Ok(val) = attr.decode_and_unescape_value(&reader) {
                                        let dt = val.into_owned();
                                        if dt.eq_ignore_ascii_case("operational") {
                                            current_date_type = Some("operational".to_string());
                                            if in_org_date {
                                                current_has_operational_date = true;
                                            }
                                        } else if dt.eq_ignore_ascii_case("legal") {
                                            current_date_type = Some("legal".to_string());
                                        }
                                    }
                                }
                            }
                        }
                    }
                    b"Start" => {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref().eq_ignore_ascii_case(b"value") {
                                if let Ok(val) = attr.decode_and_unescape_value(&reader) {
                                    let val_str = val.into_owned();
                                    if in_org_date {
                                        if current_date_type.as_deref() == Some("operational") {
                                            current_operational_start = Some(val_str.clone());
                                            if current_record.operational_start.is_none() {
                                                current_record.operational_start = Some(val_str);
                                            }
                                        } else if current_date_type.as_deref() == Some("legal") {
                                            if current_record.legal_start.is_none() {
                                                current_record.legal_start = Some(val_str);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    b"End" => {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref().eq_ignore_ascii_case(b"value") {
                                if let Ok(val) = attr.decode_and_unescape_value(&reader) {
                                    let val_str = val.into_owned();
                                    if in_org_date {
                                        if current_date_type.as_deref() == Some("operational") {
                                            if let Some(ref start_val) = current_operational_start {
                                                if val_str < *start_val {
                                                    inv.invariants.invalid_date_orders += 1;
                                                }
                                            }
                                            current_operational_end = Some(val_str.clone());
                                            if current_record.operational_end.is_none() {
                                                current_record.operational_end = Some(val_str);
                                            }
                                        } else if current_date_type.as_deref() == Some("legal") {
                                            if let Some(ref start_val) = current_record.legal_start
                                            {
                                                if val_str < *start_val {
                                                    inv.invariants.invalid_date_orders += 1;
                                                }
                                            }
                                            if current_record.legal_end.is_none() {
                                                current_record.legal_end = Some(val_str);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    b"Role" if org_depth == 1 && !in_target => {
                        in_role = true;
                        inv.roles_count += 1;
                        let mut role_id = String::new();
                        let mut unique_role_id = String::new();
                        let mut is_primary = false;
                        let mut role_status = "Active".to_string();

                        for attr in e.attributes().flatten() {
                            let k = attr.key.as_ref();
                            if k.eq_ignore_ascii_case(b"id") {
                                if let Ok(val) = attr.decode_and_unescape_value(&reader) {
                                    role_id = val.into_owned();
                                }
                            } else if k.eq_ignore_ascii_case(b"uniqueroleid") {
                                if let Ok(val) = attr.decode_and_unescape_value(&reader) {
                                    unique_role_id = val.into_owned();
                                }
                            } else if k.eq_ignore_ascii_case(b"primaryrole") {
                                if let Ok(val) = attr.decode_and_unescape_value(&reader) {
                                    is_primary = val.eq_ignore_ascii_case("true");
                                }
                            } else if k.eq_ignore_ascii_case(b"status") {
                                if let Ok(val) = attr.decode_and_unescape_value(&reader) {
                                    role_status = val.into_owned();
                                }
                            }
                        }

                        if !role_id.is_empty() {
                            role_concept_ids.push(role_id.clone());
                            if !role_status.eq_ignore_ascii_case("inactive")
                                || current_record.status.eq_ignore_ascii_case("inactive")
                            {
                                if !current_record.role_codes.contains(&role_id) {
                                    current_record.role_codes.push(role_id.clone());
                                }
                            }
                        }

                        if !unique_role_id.is_empty() {
                            if !seen_role_ids.insert(unique_role_id.clone()) {
                                inv.invariants.duplicate_role_ids += 1;
                            }
                        }
                        if is_primary && !role_id.is_empty() {
                            current_record.primary_role_code = role_id.clone();
                            org_primary_role_ids.insert(current_record.ods_code.clone(), role_id);
                        }
                    }
                    b"PrimaryRole" | b"PrimaryRoleId" if org_depth == 1 => {
                        if in_target {
                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref().eq_ignore_ascii_case(b"id") {
                                    if let Ok(val) = attr.decode_and_unescape_value(&reader) {
                                        current_target_primary_role = val.into_owned();
                                    }
                                }
                            }
                        }
                    }
                    b"Relationship" | b"Rel" if org_depth == 1 => {
                        inv.rels_count += 1;
                        in_rel = true;
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref().eq_ignore_ascii_case(b"uniquerelid") {
                                if let Ok(val) = attr.decode_and_unescape_value(&reader) {
                                    let rel_id = val.into_owned();
                                    if !seen_rel_ids.insert(rel_id) {
                                        inv.invariants.duplicate_rel_ids += 1;
                                    }
                                }
                            }
                        }
                    }
                    b"Successor" | b"Succ" if org_depth == 1 => {
                        inv.succs_raw_count += 1;
                        in_succ = true;
                        let mut succ_type = "Successor".to_string();
                        for attr in e.attributes().flatten() {
                            let k = attr.key.as_ref();
                            if k.eq_ignore_ascii_case(b"uniquesuccid") {
                                if let Ok(val) = attr.decode_and_unescape_value(&reader) {
                                    unique_succ_ids.insert(val.into_owned());
                                }
                            } else if k.eq_ignore_ascii_case(b"type") {
                                if let Ok(val) = attr.decode_and_unescape_value(&reader) {
                                    let t = val.into_owned();
                                    if t != "Predecessor" && t != "Successor" {
                                        inv.invariants.invalid_succ_types += 1;
                                    }
                                    succ_type = t;
                                }
                            }
                        }
                        let _ = succ_type;
                    }
                    b"Contact" if org_depth == 1 && !in_target => {
                        let mut ctype = String::new();
                        let mut cval = String::new();
                        for attr in e.attributes().flatten() {
                            let k = attr.key.as_ref();
                            if k.eq_ignore_ascii_case(b"type") {
                                if let Ok(val) = attr.decode_and_unescape_value(&reader) {
                                    ctype = val.into_owned();
                                }
                            } else if k.eq_ignore_ascii_case(b"value") {
                                if let Ok(val) = attr.decode_and_unescape_value(&reader) {
                                    cval = val.into_owned();
                                }
                            }
                        }
                        if ctype == "tel" && current_record.telephone.is_none() {
                            current_record.telephone = Some(cval);
                        } else if ctype == "http" && current_record.website.is_none() {
                            current_record.website = Some(cval);
                        }
                    }
                    _ => {}
                }
            }
            Event::Text(ref e) => {
                if let Ok(text) = e.unescape() {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        if in_name && current_record.name.is_empty() && !in_target {
                            current_record.name = trimmed.to_string();
                        } else if in_addr_line && !in_target {
                            current_record.address_parts.push(trimmed.to_string());
                        } else if in_town && !in_target {
                            current_record.town = Some(trimmed.to_string());
                        } else if in_county && !in_target {
                            current_record.county = Some(trimmed.to_string());
                        } else if in_postcode && !in_target {
                            current_record.postcode = Some(trimmed.to_string());
                        } else if in_country && !in_target {
                            current_record.country = Some(trimmed.to_string());
                        } else if in_uprn && !in_target {
                            current_record.uprn = Some(trimmed.to_string());
                        }
                    }
                }
            }
            Event::End(ref e) => {
                let name = e.local_name();
                match name.as_ref() {
                    b"Name" => in_name = false,
                    b"AddrLn1" | b"AddrLn2" | b"AddrLn3" | b"AddrLn4" => in_addr_line = false,
                    b"Town" => in_town = false,
                    b"County" => in_county = false,
                    b"PostCode" | b"Postcode" => in_postcode = false,
                    b"Country" => in_country = false,
                    b"UPRN" | b"Uprn" => in_uprn = false,
                    b"Date" => {
                        in_date = false;
                        in_org_date = false;
                        current_date_type = None;
                    }
                    b"Role" => in_role = false,
                    b"Relationship" | b"Rel" => {
                        in_rel = false;
                        in_target = false;
                    }
                    b"Successor" | b"Succ" => {
                        in_succ = false;
                        in_target = false;
                    }
                    b"Target" => {
                        if in_succ {
                            if !current_target_code.is_empty() {
                                succ_target_codes.push(current_target_code.clone());
                            }
                        } else if in_rel {
                            if !current_target_code.is_empty() {
                                rel_target_codes.push(current_target_code.clone());
                                if !current_target_primary_role.is_empty() {
                                    rel_primary_role_targets.push((
                                        current_target_code.clone(),
                                        current_target_primary_role.clone(),
                                    ));
                                }
                            }
                        }
                        in_target = false;
                    }
                    b"Organisation" => {
                        if org_depth == 1 && !in_target {
                            inv.total_orgs += 1;
                            let status = if current_record.status.is_empty() {
                                "Active".to_string()
                            } else {
                                current_record.status.clone()
                            };
                            if status.eq_ignore_ascii_case("active") {
                                inv.active_orgs += 1;
                            } else if current_operational_end.is_none() {
                                inv.invariants.inactive_without_operational_end += 1;
                            }

                            if !current_has_operational_date {
                                inv.invariants.orgs_missing_operational_date += 1;
                            }

                            if inv.sample_orgs.len() < max_samples
                                && !current_record.ods_code.is_empty()
                            {
                                current_record.role_codes.sort_unstable();
                                current_record.role_codes.dedup();
                                inv.sample_orgs.insert(
                                    current_record.ods_code.clone(),
                                    current_record.clone(),
                                );
                            }
                        }
                        if org_depth > 0 {
                            org_depth -= 1;
                        }
                    }
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    inv.succs_distinct_count = unique_succ_ids.len();

    // Check invariants that require the full set of org codes
    for t_code in &rel_target_codes {
        if !all_org_codes.contains(t_code) {
            inv.invariants.dangling_rel_targets += 1;
        }
    }

    for t_code in &succ_target_codes {
        if !all_org_codes.contains(t_code) {
            inv.invariants.dangling_succ_targets += 1;
        }
    }

    for (t_code, expected_primary_role) in &rel_primary_role_targets {
        if let Some(actual_primary_role) = org_primary_role_ids.get(t_code) {
            if actual_primary_role != expected_primary_role {
                inv.invariants.primary_role_id_mismatches += 1;
            }
        }
    }

    if !code_system_concepts.is_empty() {
        for role_id in &role_concept_ids {
            if !code_system_concepts.contains(role_id) {
                inv.invariants.unknown_role_concepts += 1;
            }
        }
    }

    // Now populate full succession graph edges from ndjson parse if available or from fast pass
    if let Ok((_, _, parsed_orgs)) = crate::commands::ndjson::parse_single_pass(xml_path) {
        for org in parsed_orgs.values() {
            for succ in &org.successors {
                let is_pred = succ.succ_type.eq_ignore_ascii_case("predecessor");
                if is_pred {
                    inv.succession_edges
                        .push((succ.target.ods_code.clone(), org.ods_code.clone()));
                } else {
                    inv.succession_edges
                        .push((org.ods_code.clone(), succ.target.ods_code.clone()));
                }
            }
        }
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
    let mut valid_codes = HashSet::new();
    let mut duplicate_codes = 0;

    let file_res = File::open(orgs_all_parquet).and_then(|f| {
        ParquetRecordBatchReaderBuilder::try_new(f)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    });
    match file_res {
        Ok(builder) => match builder.build() {
            Ok(reader) => {
                for batch in reader {
                    let batch = batch?;
                    let ods_code_arr = batch
                        .column(batch.schema().index_of("ods_code")?)
                        .as_any()
                        .downcast_ref::<StringArray>()
                        .context("ods_code StringArray")?;
                    for i in 0..batch.num_rows() {
                        let code = ods_code_arr.value(i);
                        if !valid_codes.insert(code.to_string()) {
                            duplicate_codes += 1;
                        }
                    }
                }
            }
            Err(e) => {
                discrepancies.push(format!(
                    "Referential Integrity Error: Failed to read orgs_all.parquet: {:#}",
                    e
                ));
            }
        },
        Err(e) => {
            discrepancies.push(format!(
                "Referential Integrity Error: Failed to open orgs_all.parquet: {:#}",
                e
            ));
        }
    }

    if duplicate_codes > 0 {
        discrepancies.push(format!(
            "PK Uniqueness Error: Found {} duplicate ods_code entries in orgs_all.parquet",
            duplicate_codes
        ));
    }

    let mut orphan_roles = 0;
    let mut uncurated_roles = 0;
    // 2. Foreign Key Check: roles.parquet ods_code -> orgs_all.parquet & role_name check
    if roles_parquet.exists() {
        if let Ok(rfile) = File::open(roles_parquet) {
            if let Ok(rbuilder) = ParquetRecordBatchReaderBuilder::try_new(rfile) {
                if let Ok(rreader) = rbuilder.build() {
                    for batch in rreader.flatten() {
                        let schema = batch.schema();
                        let code_idx = schema.index_of("ods_code").ok();
                        let role_code_idx = schema.index_of("role_code").ok();
                        let role_name_idx = schema.index_of("role_name").ok();

                        let code_arr = code_idx.and_then(|idx| {
                            batch.column(idx).as_any().downcast_ref::<StringArray>()
                        });
                        let role_code_arr = role_code_idx.and_then(|idx| {
                            batch.column(idx).as_any().downcast_ref::<StringArray>()
                        });
                        let role_name_arr = role_name_idx.and_then(|idx| {
                            batch.column(idx).as_any().downcast_ref::<StringArray>()
                        });

                        for i in 0..batch.num_rows() {
                            if let Some(arr) = code_arr {
                                let code = arr.value(i);
                                if !code.is_empty() && !valid_codes.contains(code) {
                                    orphan_roles += 1;
                                }
                            }
                            if let (Some(rc_arr), Some(rn_arr)) = (role_code_arr, role_name_arr) {
                                let rc = rc_arr.value(i);
                                let rn = rn_arr.value(i);
                                match roles::role_names().role_name(rc) {
                                    Ok(expected_rn) => {
                                        if rn != expected_rn {
                                            uncurated_roles += 1;
                                        }
                                    }
                                    Err(_) => {
                                        uncurated_roles += 1;
                                    }
                                }
                            }
                        }
                    }
                } else {
                    discrepancies.push(
                        "Referential Integrity Error: Failed to read roles.parquet".to_string(),
                    );
                }
            } else {
                discrepancies
                    .push("Referential Integrity Error: Failed to parse roles.parquet".to_string());
            }
        }

        if orphan_roles > 0 {
            discrepancies.push(format!(
                "Referential Integrity Violation: Found {} orphan ods_code links in roles.parquet",
                orphan_roles
            ));
        }
        if uncurated_roles > 0 {
            discrepancies.push(format!(
                "Referential Integrity Violation: Found {} invalid/uncurated role_name values in roles.parquet",
                uncurated_roles
            ));
        }
    }

    let mut orphan_rels = 0;
    // 2. Foreign Key Check: relationships.parquet target_code / source_code -> orgs_all.parquet
    if rels_parquet.exists() {
        if let Ok(rfile) = File::open(rels_parquet) {
            if let Ok(rbuilder) = ParquetRecordBatchReaderBuilder::try_new(rfile) {
                if let Ok(rreader) = rbuilder.build() {
                    for batch in rreader.flatten() {
                        let schema = batch.schema();
                        let target_idx = schema.index_of("target_code").ok();
                        let source_idx = schema.index_of("source_code").ok();

                        if let (Some(t_idx), Some(s_idx)) = (target_idx, source_idx) {
                            if let (Some(target_arr), Some(source_arr)) = (
                                batch.column(t_idx).as_any().downcast_ref::<StringArray>(),
                                batch.column(s_idx).as_any().downcast_ref::<StringArray>(),
                            ) {
                                for i in 0..batch.num_rows() {
                                    let target_code = target_arr.value(i);
                                    let source_code = source_arr.value(i);
                                    if (!target_code.is_empty()
                                        && !valid_codes.contains(target_code))
                                        || (!source_code.is_empty()
                                            && !valid_codes.contains(source_code))
                                    {
                                        orphan_rels += 1;
                                    }
                                }
                            }
                        }
                    }
                } else {
                    discrepancies.push(
                        "Referential Integrity Error: Failed to read relationships.parquet"
                            .to_string(),
                    );
                }
            } else {
                discrepancies.push(
                    "Referential Integrity Error: Failed to parse relationships.parquet"
                        .to_string(),
                );
            }
        }

        if orphan_rels > 0 {
            discrepancies.push(format!(
                "Referential Integrity Violation: Found {} orphan links in relationships.parquet",
                orphan_rels
            ));
        }
    }

    let mut orphan_succs = 0;
    // 3. Foreign Key Check: successions.parquet predecessor_code / successor_code -> orgs_all.parquet
    if succs_parquet.exists() {
        if let Ok(sfile) = File::open(succs_parquet) {
            if let Ok(sbuilder) = ParquetRecordBatchReaderBuilder::try_new(sfile) {
                if let Ok(sreader) = sbuilder.build() {
                    for batch in sreader.flatten() {
                        let schema = batch.schema();
                        let pred_idx = schema.index_of("predecessor_code").ok();
                        let succ_idx = schema.index_of("successor_code").ok();

                        if let (Some(p_idx), Some(s_idx)) = (pred_idx, succ_idx) {
                            if let (Some(pred_arr), Some(succ_arr)) = (
                                batch.column(p_idx).as_any().downcast_ref::<StringArray>(),
                                batch.column(s_idx).as_any().downcast_ref::<StringArray>(),
                            ) {
                                for i in 0..batch.num_rows() {
                                    let pred_code = pred_arr.value(i);
                                    let succ_code = succ_arr.value(i);
                                    if (!pred_code.is_empty() && !valid_codes.contains(pred_code))
                                        || (!succ_code.is_empty()
                                            && !valid_codes.contains(succ_code))
                                    {
                                        orphan_succs += 1;
                                    }
                                }
                            }
                        }
                    }
                } else {
                    discrepancies.push(
                        "Referential Integrity Error: Failed to read successions.parquet"
                            .to_string(),
                    );
                }
            } else {
                discrepancies.push(
                    "Referential Integrity Error: Failed to parse successions.parquet".to_string(),
                );
            }
        }

        if orphan_succs > 0 {
            discrepancies.push(format!(
                "Referential Integrity Violation: Found {} orphan links in successions.parquet",
                orphan_succs
            ));
        }
    }

    Ok((duplicate_codes, orphan_roles, orphan_rels, orphan_succs))
}

fn compute_full_closures(
    edges: &[(String, String)],
) -> (HashMap<String, Vec<String>>, HashMap<String, Vec<String>>) {
    let mut fwd_adj: HashMap<String, HashSet<String>> = HashMap::new();
    let mut rev_adj: HashMap<String, HashSet<String>> = HashMap::new();

    for (pred, succ) in edges {
        fwd_adj
            .entry(pred.clone())
            .or_default()
            .insert(succ.clone());
        rev_adj
            .entry(succ.clone())
            .or_default()
            .insert(pred.clone());
    }

    let mut successor_closures: HashMap<String, Vec<String>> = HashMap::new();
    let mut predecessor_closures: HashMap<String, Vec<String>> = HashMap::new();

    for (pred, _) in edges {
        if !successor_closures.contains_key(pred) {
            let mut visited = HashSet::new();
            let mut queue = VecDeque::new();
            queue.push_back(pred.clone());
            visited.insert(pred.clone());

            while let Some(curr) = queue.pop_front() {
                if let Some(nexts) = fwd_adj.get(&curr) {
                    for next in nexts {
                        if visited.insert(next.clone()) {
                            queue.push_back(next.clone());
                        }
                    }
                }
            }
            visited.remove(pred);
            let mut list: Vec<String> = visited.into_iter().collect();
            list.sort();
            successor_closures.insert(pred.clone(), list);
        }
    }

    for (_, succ) in edges {
        if !predecessor_closures.contains_key(succ) {
            let mut visited = HashSet::new();
            let mut queue = VecDeque::new();
            queue.push_back(succ.clone());
            visited.insert(succ.clone());

            while let Some(curr) = queue.pop_front() {
                if let Some(nexts) = rev_adj.get(&curr) {
                    for next in nexts {
                        if visited.insert(next.clone()) {
                            queue.push_back(next.clone());
                        }
                    }
                }
            }
            visited.remove(succ);
            let mut list: Vec<String> = visited.into_iter().collect();
            list.sort();
            predecessor_closures.insert(succ.clone(), list);
        }
    }

    (successor_closures, predecessor_closures)
}

fn audit_sample_and_derived_parity(
    orgs_all_parquet: &Path,
    sample_orgs: &HashMap<String, XmlSampleOrgRecord>,
    succession_edges: &[(String, String)],
    _parquet_dir: &Path,
    _trud_release_date: &str,
    discrepancies: &mut Vec<String>,
) -> Result<(bool, bool)> {
    if !orgs_all_parquet.exists() || sample_orgs.is_empty() {
        return Ok((true, true));
    }

    let initial_count = discrepancies.len();

    // 1. Compute 100% full graph closures
    let (succ_closures, pred_closures) = compute_full_closures(succession_edges);

    let file = File::open(orgs_all_parquet)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let reader = builder.build()?;

    let mut verbatim_failures = 0;
    let mut derived_failures = 0;

    for batch in reader {
        let batch = batch?;
        let schema = batch.schema();
        let num_rows = batch.num_rows();

        let ods_code_arr = batch
            .column(schema.index_of("ods_code")?)
            .as_any()
            .downcast_ref::<StringArray>()
            .context("ods_code StringArray")?;
        let name_arr = batch
            .column(schema.index_of("name")?)
            .as_any()
            .downcast_ref::<StringArray>()
            .context("name StringArray")?;
        let status_arr = batch
            .column(schema.index_of("status")?)
            .as_any()
            .downcast_ref::<StringArray>()
            .context("status StringArray")?;
        let entity_type_arr = schema
            .index_of("entity_type")
            .ok()
            .and_then(|i| batch.column(i).as_any().downcast_ref::<StringArray>());
        let primary_role_arr = batch
            .column(schema.index_of("primary_role_code")?)
            .as_any()
            .downcast_ref::<StringArray>()
            .context("primary_role_code StringArray")?;

        let role_codes_arr = batch
            .column(schema.index_of("role_codes")?)
            .as_any()
            .downcast_ref::<ListArray>()
            .context("role_codes ListArray")?;
        let role_names_arr = batch
            .column(schema.index_of("role_names")?)
            .as_any()
            .downcast_ref::<ListArray>()
            .context("role_names ListArray")?;
        let succ_codes_arr = batch
            .column(schema.index_of("successor_codes")?)
            .as_any()
            .downcast_ref::<ListArray>()
            .context("successor_codes ListArray")?;
        let pred_codes_arr = batch
            .column(schema.index_of("predecessor_codes")?)
            .as_any()
            .downcast_ref::<ListArray>()
            .context("predecessor_codes ListArray")?;

        let address_arr = schema
            .index_of("address")
            .ok()
            .and_then(|i| batch.column(i).as_any().downcast_ref::<StringArray>());
        let town_arr = schema
            .index_of("town")
            .ok()
            .and_then(|i| batch.column(i).as_any().downcast_ref::<StringArray>());
        let county_arr = schema
            .index_of("county")
            .ok()
            .and_then(|i| batch.column(i).as_any().downcast_ref::<StringArray>());
        let postcode_arr = schema
            .index_of("postcode")
            .ok()
            .and_then(|i| batch.column(i).as_any().downcast_ref::<StringArray>());
        let country_arr = schema
            .index_of("country")
            .ok()
            .and_then(|i| batch.column(i).as_any().downcast_ref::<StringArray>());
        let uprn_arr = schema
            .index_of("uprn")
            .ok()
            .and_then(|i| batch.column(i).as_any().downcast_ref::<StringArray>());
        let telephone_arr = schema
            .index_of("telephone")
            .ok()
            .and_then(|i| batch.column(i).as_any().downcast_ref::<StringArray>());
        let website_arr = schema
            .index_of("website")
            .ok()
            .and_then(|i| batch.column(i).as_any().downcast_ref::<StringArray>());

        for i in 0..num_rows {
            let code = ods_code_arr.value(i);
            if let Some(xml_sample) = sample_orgs.get(code) {
                // 1. Verbatim Source Field Parity
                let p_name = name_arr.value(i);
                if !p_name.eq_ignore_ascii_case(&xml_sample.name) {
                    verbatim_failures += 1;
                    discrepancies.push(format!(
                        "Verbatim Field Mismatch ({code}): Name XML='{}' vs Parquet='{}'",
                        xml_sample.name, p_name
                    ));
                }

                let p_status = status_arr.value(i);
                if !p_status.eq_ignore_ascii_case(&xml_sample.status) {
                    verbatim_failures += 1;
                    discrepancies.push(format!(
                        "Verbatim Field Mismatch ({code}): Status XML='{}' vs Parquet='{}'",
                        xml_sample.status, p_status
                    ));
                }

                if let Some(et_arr) = entity_type_arr {
                    let p_et = et_arr.value(i);
                    if !xml_sample.entity_type.is_empty()
                        && !p_et.eq_ignore_ascii_case(&xml_sample.entity_type)
                    {
                        verbatim_failures += 1;
                        discrepancies.push(format!(
                            "Verbatim Field Mismatch ({code}): EntityType XML='{}' vs Parquet='{}'",
                            xml_sample.entity_type, p_et
                        ));
                    }
                }

                let p_prole = primary_role_arr.value(i);
                if !xml_sample.primary_role_code.is_empty()
                    && !p_prole.eq_ignore_ascii_case(&xml_sample.primary_role_code)
                {
                    verbatim_failures += 1;
                    discrepancies.push(format!(
                        "Verbatim Field Mismatch ({code}): PrimaryRole XML='{}' vs Parquet='{}'",
                        xml_sample.primary_role_code, p_prole
                    ));
                }

                if let (Some(t_arr), Some(ref xml_town)) = (town_arr, &xml_sample.town) {
                    let p_town = t_arr.value(i);
                    if !p_town.eq_ignore_ascii_case(xml_town) {
                        verbatim_failures += 1;
                        discrepancies.push(format!(
                            "Verbatim Field Mismatch ({code}): Town XML='{}' vs Parquet='{}'",
                            xml_town, p_town
                        ));
                    }
                }

                if let (Some(c_arr), Some(ref xml_county)) = (county_arr, &xml_sample.county) {
                    let p_county = c_arr.value(i);
                    if !p_county.eq_ignore_ascii_case(xml_county) {
                        verbatim_failures += 1;
                        discrepancies.push(format!(
                            "Verbatim Field Mismatch ({code}): County XML='{}' vs Parquet='{}'",
                            xml_county, p_county
                        ));
                    }
                }

                if let (Some(co_arr), Some(ref xml_country)) = (country_arr, &xml_sample.country) {
                    let p_country = co_arr.value(i);
                    if !p_country.eq_ignore_ascii_case(xml_country) {
                        verbatim_failures += 1;
                        discrepancies.push(format!(
                            "Verbatim Field Mismatch ({code}): Country XML='{}' vs Parquet='{}'",
                            xml_country, p_country
                        ));
                    }
                }

                if let (Some(u_arr), Some(ref xml_uprn)) = (uprn_arr, &xml_sample.uprn) {
                    let p_uprn = u_arr.value(i);
                    if !p_uprn.eq_ignore_ascii_case(xml_uprn) {
                        verbatim_failures += 1;
                        discrepancies.push(format!(
                            "Verbatim Field Mismatch ({code}): UPRN XML='{}' vs Parquet='{}'",
                            xml_uprn, p_uprn
                        ));
                    }
                }

                if let (Some(tel_arr), Some(ref xml_tel)) = (telephone_arr, &xml_sample.telephone) {
                    let p_tel = tel_arr.value(i);
                    if !p_tel.eq_ignore_ascii_case(xml_tel) {
                        verbatim_failures += 1;
                        discrepancies.push(format!(
                            "Verbatim Field Mismatch ({code}): Telephone XML='{}' vs Parquet='{}'",
                            xml_tel, p_tel
                        ));
                    }
                }

                if let (Some(web_arr), Some(ref xml_web)) = (website_arr, &xml_sample.website) {
                    let p_web = web_arr.value(i);
                    if !p_web.eq_ignore_ascii_case(xml_web) {
                        verbatim_failures += 1;
                        discrepancies.push(format!(
                            "Verbatim Field Mismatch ({code}): Website XML='{}' vs Parquet='{}'",
                            xml_web, p_web
                        ));
                    }
                }

                // 2. Normalization & Derived Checks
                if let (Some(post_arr), Some(ref xml_post)) = (postcode_arr, &xml_sample.postcode) {
                    let p_post = post_arr.value(i);
                    let normalized_xml = xml_post.split_whitespace().collect::<Vec<_>>().join(" ");
                    if !p_post.eq_ignore_ascii_case(&normalized_xml) {
                        derived_failures += 1;
                        discrepancies.push(format!(
                            "Normalization Mismatch ({code}): Postcode XML='{}' vs Parquet='{}'",
                            normalized_xml, p_post
                        ));
                    }
                }

                if let Some(addr_arr) = address_arr {
                    if !xml_sample.address_parts.is_empty() {
                        let p_addr = addr_arr.value(i);
                        let joined_xml = xml_sample.address_parts.join(", ");
                        if !p_addr.contains(&xml_sample.address_parts[0]) {
                            derived_failures += 1;
                            discrepancies.push(format!("Address Normalization Mismatch ({code}): XML address parts '{:?}' not in Parquet '{}'", joined_xml, p_addr));
                        }
                    }
                }

                // Role codes list check
                let r_values = role_codes_arr.value(i);
                let r_str_arr = r_values.as_any().downcast_ref::<StringArray>().unwrap();
                let mut parquet_roles: Vec<String> = (0..r_str_arr.len())
                    .map(|j| r_str_arr.value(j).to_string())
                    .collect();
                parquet_roles.sort();
                parquet_roles.dedup();
                let mut xml_roles = xml_sample.role_codes.clone();
                xml_roles.sort();
                xml_roles.dedup();
                if parquet_roles != xml_roles {
                    derived_failures += 1;
                    discrepancies.push(format!(
                        "Role Codes Set Union Mismatch ({code}): XML={:?} vs Parquet={:?}",
                        xml_roles, parquet_roles
                    ));
                }

                // Role names positional alignment & curation check
                let rn_values = role_names_arr.value(i);
                let rn_str_arr = rn_values.as_any().downcast_ref::<StringArray>().unwrap();
                if r_str_arr.len() != rn_str_arr.len() {
                    derived_failures += 1;
                    discrepancies.push(format!(
                        "Role Names Positional Alignment Mismatch ({code}): role_codes len={} != role_names len={}",
                        r_str_arr.len(),
                        rn_str_arr.len()
                    ));
                } else {
                    for j in 0..r_str_arr.len() {
                        let c = r_str_arr.value(j);
                        let n = rn_str_arr.value(j);
                        match roles::role_names().role_name(c) {
                            Ok(expected_n) => {
                                if n != expected_n {
                                    derived_failures += 1;
                                    discrepancies.push(format!(
                                        "Role Name Curation Mismatch ({code}): code '{c}' has name '{n}' in Parquet != expected '{expected_n}'"
                                    ));
                                }
                            }
                            Err(e) => {
                                derived_failures += 1;
                                discrepancies.push(format!(
                                    "Uncurated Role Code in Parquet ({code}): code '{c}' error: {e}"
                                ));
                            }
                        }
                    }
                }

                // Transitive Closures Check
                let empty_vec = Vec::new();
                let expected_succs = succ_closures.get(code).unwrap_or(&empty_vec);
                let succ_values = succ_codes_arr.value(i);
                let succ_str_arr = succ_values.as_any().downcast_ref::<StringArray>().unwrap();
                let parquet_succs: Vec<String> = (0..succ_str_arr.len())
                    .map(|j| succ_str_arr.value(j).to_string())
                    .collect();
                if &parquet_succs != expected_succs {
                    derived_failures += 1;
                    discrepancies.push(format!(
                        "Successor Closure Mismatch ({code}): Expected={:?} vs Parquet={:?}",
                        expected_succs, parquet_succs
                    ));
                }

                let expected_preds = pred_closures.get(code).unwrap_or(&empty_vec);
                let pred_values = pred_codes_arr.value(i);
                let pred_str_arr = pred_values.as_any().downcast_ref::<StringArray>().unwrap();
                let parquet_preds: Vec<String> = (0..pred_str_arr.len())
                    .map(|j| pred_str_arr.value(j).to_string())
                    .collect();
                if &parquet_preds != expected_preds {
                    derived_failures += 1;
                    discrepancies.push(format!(
                        "Predecessor Closure Mismatch ({code}): Expected={:?} vs Parquet={:?}",
                        expected_preds, parquet_preds
                    ));
                }
            }
        }
    }

    let verbatim_ok = verbatim_failures == 0;
    let derived_ok = derived_failures == 0;

    Ok((
        verbatim_ok,
        derived_ok && (discrepancies.len() == initial_count),
    ))
}

fn audit_table_invariants(
    orgs_parquet: &Path,
    orgs_all_parquet: &Path,
    discrepancies: &mut Vec<String>,
) -> Result<(usize, usize)> {
    let mut inactive_in_orgs = 0;
    let mut role_list_mismatches = 0;

    if orgs_parquet.exists() {
        let ofile = File::open(orgs_parquet)?;
        let obuilder = ParquetRecordBatchReaderBuilder::try_new(ofile)?;
        let oreader = obuilder.build()?;
        for batch in oreader {
            let batch = batch?;
            let schema = batch.schema();
            let status_idx = schema.index_of("status")?;
            let role_codes_idx = schema.index_of("role_codes")?;
            let role_names_idx = schema.index_of("role_names")?;

            let status_arr = batch
                .column(status_idx)
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            let role_codes_arr = batch
                .column(role_codes_idx)
                .as_any()
                .downcast_ref::<ListArray>()
                .unwrap();
            let role_names_arr = batch
                .column(role_names_idx)
                .as_any()
                .downcast_ref::<ListArray>()
                .unwrap();

            for i in 0..batch.num_rows() {
                let st = status_arr.value(i);
                if !st.eq_ignore_ascii_case("active") {
                    inactive_in_orgs += 1;
                }

                let rc_val = role_codes_arr.value(i);
                let rn_val = role_names_arr.value(i);
                let rc_str = rc_val.as_any().downcast_ref::<StringArray>().unwrap();
                let rn_str = rn_val.as_any().downcast_ref::<StringArray>().unwrap();

                if rc_str.len() != rn_str.len() {
                    role_list_mismatches += 1;
                } else {
                    for j in 0..rc_str.len() {
                        let c = rc_str.value(j);
                        let n = rn_str.value(j);
                        match roles::role_names().role_name(c) {
                            Ok(expected) => {
                                if n != expected {
                                    role_list_mismatches += 1;
                                }
                            }
                            Err(_) => {
                                role_list_mismatches += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    if orgs_all_parquet.exists() {
        let ofile = File::open(orgs_all_parquet)?;
        let obuilder = ParquetRecordBatchReaderBuilder::try_new(ofile)?;
        let oreader = obuilder.build()?;
        for batch in oreader {
            let batch = batch?;
            let schema = batch.schema();
            let role_codes_idx = schema.index_of("role_codes")?;
            let role_names_idx = schema.index_of("role_names")?;

            let role_codes_arr = batch
                .column(role_codes_idx)
                .as_any()
                .downcast_ref::<ListArray>()
                .unwrap();
            let role_names_arr = batch
                .column(role_names_idx)
                .as_any()
                .downcast_ref::<ListArray>()
                .unwrap();

            for i in 0..batch.num_rows() {
                let rc_val = role_codes_arr.value(i);
                let rn_val = role_names_arr.value(i);
                let rc_str = rc_val.as_any().downcast_ref::<StringArray>().unwrap();
                let rn_str = rn_val.as_any().downcast_ref::<StringArray>().unwrap();

                if rc_str.len() != rn_str.len() {
                    role_list_mismatches += 1;
                } else {
                    for j in 0..rc_str.len() {
                        let c = rc_str.value(j);
                        let n = rn_str.value(j);
                        match roles::role_names().role_name(c) {
                            Ok(expected) => {
                                if n != expected {
                                    role_list_mismatches += 1;
                                }
                            }
                            Err(_) => {
                                role_list_mismatches += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    if inactive_in_orgs > 0 {
        discrepancies.push(format!(
            "Table Invariant Violation: Found {} inactive records in orgs.parquet (must be 100% active)",
            inactive_in_orgs
        ));
    }

    if role_list_mismatches > 0 {
        discrepancies.push(format!(
            "Table Invariant Violation: Found {} role_codes vs role_names length/curation mismatches across organisation records",
            role_list_mismatches
        ));
    }

    Ok((inactive_in_orgs, role_list_mismatches))
}

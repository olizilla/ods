use anyhow::{Context, Result};
use arrow::array::Array;
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::ods_xml::{convert_parsed_orgs, find_xml_file, parse_single_pass, OdsRecord};
use crate::provenance::OdsProvenance;

#[derive(Parser, Debug)]
pub struct Args {
    /// Baseline Parquet, TRUD XML file, or release date tag (defaults to previous release in workspace)
    pub old: Option<PathBuf>,

    /// Target Parquet, TRUD XML file, or release date tag (defaults to current release in workspace)
    pub new: Option<PathBuf>,

    /// Output format: summary (default TUI), json, patch, csv, markdown
    #[arg(long, short, default_value = "summary")]
    pub format: String,

    /// Filter diffs to specific primary role (e.g. "General Practice", "NHS Trust")
    #[arg(long, short)]
    pub role: Option<String>,

    /// Include detailed categorized change log in summary mode
    #[arg(long, short)]
    pub verbose: bool,

    /// Only include entities active in either release
    #[arg(long)]
    pub only_active: bool,

    /// Write diff output to file instead of stdout
    #[arg(long, short)]
    pub output: Option<PathBuf>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct DiffSummary {
    pub baseline_release: Option<String>,
    pub target_release: Option<String>,
    pub baseline_count: usize,
    pub target_count: usize,
    pub added_count: usize,
    pub removed_count: usize,
    pub modified_count: usize,
    pub unchanged_count: usize,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ValueChange {
    pub old: Option<String>,
    pub new: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EntityDiff {
    pub ods_code: String,
    pub name: String,
    pub role: String,
    pub change_type: String, // "added", "removed", "modified"
    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    pub changes: HashMap<String, ValueChange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record: Option<OdsRecord>,
}

pub fn run(args: Args) -> Result<()> {
    let (old_path, new_path) = match (&args.old, &args.new) {
        (Some(o), Some(n)) => (o.clone(), n.clone()),
        _ => {
            let ws = crate::workspace::Workspace::open(None)?;
            let releases = ws.releases()?;
            if releases.len() < 2 {
                anyhow::bail!("At least 2 release snapshots are required in releases/ to auto-diff. Found {}.", releases.len());
            }
            let new_p = args.new.clone().unwrap_or_else(|| releases[0].path.clone());
            let old_p = args.old.clone().unwrap_or_else(|| releases[1].path.clone());
            (old_p, new_p)
        }
    };

    eprintln!("Loading baseline dataset from {}...", old_path.display());
    let (old_prov, old_records) = load_dataset(&old_path)?;

    eprintln!("Loading target dataset from {}...", new_path.display());
    let (new_prov, new_records) = load_dataset(&new_path)?;

    eprintln!(
        "Loaded {} baseline records and {} target records.",
        old_records.len(),
        new_records.len()
    );

    let (added, removed, modified, stats) = compute_diff(&old_records, &new_records, &args)?;

    let mut writer: Box<dyn Write> = match &args.output {
        Some(path) => {
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)?;
                }
            }
            Box::new(BufWriter::new(File::create(path)?))
        }
        None => Box::new(std::io::stdout()),
    };

    match args.format.to_lowercase().as_str() {
        "summary" => render_summary_tui(&mut writer, &args, old_prov.as_ref(), new_prov.as_ref(), old_records.len(), new_records.len(), &added, &removed, &modified, &stats)?,
        "json" | "ndjson" => render_json_ndjson(&mut writer, &added, &removed, &modified)?,
        "patch" => render_unified_patch(&mut writer, &added, &removed, &modified)?,
        "csv" => render_csv(&mut writer, &added, &removed, &modified)?,
        "markdown" | "md" => render_markdown(&mut writer, &args, old_prov.as_ref(), new_prov.as_ref(), old_records.len(), new_records.len(), &added, &removed, &modified, &stats)?,
        other => anyhow::bail!("Unsupported format '{}'. Supported formats: summary, json, patch, csv, markdown", other),
    }

    writer.flush()?;
    Ok(())
}

fn load_dataset(path: &Path) -> Result<(Option<OdsProvenance>, HashMap<String, OdsRecord>)> {
    if !path.exists() {
        anyhow::bail!("Path does not exist: {}", path.display());
    }

    if path.is_file() {
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        if ext.eq_ignore_ascii_case("parquet") {
            return load_parquet(path);
        } else if ext.eq_ignore_ascii_case("ndjson") || ext.eq_ignore_ascii_case("json") {
            return load_ndjson(path);
        } else if ext.eq_ignore_ascii_case("zip") {
            return load_zip(path);
        } else if ext.eq_ignore_ascii_case("xml") {
            return load_xml(path);
        }
    }

    // Directory lookup: try finding Parquet, NDJSON or XML inside directory
    if path.is_dir() {
        let orgs_parquet = if path.join("orgs.parquet").exists() {
            Some(path.join("orgs.parquet"))
        } else if path.join("orgs_all.parquet").exists() {
            Some(path.join("orgs_all.parquet"))
        } else {
            None
        };
        if let Some(pfile) = orgs_parquet {
            return load_parquet(&pfile);
        }

        return load_xml(path);
    }

    anyhow::bail!("Could not determine file format for {}", path.display())
}

fn load_parquet(path: &Path) -> Result<(Option<OdsProvenance>, HashMap<String, OdsRecord>)> {
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    let file = File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let reader = builder.build()?;

    let mut records = HashMap::new();
    for batch in reader {
        let batch = batch?;
        let schema = batch.schema();
        let num_rows = batch.num_rows();

        let ods_code_arr = batch.column(schema.index_of("ods_code")?)
            .as_any().downcast_ref::<arrow::array::StringArray>().context("ods_code StringArray")?;
        let name_arr = batch.column(schema.index_of("name")?)
            .as_any().downcast_ref::<arrow::array::StringArray>().context("name StringArray")?;
        let status_arr = batch.column(schema.index_of("status")?)
            .as_any().downcast_ref::<arrow::array::StringArray>().context("status StringArray")?;
        // orgs.parquet carries the primary role *code*; diff reports the
        // curated name so changelogs stay readable.
        let role_arr = batch.column(schema.index_of("primary_role_code")?)
            .as_any().downcast_ref::<arrow::array::StringArray>().context("primary_role_code StringArray")?;
        let record_class_idx = schema.index_of("record_class").ok();
        let town_idx = schema.index_of("town").ok();
        let postcode_idx = schema.index_of("postcode").ok();

        for i in 0..num_rows {
            let ods_code = ods_code_arr.value(i).to_string();
            let name = name_arr.value(i).to_string();
            let status = status_arr.value(i).to_string();
            let role_code = role_arr.value(i);
            let role = crate::roles::role_names()
                .name(role_code)
                .unwrap_or(role_code)
                .to_string();
            let record_class = record_class_idx.map(|idx| batch.column(idx).as_any().downcast_ref::<arrow::array::StringArray>().unwrap().value(i).to_string()).unwrap_or_else(|| "org".to_string());

            let town = town_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<arrow::array::StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i).to_string()) } else { None }
            });
            let postcode = postcode_idx.and_then(|idx| {
                let arr = batch.column(idx).as_any().downcast_ref::<arrow::array::StringArray>()?;
                if arr.is_valid(i) { Some(arr.value(i).to_string()) } else { None }
            });
            let geo_loc = if town.is_some() || postcode.is_some() {
                Some(crate::ods_xml::Location {
                    town,
                    postcode,
                    ..Default::default()
                })
            } else {
                None
            };

            let record = OdsRecord {
                ods_code: ods_code.clone(),
                name,
                status,
                role,
                record_class,
                geo_loc,
                ..Default::default()
            };
            records.insert(ods_code, record);
        }
    }

    Ok((None, records))
}

fn load_ndjson(path: &Path) -> Result<(Option<OdsProvenance>, HashMap<String, OdsRecord>)> {
    let file = File::open(path)?;
    let reader = BufReader::with_capacity(128 * 1024, file);
    let mut records = HashMap::new();
    let mut provenance = None;

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if provenance.is_none() {
            if let Ok(prov) = serde_json::from_str::<OdsProvenance>(&line) {
                if prov.type_tag == crate::provenance::PROVENANCE_TYPE_TAG {
                    provenance = Some(prov);
                    continue;
                }
            }
        }
        let record: OdsRecord = serde_json::from_str(&line)
            .with_context(|| format!("failed to parse NDJSON line in {}", path.display()))?;
        records.insert(record.ods_code.clone(), record);
    }

    Ok((provenance, records))
}

fn load_xml(path: &Path) -> Result<(Option<OdsProvenance>, HashMap<String, OdsRecord>)> {
    let xml_path = find_xml_file(path)?;
    let (prov, _, parsed) = parse_single_pass(&xml_path)?;
    let map: HashMap<String, OdsRecord> = convert_parsed_orgs(parsed).into_iter().collect();
    Ok((Some(prov), map))
}

fn load_zip(path: &Path) -> Result<(Option<OdsProvenance>, HashMap<String, OdsRecord>)> {
    let file = File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)?;

    // Check if zip directly contains an XML file
    for i in 0..archive.len() {
        let file = archive.by_index(i)?;
        if file.name().ends_with(".xml") {
            let temp_dir = std::env::temp_dir().join("ods_zip_cache");
            std::fs::create_dir_all(&temp_dir)?;
            let temp_xml_path = temp_dir.join(Path::new(file.name()).file_name().unwrap_or_default());
            let mut outfile = File::create(&temp_xml_path)?;
            let mut reader = BufReader::new(file);
            std::io::copy(&mut reader, &mut outfile)?;
            return load_xml(&temp_xml_path);
        }
    }

    // Check if zip contains fullfile.zip
    if let Ok(mut fullfile) = archive.by_name("fullfile.zip") {
        let mut buffer = Vec::new();
        std::io::copy(&mut fullfile, &mut buffer)?;
        let mut inner_archive = zip::ZipArchive::new(std::io::Cursor::new(buffer))?;
        for i in 0..inner_archive.len() {
            let file = inner_archive.by_index(i)?;
            if file.name().ends_with(".xml") {
                let temp_dir = std::env::temp_dir().join("ods_zip_cache");
                std::fs::create_dir_all(&temp_dir)?;
                let temp_xml_path = temp_dir.join(Path::new(file.name()).file_name().unwrap_or_default());
                let mut outfile = File::create(&temp_xml_path)?;
                let mut reader = BufReader::new(file);
                std::io::copy(&mut reader, &mut outfile)?;
                return load_xml(&temp_xml_path);
            }
        }
    }

    anyhow::bail!("No XML file found inside zip archive {}", path.display())
}

#[derive(Default, Debug)]
pub struct DiffStats {
    pub status_changes: usize,
    pub name_changes: usize,
    pub geo_changes: usize,
    pub other_changes: usize,
}

fn compute_diff(
    old_records: &HashMap<String, OdsRecord>,
    new_records: &HashMap<String, OdsRecord>,
    args: &Args,
) -> Result<(
    Vec<OdsRecord>,
    Vec<OdsRecord>,
    Vec<EntityDiff>,
    DiffStats,
)> {
    let mut stats = DiffStats::default();
    let old_keys: HashSet<_> = old_records.keys().cloned().collect();
    let new_keys: HashSet<_> = new_records.keys().cloned().collect();

    let added_keys: Vec<_> = new_keys.difference(&old_keys).cloned().collect();
    let removed_keys: Vec<_> = old_keys.difference(&new_keys).cloned().collect();
    let common_keys: Vec<_> = old_keys.intersection(&new_keys).cloned().collect();

    let mut added = Vec::new();
    for key in added_keys {
        let rec = &new_records[&key];
        if matches_filter(rec, args) {
            added.push(rec.clone());
        }
    }

    let mut removed = Vec::new();
    for key in removed_keys {
        let rec = &old_records[&key];
        if matches_filter(rec, args) {
            removed.push(rec.clone());
        }
    }

    let mut modified = Vec::new();

    for key in common_keys {
        let old_rec = &old_records[&key];
        let new_rec = &new_records[&key];

        if !matches_filter(old_rec, args) && !matches_filter(new_rec, args) {
            continue;
        }

        let mut changes = HashMap::new();
        let mut has_status = false;
        let mut has_name = false;
        let mut has_geo = false;
        let mut has_other = false;

        if old_rec.status != new_rec.status {
            changes.insert(
                "status".to_string(),
                ValueChange {
                    old: Some(old_rec.status.clone()),
                    new: Some(new_rec.status.clone()),
                },
            );
            has_status = true;
        }

        if old_rec.name != new_rec.name {
            changes.insert(
                "name".to_string(),
                ValueChange {
                    old: Some(old_rec.name.clone()),
                    new: Some(new_rec.name.clone()),
                },
            );
            has_name = true;
        }

        if old_rec.role != new_rec.role {
            changes.insert(
                "role".to_string(),
                ValueChange {
                    old: Some(old_rec.role.clone()),
                    new: Some(new_rec.role.clone()),
                },
            );
            has_other = true;
        }

        let old_postcode = old_rec.geo_loc.as_ref().and_then(|g| g.postcode.as_deref());
        let new_postcode = new_rec.geo_loc.as_ref().and_then(|g| g.postcode.as_deref());
        if old_postcode != new_postcode || old_rec.geo_loc != new_rec.geo_loc {
            changes.insert(
                "postcode".to_string(),
                ValueChange {
                    old: old_postcode.map(String::from),
                    new: new_postcode.map(String::from),
                },
            );
            has_geo = true;
        }

        if old_rec.contacts != new_rec.contacts {
            changes.insert(
                "contacts".to_string(),
                ValueChange {
                    old: Some(format!("{:?}", old_rec.contacts)),
                    new: Some(format!("{:?}", new_rec.contacts)),
                },
            );
            has_other = true;
        }

        if !changes.is_empty() {
            if has_status { stats.status_changes += 1; }
            if has_name { stats.name_changes += 1; }
            if has_geo { stats.geo_changes += 1; }
            if has_other && !has_status && !has_name && !has_geo {
                stats.other_changes += 1;
            }

            modified.push(EntityDiff {
                ods_code: new_rec.ods_code.clone(),
                name: new_rec.name.clone(),
                role: new_rec.role.clone(),
                change_type: "modified".to_string(),
                changes,
                record: None,
            });
        }
    }

    added.sort_by(|a, b| a.ods_code.cmp(&b.ods_code));
    removed.sort_by(|a, b| a.ods_code.cmp(&b.ods_code));
    modified.sort_by(|a, b| a.ods_code.cmp(&b.ods_code));

    Ok((added, removed, modified, stats))
}

fn matches_filter(rec: &OdsRecord, args: &Args) -> bool {
    if args.only_active && !rec.status.eq_ignore_ascii_case("active") {
        return false;
    }
    if let Some(ref role_filter) = args.role {
        let rf = role_filter.to_lowercase();
        if !rec.role.to_lowercase().contains(&rf) {
            return false;
        }
    }
    true
}

fn render_summary_tui(
    w: &mut dyn Write,
    args: &Args,
    old_prov: Option<&OdsProvenance>,
    new_prov: Option<&OdsProvenance>,
    total_old: usize,
    total_new: usize,
    added: &[OdsRecord],
    removed: &[OdsRecord],
    modified: &[EntityDiff],
    stats: &DiffStats,
) -> Result<()> {
    let bold = |s: &str| format!("\x1b[1m{}\x1b[0m", s);
    let green = |s: &str| format!("\x1b[1;32m{}\x1b[0m", s);
    let red = |s: &str| format!("\x1b[1;31m{}\x1b[0m", s);
    let yellow = |s: &str| format!("\x1b[1;33m{}\x1b[0m", s);

    let old_label = old_prov
        .and_then(|p| p.trud_release_date.as_deref())
        .map(|d| format!("{} ({} recs)", d, total_old))
        .unwrap_or_else(|| format!("{} records", total_old));

    let new_label = new_prov
        .and_then(|p| p.trud_release_date.as_deref())
        .map(|d| format!("{} ({} recs)", d, total_new))
        .unwrap_or_else(|| format!("{} records", total_new));

    writeln!(w, "\n{}", bold("================ ODS TRUD RELEASE DIFF REPORT ================"))?;
    writeln!(w, "  Baseline: {}", old_label)?;
    writeln!(w, "  Target:   {}", new_label)?;

    writeln!(w, "\n  {}", bold("Entity Lifecycle:"))?;
    writeln!(
        w,
        "    {} Added:      {:>6} organisations",
        green("✚"),
        added.len()
    )?;
    writeln!(
        w,
        "    {} Removed:    {:>6} organisations",
        red("✖"),
        removed.len()
    )?;
    let mod_pct = if total_old > 0 {
        (modified.len() as f64 / total_old as f64) * 100.0
    } else {
        0.0
    };
    writeln!(
        w,
        "    {} Modified:   {:>6} organisations ({:.2}%)",
        yellow("✎"),
        modified.len(),
        mod_pct
    )?;

    writeln!(w, "\n  {}", bold("Property Changes:"))?;
    writeln!(w, "    • {:<36} {:>6}", "Status Changes (Active ↔ Inactive):", stats.status_changes)?;
    writeln!(w, "    • {:<36} {:>6}", "Name Modifications:", stats.name_changes)?;
    writeln!(w, "    • {:<36} {:>6}", "Address / Geography Changes:", stats.geo_changes)?;
    if stats.other_changes > 0 {
        writeln!(w, "    • {:<36} {:>6}", "Other Property Changes:", stats.other_changes)?;
    }

    if args.verbose {
        writeln!(w, "\n{}", bold("================ CATEGORIZED ENTITY CHANGES ================"))?;

        if !added.is_empty() {
            writeln!(w, "\n  {} {}", green("✚ Added Organisations:"), added.len())?;
            for item in added.iter().take(20) {
                writeln!(w, "    • {} ({}) [{}]", bold(&item.ods_code), item.name, item.role)?;
            }
            if added.len() > 20 {
                writeln!(w, "    ... and {} more added entities", added.len() - 20)?;
            }
        }

        if !removed.is_empty() {
            writeln!(w, "\n  {} {}", red("✖ Removed / Retired Organisations:"), removed.len())?;
            for item in removed.iter().take(20) {
                writeln!(w, "    • {} ({}) [{}]", bold(&item.ods_code), item.name, item.role)?;
            }
            if removed.len() > 20 {
                writeln!(w, "    ... and {} more removed entities", removed.len() - 20)?;
            }
        }

        if !modified.is_empty() {
            writeln!(w, "\n  {} {}", yellow("✎ Modified Organisations:"), modified.len())?;
            for diff in modified.iter().take(30) {
                let change_strs: Vec<String> = diff
                    .changes
                    .iter()
                    .map(|(k, v)| {
                        format!(
                            "{}: {} ➔ {}",
                            k,
                            v.old.as_deref().unwrap_or("null"),
                            v.new.as_deref().unwrap_or("null")
                        )
                    })
                    .collect();
                writeln!(
                    w,
                    "    • {} ({}): {}",
                    bold(&diff.ods_code),
                    diff.name,
                    change_strs.join(", ")
                )?;
            }
            if modified.len() > 30 {
                writeln!(w, "    ... and {} more modified entities", modified.len() - 30)?;
            }
        }
    }

    Ok(())
}

fn render_json_ndjson(
    w: &mut dyn Write,
    added: &[OdsRecord],
    removed: &[OdsRecord],
    modified: &[EntityDiff],
) -> Result<()> {
    for item in added {
        let diff = EntityDiff {
            ods_code: item.ods_code.clone(),
            name: item.name.clone(),
            role: item.role.clone(),
            change_type: "added".to_string(),
            changes: HashMap::new(),
            record: Some(item.clone()),
        };
        writeln!(w, "{}", serde_json::to_string(&diff)?)?;
    }

    for item in removed {
        let diff = EntityDiff {
            ods_code: item.ods_code.clone(),
            name: item.name.clone(),
            role: item.role.clone(),
            change_type: "removed".to_string(),
            changes: HashMap::new(),
            record: Some(item.clone()),
        };
        writeln!(w, "{}", serde_json::to_string(&diff)?)?;
    }

    for diff in modified {
        writeln!(w, "{}", serde_json::to_string(diff)?)?;
    }

    Ok(())
}

fn render_unified_patch(
    w: &mut dyn Write,
    added: &[OdsRecord],
    removed: &[OdsRecord],
    modified: &[EntityDiff],
) -> Result<()> {
    for item in added {
        writeln!(w, "--- /dev/null")?;
        writeln!(w, "+++ b/{}", item.ods_code)?;
        writeln!(w, "@@ -0,0 +1 @@")?;
        writeln!(w, "+{}", serde_json::to_string(item)?)?;
    }

    for item in removed {
        writeln!(w, "--- a/{}", item.ods_code)?;
        writeln!(w, "+++ /dev/null")?;
        writeln!(w, "@@ -1 +0,0 @@")?;
        writeln!(w, "-{}", serde_json::to_string(item)?)?;
    }

    for diff in modified {
        writeln!(w, "--- a/{}", diff.ods_code)?;
        writeln!(w, "+++ b/{}", diff.ods_code)?;
        writeln!(w, "@@ -1 +1 @@")?;
        for (k, v) in &diff.changes {
            writeln!(w, "-{}: {}", k, v.old.as_deref().unwrap_or("null"))?;
            writeln!(w, "+{}: {}", k, v.new.as_deref().unwrap_or("null"))?;
        }
    }

    Ok(())
}

fn render_csv(
    w: &mut dyn Write,
    added: &[OdsRecord],
    removed: &[OdsRecord],
    modified: &[EntityDiff],
) -> Result<()> {
    writeln!(w, "ods_code,name,role,change_type,field,old_value,new_value")?;

    for item in added {
        writeln!(
            w,
            "\"{}\",\"{}\",\"{}\",\"added\",\"\",,",
            item.ods_code, item.name, item.role
        )?;
    }

    for item in removed {
        writeln!(
            w,
            "\"{}\",\"{}\",\"{}\",\"removed\",\"\",,",
            item.ods_code, item.name, item.role
        )?;
    }

    for diff in modified {
        for (k, v) in &diff.changes {
            writeln!(
                w,
                "\"{}\",\"{}\",\"{}\",\"modified\",\"{}\",\"{}\",\"{}\"",
                diff.ods_code,
                diff.name,
                diff.role,
                k,
                v.old.as_deref().unwrap_or(""),
                v.new.as_deref().unwrap_or("")
            )?;
        }
    }

    Ok(())
}

fn render_markdown(
    w: &mut dyn Write,
    _args: &Args,
    old_prov: Option<&OdsProvenance>,
    new_prov: Option<&OdsProvenance>,
    total_old: usize,
    total_new: usize,
    added: &[OdsRecord],
    removed: &[OdsRecord],
    modified: &[EntityDiff],
    stats: &DiffStats,
) -> Result<()> {
    let mod_pct = if total_old > 0 { (modified.len() as f64 / total_old as f64) * 100.0 } else { 0.0 };

    let old_label = old_prov
        .and_then(|p| p.trud_release_date.as_deref())
        .map(|d| format!("{} ({} recs)", d, total_old))
        .unwrap_or_else(|| format!("{} records", total_old));

    let new_label = new_prov
        .and_then(|p| p.trud_release_date.as_deref())
        .map(|d| format!("{} ({} recs)", d, total_new))
        .unwrap_or_else(|| format!("{} records", total_new));

    writeln!(w, "# ODS TRUD Release Diff Report\n")?;
    writeln!(w, "Baseline: {}  ➔  Target: {}\n", old_label, new_label)?;

    writeln!(w, "### Entity Lifecycle\n")?;
    writeln!(w, "- ✚ Added:      {:>6} organisations", added.len())?;
    writeln!(w, "- ✖ Removed:    {:>6} organisations", removed.len())?;
    writeln!(w, "- ✎ Modified:   {:>6} organisations ({:.2}%)\n", modified.len(), mod_pct)?;

    writeln!(w, "### Property Changes\n")?;
    writeln!(w, "| Metric                               | Count |")?;
    writeln!(w, "| ------------------------------------ | ----- |")?;
    writeln!(w, "| {:<36} | {:>5} |", "Status Changes (Active ↔ Inactive)", stats.status_changes)?;
    writeln!(w, "| {:<36} | {:>5} |", "Name Modifications", stats.name_changes)?;
    writeln!(w, "| {:<36} | {:>5} |", "Address / Geography Changes", stats.geo_changes)?;
    if stats.other_changes > 0 {
        writeln!(w, "| {:<36} | {:>5} |", "Other Property Changes", stats.other_changes)?;
    }
    writeln!(w)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_record(code: &str, name: &str, status: &str, role: &str) -> OdsRecord {
        OdsRecord {
            ods_code: code.to_string(),
            name: name.to_string(),
            status: status.to_string(),
            role: role.to_string(),
            record_class: "org".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn test_compute_diff_added_removed_modified() -> Result<()> {
        let mut old_map = HashMap::new();
        let mut new_map = HashMap::new();

        old_map.insert("A101".to_string(), sample_record("A101", "Old Hospital", "active", "NHS Trust"));
        old_map.insert("A102".to_string(), sample_record("A102", "Retired Practice", "active", "General Practice"));

        new_map.insert("A101".to_string(), sample_record("A101", "New Hospital Name", "active", "NHS Trust"));
        new_map.insert("A103".to_string(), sample_record("A103", "New Practice", "active", "General Practice"));

        let args = Args {
            old: Some(PathBuf::from("old")),
            new: Some(PathBuf::from("new")),
            format: "summary".to_string(),
            role: None,
            verbose: false,
            only_active: false,
            output: None,
        };

        let (added, removed, modified, stats) = compute_diff(&old_map, &new_map, &args)?;

        assert_eq!(added.len(), 1);
        assert_eq!(added[0].ods_code, "A103");

        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].ods_code, "A102");

        assert_eq!(modified.len(), 1);
        assert_eq!(modified[0].ods_code, "A101");
        assert_eq!(stats.name_changes, 1);

        Ok(())
    }

    #[test]
    fn test_compute_diff_role_filter() -> Result<()> {
        let mut old_map = HashMap::new();
        let mut new_map = HashMap::new();

        old_map.insert("A101".to_string(), sample_record("A101", "Hospital", "active", "NHS Trust"));
        new_map.insert("A101".to_string(), sample_record("A101", "Hospital", "inactive", "NHS Trust"));

        old_map.insert("A102".to_string(), sample_record("A102", "GP Practice", "active", "General Practice"));
        new_map.insert("A102".to_string(), sample_record("A102", "GP Practice", "inactive", "General Practice"));

        let args = Args {
            old: Some(PathBuf::from("old")),
            new: Some(PathBuf::from("new")),
            format: "summary".to_string(),
            role: Some("General Practice".to_string()),
            verbose: false,
            only_active: false,
            output: None,
        };

        let (_added, _removed, modified, stats) = compute_diff(&old_map, &new_map, &args)?;

        assert_eq!(modified.len(), 1);
        assert_eq!(modified[0].ods_code, "A102");
        assert_eq!(stats.status_changes, 1);

        Ok(())
    }
}

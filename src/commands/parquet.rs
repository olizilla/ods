use anyhow::{Context, Result};
use arrow::array::{ArrayRef, BooleanBuilder, Date32Builder, ListBuilder, StringBuilder};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use clap::Parser;
use parquet::arrow::arrow_writer::ArrowWriter;
use parquet::file::properties::WriterProperties;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::ods_xml::OdsRecord;
use crate::progress::{render_make_block, MakeBlockParams, MakeTableDone, Progress, ProgressCaps};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

const BATCH_SIZE: usize = 50_000;

#[derive(Parser, Debug, Clone, Default)]
pub struct Args {
    /// TRUD XML file or ZIP archive input path [default: the active release]
    #[arg(long, short)]
    pub input: Option<PathBuf>,

    /// Output Parquet directory path [default: the active release]
    #[arg(long, short)]
    pub output: Option<PathBuf>,

    /// Show errors and the settled report only
    #[arg(long, short = 'q', conflicts_with = "verbose")]
    pub quiet: bool,

    /// Disable interactive live progress animations
    #[arg(long)]
    pub no_progress: bool,

    /// Show how many stubs the merge set aside
    #[arg(long, short = 'v')]
    pub verbose: bool,
}

fn embed_metadata(
    schema: &Schema,
    _prov: Option<&crate::provenance::OdsProvenance>,
) -> Arc<Schema> {
    Arc::new(schema.clone())
}

fn writer_properties(prov: Option<&crate::provenance::OdsProvenance>) -> WriterProperties {
    let meta_kv = if let Some(p) = prov {
        p.to_parquet_declared_metadata()
            .into_iter()
            .map(|(k, v)| parquet::file::metadata::KeyValue {
                key: k,
                value: Some(v),
            })
            .collect()
    } else {
        vec![]
    };

    // Parquet Encoding Rationale:
    // 1. ZSTD at level 3 is explicitly pinned for deterministic cross-build compression and byte stability.
    // 2. Max row group size is set to 64,000. This aligns with our 50,000 BATCH_SIZE and splits large
    //    tables (orgs: 371k, rels: 770k) into multiple row groups to enable HTTP range-request
    //    pruning in DuckDB.
    // 3. Orgs rows are pre-sorted by `status` then `ods_code` before export: active rows share the leading row
    //    groups, and `ods_code` ranges do not overlap within a status. Min/max stats then prune a `status` filter
    //    and an `ods_code` lookup alike, so bloom filters are omitted to avoid inflating file size.
    WriterProperties::builder()
        .set_key_value_metadata(Some(meta_kv))
        .set_compression(parquet::basic::Compression::ZSTD(
            parquet::basic::ZstdLevel::try_new(3).expect("valid zstd level 3"),
        ))
        .set_max_row_group_size(64_000)
        .build()
}

pub fn run(args: Args) -> Result<PathBuf> {
    build(args, false)
}

/// `run` for a command line that ends when it returns: the big collections are left for the
/// operating system to reclaim instead of being freed one allocation at a time (about two
/// seconds of the 2026-08-28 build). Library callers and tests use `run`, which drops them.
pub fn run_before_process_exit(args: Args) -> Result<PathBuf> {
    build(args, true)
}

fn build(args: Args, abandon_memory: bool) -> Result<PathBuf> {
    let (quiet, no_progress, verbose) = (args.quiet, args.no_progress, args.verbose);
    // Warnings wait for the report: a `!` line above a repainting block would be painted over.
    let mut held_warnings: Vec<String> = Vec::new();
    let (input_path, inferred_date) = match args.input {
        Some(p) => (p, None),
        None => {
            let ws = crate::workspace::Workspace::open(None)?;
            let (date, active_dir) = ws.active_release()?;
            let trud_dir = active_dir.join("trud");
            let p = if trud_dir.exists() {
                trud_dir
            } else {
                active_dir
            };
            (p, Some(date))
        }
    };

    let output_path = match args.output {
        Some(p) => p,
        None => {
            let ws = crate::workspace::Workspace::open(None)?;
            let (_, active_dir) = ws.active_release()?;
            active_dir
        }
    };

    let archive_info = crate::archive::resolve_trud_archive(&input_path)?;

    // The first line names what is being read, as `ods info` names its source. The old
    // "<date> (current) → <dir>" line is kept only for the case that warns: building a release
    // other than the one you are standing in.
    let stderr_color = std::io::IsTerminal::is_terminal(&std::io::stderr()) && std::env::var("NO_COLOR").is_err();
    // `--quiet` prints the report and the warnings and nothing else, so the `*` and `✓` lines
    // that say what was read and verified are skipped.
    if !quiet {
        eprintln!(
            "{}",
            crate::workspace::format_source_line(
                &crate::provenance::format_provenance_display_path(&archive_info.archive_path),
                stderr_color
            )
        );
    }
    if let Some(ref date) = inferred_date {
        if crate::workspace::detect_cwd_release().is_some_and(|cwd| &cwd != date) {
            crate::workspace::report_inferred_release_write(date, &output_path);
        }
    }

    let disk_prov = match crate::provenance::OdsProvenance::load_from_dir_with_path(&input_path).error_building_with_path()? {
        Some(p) => Some(p),
        None => crate::provenance::OdsProvenance::load_from_dir_with_path(&archive_info.archive_path).error_building_with_path()?,
    };

    let parent_prov = match disk_prov {
        Some((prov, prov_path)) => {
            if input_path.is_dir() {
                if let Err(e) = prov.validate_baseline() {
                    anyhow::bail!(
                        "Invalid baseline _provenance.json in input '{}': {}. Did you run 'ods trud pull' first?",
                        input_path.display(),
                        e
                    );
                }
            }
            // The release directory's own `_provenance.json` is the expected one and goes
            // unmentioned; a provenance carried in from somewhere else is named.
            if !quiet && !same_file(&prov_path, &output_path.join(crate::provenance::PROVENANCE_FILENAME)) {
                let display_path = crate::provenance::format_provenance_display_path(&prov_path);
                eprintln!("* Provenance: {}", display_path);
            }
            Some(prov)
        }
        None => {
            if input_path.is_dir() {
                anyhow::bail!(
                    "Missing _provenance.json in input directory '{}'. Did you run 'ods trud pull' first?",
                    input_path.display()
                );
            }
            let local_sha256 = crate::provenance::compute_file_sha256(&archive_info.archive_path)?;
            let ws_root = crate::workspace::find_workspace_root_from(&output_path, None).ok().flatten()
                .or_else(|| crate::workspace::find_workspace_root_from(&input_path, None).ok().flatten())
                .or_else(|| std::env::current_dir().ok().and_then(|cwd| crate::workspace::find_workspace_root_from(&cwd, None).ok().flatten()))
                .unwrap_or_else(|| PathBuf::from(crate::workspace::DEFAULT_WORKSPACE_DIR));

            let outcome = crate::commands::fetch::verify_archive::<crate::commands::fetch::UreqTrudFetcher, crate::commands::pull::HttpOciFetcher>(
                &archive_info.release_date,
                &local_sha256,
                &ws_root,
                None,
                false,
                false,
                None,
                None,
            )?;

            match outcome {
                crate::commands::fetch::ArchiveVerificationOutcome::VerifiedPublished { .. } => {
                    if !quiet {
                        eprintln!(
                            "✓ {}  SHA-256 verified by ods release index",
                            input_path.display()
                        );
                    }
                    crate::provenance::OdsProvenance::try_extract_trud_zip_provenance(
                        &archive_info.archive_path,
                    )
                }
                crate::commands::fetch::ArchiveVerificationOutcome::Mismatch { source_name, expected_sha256, actual_sha256 } => {
                    eprintln!(
                        "✖ SHA-256 Checksum Failed!\n  Local SHA-256: {}\n  {} SHA-256: {}",
                        actual_sha256, source_name, expected_sha256
                    );
                    return Err(crate::commands::pull::AlreadyReported.into());
                }
                _ => {
                    held_warnings.push(format!(
                        "! No provenance info found for {}. Source is unverified.",
                        input_path.display()
                    ));
                    crate::provenance::OdsProvenance::try_extract_trud_zip_provenance(
                        &archive_info.archive_path,
                    )
                }
            }
        }
    };

    let progress = Progress::stderr(ProgressCaps::detect(quiet, verbose, no_progress));
    progress.step("unpacking the release…");
    let xml_paths = crate::ods_xml::find_xml_file(&archive_info.archive_path)?;

    // Both totals are known before the work starts: the manifests declare their records and the
    // files' sizes are on disk. They are stated once and never move; the bars carry the change.
    let mut declared: Option<usize> = Some(0);
    let mut xml_bytes = 0u64;
    for path in &xml_paths {
        declared = match (declared, crate::ods_xml::declared_record_count(path)?) {
            (Some(total), Some(n)) => Some(total + n),
            _ => None,
        };
        xml_bytes += std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    }
    let state = MakeState::new(declared, xml_bytes, verbose);
    state.paint(&progress);

    // Everything from here on repaints the block, so a failure clears it before the error prints.
    macro_rules! or_clear {
        ($e:expr) => {
            match $e {
                Ok(v) => v,
                Err(err) => {
                    progress.clear_live();
                    return Err(err.into());
                }
            }
        };
    }

    let release = or_clear!(crate::ods_xml::parse_release_reporting(&xml_paths, &|records, bytes| {
        state.reading(records, bytes);
        state.paint(&progress);
    }));
    or_clear!(check_record_counts(&release));
    state.parsed(release.stubs_superseded);
    let duplicates_dropped = release.duplicates_dropped.clone();
    let stubs_remaining = release.stubs_remaining;
    let mut prov = release.provenance;
    let parsed = release.orgs;

    if let Some(parent) = parent_prov {
        if let Some(ref parent_date) = parent.trud_release_date {
            if parent_date != &archive_info.release_date {
                progress.clear_live();
                anyhow::bail!(
                    "✖ Release date mismatch: _provenance.json specifies '{}' but archive filename specifies '{}'",
                    parent_date,
                    archive_info.release_date
                );
            }
        }
        prov.trud_release_date = Some(archive_info.release_date);
        if parent.trud_release_sha256.is_some() {
            prov.trud_release_sha256 = parent.trud_release_sha256;
        }
        if parent.trud_release_filesize_bytes.is_some() {
            prov.trud_release_filesize_bytes = parent.trud_release_filesize_bytes;
        }
    } else {
        prov.trud_release_date = Some(archive_info.release_date);
        if let Ok(meta) = std::fs::metadata(&archive_info.archive_path) {
            prov.trud_release_filesize_bytes = Some(meta.len());
        }
        if let Ok(hash) = crate::provenance::compute_file_sha256(&archive_info.archive_path) {
            prov.trud_release_sha256 = Some(hash);
        }
    }
    let resolved = crate::ods_xml::convert_parsed_orgs(parsed);
    let (provenance, records): (Option<crate::provenance::OdsProvenance>, Vec<OdsRecord>) =
        (Some(prov), resolved.into_values().collect());

    or_clear!(std::fs::create_dir_all(&output_path)
        .with_context(|| format!("creating output directory: {}", output_path.display())));

    // The wait here, converting and closing the succession graph, is left visible on purpose:
    // the reading bar is full and the writing bar empty, with no rate, because nothing is
    // being written.
    let edges = build_succession_edges(&records);
    let (successor_closures, predecessor_closures) = compute_transitive_closures(&records, &edges);

    let rows_total = records.len()
        + records.iter().map(|r| r.roles.len()).sum::<usize>()
        + records.iter().map(|r| r.relationships.len()).sum::<usize>()
        + edges.len();
    state.writing(rows_total, &output_path);

    // The four tables share nothing mutable, so each is built and written by one thread of
    // its own. Parallelism goes across files, never inside one: a file's bytes don't depend
    // on how many cores the machine has.
    let prov = provenance.as_ref();
    let on_rows = |n: usize| {
        state.rows_written(n);
        state.paint(&progress);
    };
    let done = |table: usize, file: &str, exported: Exported| -> Result<Exported> {
        let bytes = std::fs::metadata(output_path.join(file)).with_context(|| format!("reading the size of {file}"))?.len();
        state.table_done(table, exported.rows, bytes);
        state.paint(&progress);
        Ok(exported)
    };
    let (successions, roles, relationships, orgs) = std::thread::scope(|scope| {
        let orgs = scope.spawn(|| {
            let exported = write_orgs(&output_path, &records, &successor_closures, &predecessor_closures, prov, &on_rows)
                .context("writing orgs.parquet")?;
            done(3, "orgs.parquet", exported)
        });
        let roles = scope.spawn(|| {
            let exported = write_roles(&output_path, &records, prov, &on_rows).context("writing roles.parquet")?;
            done(1, "roles.parquet", exported)
        });
        let relationships = scope.spawn(|| {
            let exported =
                write_relationships(&output_path, &records, prov, &on_rows).context("writing relationships.parquet")?;
            done(2, "relationships.parquet", exported)
        });
        let successions = scope.spawn(|| {
            let exported =
                write_successions(&output_path, &records, prov, &on_rows).context("writing successions.parquet")?;
            done(0, "successions.parquet", exported)
        });
        // The block's rate is bytes landing on disk, so repaint from here while the writers run.
        let handles = [&successions, &roles, &relationships, &orgs];
        while !handles.iter().all(|h| h.is_finished()) {
            std::thread::sleep(std::time::Duration::from_millis(20));
            state.paint(&progress);
        }
        let join = |h: std::thread::ScopedJoinHandle<'_, Result<Exported>>, table: &str| {
            h.join().unwrap_or_else(|_| Err(anyhow::anyhow!("writing {table}.parquet: the writer thread panicked")))
        };
        (
            join(successions, "successions"),
            join(roles, "roles"),
            join(relationships, "relationships"),
            join(orgs, "orgs"),
        )
    });
    let exported = [or_clear!(successions), or_clear!(roles), or_clear!(relationships), or_clear!(orgs)];

    // Settled: the rate becomes the total size, and the block prints as it stands (once, when
    // stderr isn't a terminal or the command is quiet).
    progress.finish_block(&state.settled(&progress));

    // 6. Write initial _provenance.json to output directory if present
    if let Some(ref p) = provenance {
        if let Ok(prov_json) = serde_json::to_string_pretty(p) {
            let _ = std::fs::write(
                output_path.join(crate::provenance::PROVENANCE_FILENAME),
                prov_json,
            );
        }
    }

    // 7. Complete _provenance.json: check the archive under trud/ against it and take
    //    trud_schema_version from the XML manifest
    crate::provenance::update_provenance(&output_path)?;

    // 8. Ship the datapackage.json alongside the data so the schema and metadata
    //    are reproducible from a release alone, without the tool.
    let release_pkg =
        crate::datapackage::generate_release_datapackage(&output_path, None, None);
    let pkg_json = serde_json::to_string_pretty(&release_pkg)? + "\n";
    std::fs::write(output_path.join("datapackage.json"), pkg_json)
        .context("writing datapackage.json")?;

    if !quiet {
        warn_unexpected_files(&output_path);
    }

    // Warnings come after the block, and only for data the source gave us that isn't in the
    // tables. Stubs the merge set aside lose nothing and aren't warned about.
    for line in &duplicates_dropped {
        eprintln!("! {line}");
    }
    if stubs_remaining > 0 {
        eprintln!(
            "! {} organisations exist only as stubs: no file holds their complete record",
            crate::commands::role::format_number_with_commas(stubs_remaining)
        );
    }
    if let Some(line) = unread_dates_warning(&exported) {
        eprintln!("{line}");
    }
    for line in &held_warnings {
        eprintln!("{line}");
    }

    if abandon_memory {
        // Every writer is closed and every file is flushed. What is left is plain data: no
        // handle, no writer, nothing whose `Drop` does work. `cleanup_scratch()` in `main`
        // still runs, so the extracted XML goes as before.
        std::mem::forget(records);
        std::mem::forget(successor_closures);
        std::mem::forget(predecessor_closures);
        std::mem::forget(edges);
    }

    Ok(output_path)
}

/// What the `ods make` block shows, updated from the parser and writer threads.
struct MakeState {
    records_total: Option<usize>,
    xml_bytes: u64,
    verbose: bool,
    records_done: AtomicUsize,
    bytes_done: AtomicU64,
    rows_total: AtomicUsize,
    rows_done: AtomicUsize,
    writing: Mutex<Option<(Instant, PathBuf)>>,
    tables: Mutex<[Option<MakeTableDone>; 4]>,
    stubs: Mutex<Option<usize>>,
}

const TABLE_FILES: [&str; 4] = ["successions.parquet", "roles.parquet", "relationships.parquet", "orgs.parquet"];

impl MakeState {
    fn new(records_total: Option<usize>, xml_bytes: u64, verbose: bool) -> Self {
        Self {
            records_total,
            xml_bytes,
            verbose,
            records_done: AtomicUsize::new(0),
            bytes_done: AtomicU64::new(0),
            rows_total: AtomicUsize::new(0),
            rows_done: AtomicUsize::new(0),
            writing: Mutex::new(None),
            tables: Mutex::new([None; 4]),
            stubs: Mutex::new(None),
        }
    }

    fn reading(&self, records: usize, bytes: u64) {
        self.records_done.fetch_add(records, Ordering::Relaxed);
        self.bytes_done.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Reading is over: the bar is full, and the stubs the merge set aside are known.
    fn parsed(&self, stubs_superseded: usize) {
        self.records_done.store(self.records_total.unwrap_or_else(|| self.records_done.load(Ordering::Relaxed)), Ordering::Relaxed);
        self.bytes_done.store(self.xml_bytes, Ordering::Relaxed);
        *self.stubs.lock().unwrap() = Some(stubs_superseded);
    }

    fn writing(&self, rows_total: usize, output_dir: &Path) {
        self.rows_total.store(rows_total, Ordering::Relaxed);
        *self.writing.lock().unwrap() = Some((Instant::now(), output_dir.to_path_buf()));
    }

    fn rows_written(&self, rows: usize) {
        self.rows_done.fetch_add(rows, Ordering::Relaxed);
    }

    fn table_done(&self, table: usize, rows: usize, bytes: u64) {
        self.tables.lock().unwrap()[table] = Some(MakeTableDone { rows, bytes });
    }

    /// Bytes on disk across the four files, and how fast they are landing.
    fn landed(&self) -> (u64, Option<f64>) {
        let Some((started, dir)) = self.writing.lock().unwrap().clone() else {
            return (0, None);
        };
        let bytes: u64 = TABLE_FILES
            .iter()
            .filter_map(|f| std::fs::metadata(dir.join(f)).ok())
            .map(|m| m.len())
            .sum();
        let secs = started.elapsed().as_secs_f64();
        (bytes, (bytes > 0 && secs > 0.0).then(|| bytes as f64 / secs))
    }

    fn params(&self, color: bool) -> MakeBlockParams {
        let (landed, rate) = self.landed();
        MakeBlockParams {
            records_total: self.records_total,
            records_done: self.records_done.load(Ordering::Relaxed),
            xml_bytes: self.xml_bytes,
            xml_bytes_done: self.bytes_done.load(Ordering::Relaxed),
            rows_done: self.rows_done.load(Ordering::Relaxed),
            rows_total: self.rows_total.load(Ordering::Relaxed),
            rate,
            bytes_written: 0,
            bytes_landed: landed,
            writing_done: false,
            tables: *self.tables.lock().unwrap(),
            stubs: if self.verbose { *self.stubs.lock().unwrap() } else { None },
            color,
        }
    }

    /// Repaints the live block (a terminal only; `Progress` rate-limits the drawing).
    fn paint(&self, progress: &Progress) {
        let caps = progress.caps();
        if !caps.is_tty || caps.quiet {
            return;
        }
        progress.update_live_block(render_make_block(&self.params(!caps.no_color)));
    }

    /// The finished block: every table closed, the rate settled into the total size.
    fn settled(&self, progress: &Progress) -> Vec<String> {
        let caps = progress.caps();
        let mut params = self.params(caps.is_tty && !caps.no_color);
        params.records_done = params.records_total.unwrap_or(params.records_done);
        params.xml_bytes_done = params.xml_bytes;
        params.rows_done = params.rows_total;
        params.rate = None;
        params.bytes_landed = 0;
        params.writing_done = true;
        params.bytes_written = params.tables.iter().flatten().map(|t| t.bytes).sum();
        render_make_block(&params)
    }
}

/// Fails when the organisations read differ from what the XML manifests declare.
///
/// The comparison is sums over every file, before stubs are merged away: each
/// manifest counts the stubs in its own file.
fn check_record_counts(release: &crate::ods_xml::ParsedRelease) -> Result<()> {
    let Some(declared) = release.records_declared() else {
        return Ok(());
    };
    let read = release.records_read();
    if declared == read {
        return Ok(());
    }
    let fmt = crate::commands::role::format_number_with_commas;
    let mut msg = format!(
        "✖ Manifest record count mismatch: declared {} != read {}",
        fmt(declared),
        fmt(read)
    );
    for f in &release.files {
        msg.push_str(&format!(
            "\n  {}: declared {}, read {}",
            f.name,
            f.declared.map_or_else(|| "none".to_string(), fmt),
            fmt(f.read)
        ));
    }
    anyhow::bail!(msg)
}

/// Whether two paths name the same file, comparing canonical paths where both exist.
fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// One line for the dates the tables couldn't read, or none when every date parsed.
///
/// The tables are taken in their listed order, so "the first" doesn't depend on which writer
/// thread finished first.
fn unread_dates_warning(exported: &[Exported; 4]) -> Option<String> {
    let count: usize = exported.iter().map(|e| e.date_problems.count).sum();
    let first = exported.iter().find_map(|e| e.date_problems.first.as_ref())?;
    let noun = if count == 1 { "1 date could not be read and is null".to_string() } else { format!("{count} dates could not be read and are null") };
    Some(format!("! {noun} (first: {} \"{}\" on {})", first.column, first.value, first.code))
}

pub fn get_unexpected_files(output_dir: &Path) -> Vec<String> {
    let known_files: HashSet<&str> = [
        "orgs.parquet",
        "roles.parquet",
        "relationships.parquet",
        "successions.parquet",
        "datapackage.json",
        crate::provenance::PROVENANCE_FILENAME,
        "provenance.json",
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

/// Days since 1970-01-01 for a `YYYY-MM-DD` date, or `None` when it doesn't parse.
///
/// The XML writes every date in that shape, so the common case is read in place. Anything else,
/// including a well-shaped date that isn't one (`2026-02-30`), goes to chrono, which is what
/// decided the answer before this fast path existed; a malformed date stays null exactly as it
/// did, and an oddity chrono accepts (`2026-8-3`) still parses.
fn parse_date_to_days(val: &str) -> Option<i32> {
    canonical_date_to_days(val).or_else(|| chrono_date_to_days(val))
}

fn chrono_date_to_days(val: &str) -> Option<i32> {
    chrono::NaiveDate::parse_from_str(val, "%Y-%m-%d")
        .ok()
        .map(|date| {
            date.signed_duration_since(chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap())
                .num_days() as i32
        })
}

/// `dddd-dd-dd` that is a real calendar date, read without allocating or formatting machinery.
fn canonical_date_to_days(val: &str) -> Option<i32> {
    let b = val.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let digit = |i: usize| b[i].checked_sub(b'0').filter(|d| *d <= 9).map(i32::from);
    let year = digit(0)? * 1000 + digit(1)? * 100 + digit(2)? * 10 + digit(3)?;
    let month = (digit(5)? * 10 + digit(6)?) as u32;
    let day = (digit(8)? * 10 + digit(9)?) as u32;
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let last_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    if day < 1 || day > last_day {
        return None;
    }
    // Days from civil, after Howard Hinnant's algorithm.
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) as i32 + 2) / 5 + day as i32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146097 + doe - 719468)
}

/// The first date a table couldn't read, kept for the warning after the block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadDate {
    pub column: &'static str,
    pub value: String,
    pub code: String,
}

/// How many dates in one table were null because they didn't parse, and the first of them.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DateProblems {
    pub count: usize,
    pub first: Option<UnreadDate>,
}

impl DateProblems {
    fn note(&mut self, column: &'static str, value: &str, code: &str) {
        self.count += 1;
        self.first.get_or_insert_with(|| UnreadDate {
            column,
            value: value.to_string(),
            code: code.to_string(),
        });
    }
}

/// What a table's writer reports when its file has closed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Exported {
    pub rows: usize,
    pub date_problems: DateProblems,
}

fn append_date(
    builder: &mut Date32Builder,
    val: Option<&str>,
    column: &'static str,
    code: &str,
    problems: &mut DateProblems,
) {
    match val {
        Some(v) => match parse_date_to_days(v) {
            Some(days) => builder.append_value(days),
            None => {
                problems.note(column, v, code);
                builder.append_null();
            }
        },
        None => builder.append_null(),
    }
}

fn extract_dates(
    dates: &[crate::ods_xml::OdsDate],
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
        Field::new("status", DataType::Utf8, false),
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
    problems: &mut DateProblems,
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
        append_date(&mut legal_start, l_start.as_deref(), "legal_start", &r.ods_code, problems);
        append_date(&mut legal_end, l_end.as_deref(), "legal_end", &r.ods_code, problems);
        append_date(&mut operational_start, o_start.as_deref(), "operational_start", &r.ods_code, problems);
        append_date(&mut operational_end, o_end.as_deref(), "operational_end", &r.ods_code, problems);
        append_date(&mut last_change_date, r.last_change_date.as_deref(), "last_changed", &r.ods_code, problems);
        trud_release_date.append_value(release_days);
    }

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(ods_code.finish()) as ArrayRef,
            Arc::new(name.finish()) as ArrayRef,
            Arc::new(status.finish()) as ArrayRef,
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

/// Writes every organisation, sorted by `status` then `ods_code`.
///
/// `active` sorts before `inactive`, so a `WHERE status = 'active'` query over
/// HTTP reads only the row groups that hold active rows.
pub fn export_orgs(
    output_dir: &Path,
    records: &[OdsRecord],
    successor_closures: &HashMap<String, Vec<String>>,
    predecessor_closures: &HashMap<String, Vec<String>>,
    provenance: Option<&crate::provenance::OdsProvenance>,
) -> Result<usize> {
    write_orgs(output_dir, records, successor_closures, predecessor_closures, provenance, &|_| {}).map(|e| e.rows)
}

/// Writes `orgs.parquet` and reports each 50,000-row batch to `on_rows` as it is written.
pub fn write_orgs(
    output_dir: &Path,
    records: &[OdsRecord],
    successor_closures: &HashMap<String, Vec<String>>,
    predecessor_closures: &HashMap<String, Vec<String>>,
    provenance: Option<&crate::provenance::OdsProvenance>,
    on_rows: &(dyn Fn(usize) + Sync),
) -> Result<Exported> {
    let mut problems = DateProblems::default();
    let mut all_records: Vec<&OdsRecord> = records.iter().collect();
    all_records.sort_by(|a, b| (&a.status, &a.ods_code).cmp(&(&b.status, &b.ods_code)));

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

    for chunk in all_records.chunks(BATCH_SIZE) {
        let batch = build_orgs_batch(
            &schema,
            chunk,
            successor_closures,
            predecessor_closures,
            release_days,
            &mut problems,
        )?;
        writer.write(&batch).context("writing orgs batch")?;
        on_rows(chunk.len());
    }
    writer.close().context("finalising orgs writer")?;
    Ok(Exported { rows: all_records.len(), date_problems: problems })
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
    problems: &mut DateProblems,
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
        append_date(&mut legal_start, r.legal_start.as_deref(), "legal_start", &r.ods_code, problems);
        append_date(&mut legal_end, r.legal_end.as_deref(), "legal_end", &r.ods_code, problems);
        append_date(&mut operational_start, r.operational_start.as_deref(), "operational_start", &r.ods_code, problems);
        append_date(&mut operational_end, r.operational_end.as_deref(), "operational_end", &r.ods_code, problems);
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
) -> Result<usize> {
    write_roles(output_dir, records, provenance, &|_| {}).map(|e| e.rows)
}

/// Writes `roles.parquet` and reports each 50,000-row batch to `on_rows` as it is written.
pub fn write_roles(
    output_dir: &Path,
    records: &[OdsRecord],
    provenance: Option<&crate::provenance::OdsProvenance>,
    on_rows: &(dyn Fn(usize) + Sync),
) -> Result<Exported> {
    let mut problems = DateProblems::default();
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
        let batch = build_roles_batch(&schema, chunk, release_days, &mut problems)?;
        writer.write(&batch).context("writing roles batch")?;
        on_rows(chunk.len());
    }
    writer.close().context("finalising roles writer")?;
    Ok(Exported { rows: rows.len(), date_problems: problems })
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
    problems: &mut DateProblems,
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
        append_date(&mut legal_start, r.legal_start.as_deref(), "legal_start", &r.source_code, problems);
        append_date(&mut legal_end, r.legal_end.as_deref(), "legal_end", &r.source_code, problems);
        append_date(&mut operational_start, r.operational_start.as_deref(), "operational_start", &r.source_code, problems);
        append_date(&mut operational_end, r.operational_end.as_deref(), "operational_end", &r.source_code, problems);
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
) -> Result<usize> {
    write_relationships(output_dir, records, provenance, &|_| {}).map(|e| e.rows)
}

/// Writes `relationships.parquet` and reports each 50,000-row batch to `on_rows` as it is written.
pub fn write_relationships(
    output_dir: &Path,
    records: &[OdsRecord],
    provenance: Option<&crate::provenance::OdsProvenance>,
    on_rows: &(dyn Fn(usize) + Sync),
) -> Result<Exported> {
    let mut problems = DateProblems::default();
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
        let batch = build_relationships_batch(&schema, chunk, release_days, &mut problems)?;
        writer
            .write(&batch)
            .context("writing relationships batch")?;
        on_rows(chunk.len());
    }
    writer.close().context("finalising relationships writer")?;
    Ok(Exported { rows: rows.len(), date_problems: problems })
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
    problems: &mut DateProblems,
) -> Result<RecordBatch> {
    let mut predecessor_code = StringBuilder::new();
    let mut successor_code = StringBuilder::new();
    let mut legal_start = Date32Builder::new();
    let mut succession_id = StringBuilder::new();
    let mut trud_release_date = Date32Builder::new();

    for edge in edges {
        predecessor_code.append_value(&edge.predecessor_code);
        successor_code.append_value(&edge.successor_code);
        append_date(&mut legal_start, edge.legal_start.as_deref(), "legal_start", &edge.predecessor_code, problems);
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
) -> Result<usize> {
    write_successions(output_dir, records, provenance, &|_| {}).map(|e| e.rows)
}

/// Writes `successions.parquet` and reports each 50,000-row batch to `on_rows` as it is written.
pub fn write_successions(
    output_dir: &Path,
    records: &[OdsRecord],
    provenance: Option<&crate::provenance::OdsProvenance>,
    on_rows: &(dyn Fn(usize) + Sync),
) -> Result<Exported> {
    let mut problems = DateProblems::default();
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
        let batch = build_successions_batch(&schema, chunk, release_days, &mut problems)?;
        writer.write(&batch).context("writing successions batch")?;
        on_rows(chunk.len());
    }
    writer.close().context("finalising successions writer")?;
    Ok(Exported { rows: edges.len(), date_problems: problems })
}

#[cfg(test)]
mod tests {
    /// D1: the fast date parser gives the answer chrono gives, for every day of the years the
    /// source can hold and for the malformed strings it can't.
    #[test]
    fn fast_date_parse_agrees_with_chrono() {
        let start = chrono::NaiveDate::from_ymd_opt(0, 1, 1).unwrap();
        let mut day = start;
        while day.format("%Y").to_string().parse::<i32>().unwrap() <= 2200 {
            let text = day.format("%Y-%m-%d").to_string();
            assert_eq!(parse_date_to_days(&text), chrono_date_to_days(&text), "{text}");
            day = day.succ_opt().unwrap();
        }
        for year in [2400, 2999, 4000, 9999] {
            for text in [format!("{year}-02-28"), format!("{year}-02-29"), format!("{year}-12-31")] {
                assert_eq!(parse_date_to_days(&text), chrono_date_to_days(&text), "{text}");
            }
        }
        for text in [
            "", "2026", "2026-02-30", "2026-02-29", "2026-13-01", "2026-00-10", "2026-01-00", "2026-01-32",
            "2026-8-3", "2026-08-3x", "2026-08-28T", " 2026-08-28", "2026-08-28 ", "20260828", "+2026-08-28",
            "-0001-01-01", "2026/08/28", "2026-08-28T00:00:00", "١٢٣٤-05-06", "abcd-ef-gh", "0000-00-00",
        ] {
            assert_eq!(parse_date_to_days(text), chrono_date_to_days(text), "{text:?}");
        }
        assert_eq!(parse_date_to_days("1970-01-01"), Some(0));
        assert_eq!(parse_date_to_days("2026-02-30"), None);
    }

    use super::*;
    use crate::ods_xml::Location;

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
                crate::ods_xml::OdsDate {
                    date_type: "Legal".to_string(),
                    start: Some("2006-10-01".to_string()),
                    end: Some("2013-03-31".to_string()),
                },
                crate::ods_xml::OdsDate {
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
            successors: vec![crate::ods_xml::OdsSuccessor {
                unique_succ_id: "777".to_string(),
                succ_type: "Predecessor".to_string(),
                dates: vec![crate::ods_xml::OdsDate {
                    date_type: "Legal".to_string(),
                    start: Some("2006-10-01".to_string()),
                    end: None,
                }],
                target: crate::ods_xml::OdsRelationshipTarget {
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
            &mut DateProblems::default(),
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
                successors: vec![crate::ods_xml::OdsSuccessor {
                    unique_succ_id: "101".to_string(),
                    succ_type: "Successor".to_string(),
                    dates: vec![crate::ods_xml::OdsDate {
                        date_type: "Legal".to_string(),
                        start: Some("2002-04-01".to_string()),
                        end: None,
                    }],
                    target: crate::ods_xml::OdsRelationshipTarget {
                        ods_code: "0CE".to_string(),
                        ..Default::default()
                    },
                }],
                ..Default::default()
            },
            OdsRecord {
                ods_code: "0AN".to_string(),
                status: "inactive".to_string(),
                successors: vec![crate::ods_xml::OdsSuccessor {
                    unique_succ_id: "102".to_string(),
                    succ_type: "Successor".to_string(),
                    dates: vec![],
                    target: crate::ods_xml::OdsRelationshipTarget {
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
                    crate::ods_xml::OdsSuccessor {
                        unique_succ_id: "101".to_string(),
                        succ_type: "Predecessor".to_string(),
                        dates: vec![crate::ods_xml::OdsDate {
                            date_type: "Legal".to_string(),
                            start: Some("2002-04-01".to_string()),
                            end: None,
                        }],
                        target: crate::ods_xml::OdsRelationshipTarget {
                            ods_code: "0AF".to_string(),
                            ..Default::default()
                        },
                    },
                    crate::ods_xml::OdsSuccessor {
                        unique_succ_id: "103".to_string(),
                        succ_type: "Successor".to_string(),
                        dates: vec![],
                        target: crate::ods_xml::OdsRelationshipTarget {
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
                successors: vec![crate::ods_xml::OdsSuccessor {
                    unique_succ_id: "104".to_string(),
                    succ_type: "Successor".to_string(),
                    dates: vec![],
                    target: crate::ods_xml::OdsRelationshipTarget {
                        ods_code: "0CY".to_string(),
                        ..Default::default()
                    },
                }],
                ..Default::default()
            },
            OdsRecord {
                ods_code: "0CY".to_string(),
                status: "inactive".to_string(),
                successors: vec![crate::ods_xml::OdsSuccessor {
                    unique_succ_id: "105".to_string(),
                    succ_type: "Successor".to_string(),
                    dates: vec![],
                    target: crate::ods_xml::OdsRelationshipTarget {
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

        let record = OdsRecord {
            ods_code: "0AF".to_string(),
            name: "Bury HA".to_string(),
            relationships: vec![crate::ods_xml::OdsRelationship {
                id: "RE4".to_string(),
                display_name: Some("IS COMMISSIONED BY".to_string()),
                unique_rel_id: "999".to_string(),
                status: "active".to_string(),
                dates: vec![],
                target: crate::ods_xml::OdsRelationshipTarget {
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
    fn test_column_renames_to_snake_case() {
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
    fn test_trud_release_date_column_on_all_tables() {
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
    }
}

mod common;

use common::{create_inner_zip, create_nested_trud_zip};
use ods::commands::parquet;
use ods::workspace;
use std::fs;
use tempfile::TempDir;

fn manifest_xml(declared: usize, orgs: &[(&str, &str, &str)]) -> String {
    let body: String = orgs
        .iter()
        .map(|(code, name, status)| {
            format!(
                r#"<un:Organisation><un:Name>{name}</un:Name><un:OrgId root="2.16.840.1.113883.2.1.3.2.4.18.48" extension="{code}" /><un:Status value="{status}" /></un:Organisation>"#
            )
        })
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<un:OrganisationManifest xmlns:un="http://refdata.hscic.gov.uk/org/v2-0-0">
  <un:ManifestHeader><un:Version value="2.0.0" /><un:RecordCount value="{declared}" /></un:ManifestHeader>
  <un:Organisations>{body}</un:Organisations>
</un:OrganisationManifest>"#
    )
}

fn write_release_zip(dir: &std::path::Path, full_xml: &str, archive_xml: &str) -> std::path::PathBuf {
    let zip_path = dir.join("hscorgrefdataxml_data_7.0.0_20260529000001.zip");
    let archive_zip = create_inner_zip("HSCOrgRefData_Archive_20260518.xml", archive_xml.as_bytes());
    let full_zip = create_inner_zip("HSCOrgRefData_Full_20260518.xml", full_xml.as_bytes());
    create_nested_trud_zip(&zip_path, &[("archive.zip", &archive_zip), ("fullfile.zip", &full_zip)]);
    zip_path
}

fn parquet_files(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".parquet")).map(String::from))
        .collect();
    names.sort();
    names
}

fn read_codes_and_statuses(orgs_parquet: &std::path::Path) -> Vec<(String, String)> {
    use arrow::array::StringArray;
    use ::parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

    let reader = ParquetRecordBatchReaderBuilder::try_new(fs::File::open(orgs_parquet).unwrap())
        .unwrap()
        .build()
        .unwrap();
    let mut rows = Vec::new();
    for batch in reader {
        let batch = batch.unwrap();
        let col = |name: &str| {
            batch
                .column(batch.schema().index_of(name).unwrap())
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .clone()
        };
        let (codes, statuses) = (col("ods_code"), col("status"));
        rows.extend((0..batch.num_rows()).map(|i| (codes.value(i).to_string(), statuses.value(i).to_string())));
    }
    rows
}

/// D2, D4: `ods make` reads both XML files into one `orgs.parquet`, sorted active first.
#[test]
fn make_writes_one_orgs_table_with_archived_organisations_sorted_active_first() {
    let tmp = TempDir::new().unwrap();
    // The archive holds a closed organisation whose code sorts before every live one,
    // so an ods_code-only sort would put it first.
    let full = manifest_xml(2, &[("RAE", "LIVE ONE", "Active"), ("Y01234", "LIVE TWO", "Active")]);
    let archive = manifest_xml(1, &[("ARCH1", "CLOSED LONG AGO", "Inactive")]);
    let zip_path = write_release_zip(tmp.path(), &full, &archive);

    let out_dir = tmp.path().join("parquet_out");
    parquet::run(parquet::Args { input: Some(zip_path), output: Some(out_dir.clone()), ..Default::default() })
        .expect("make on a release holding fullfile.zip and archive.zip should succeed");

    assert_eq!(parquet_files(&out_dir), ["orgs", "relationships", "roles", "successions"], "one orgs table, and three supporting tables");
    let rows = read_codes_and_statuses(&out_dir.join("orgs.parquet"));
    let expected: Vec<(String, String)> = [("RAE", "active"), ("Y01234", "active"), ("ARCH1", "inactive")]
        .iter()
        .map(|(c, s)| (c.to_string(), s.to_string()))
        .collect();
    assert_eq!(rows, expected, "every organisation from both files, active rows first");
    assert_eq!(workspace::count_records_in_parquet(&out_dir.join("orgs.parquet")).unwrap(), 3);
}

/// D4: the record-count guard compares the sum of both manifests with the records read.
#[test]
fn make_fails_when_a_manifest_record_count_disagrees_with_records_read() {
    let tmp = TempDir::new().unwrap();
    let full = manifest_xml(2, &[("RAE", "LIVE ONE", "Active"), ("Y01234", "LIVE TWO", "Active")]);
    // Declares 5 records and holds 1.
    let archive = manifest_xml(5, &[("ARCH1", "CLOSED LONG AGO", "Inactive")]);
    let zip_path = write_release_zip(tmp.path(), &full, &archive);

    let err = parquet::run(parquet::Args { input: Some(zip_path), output: Some(tmp.path().join("out")), ..Default::default() })
        .expect_err("a manifest that declares more records than the file holds must fail the build");

    let msg = format!("{err:#}");
    assert!(msg.contains("declared 7 != read 3"), "names both totals, got: {msg}");
    assert!(msg.contains("HSCOrgRefData_Archive_20260518.xml: declared 5, read 1"), "names the file, got: {msg}");
}

/// D1: the four tables are written on threads of their own, and a failure in one fails the
/// command naming that table.
#[test]
fn a_failing_export_names_its_table() {
    let tmp = TempDir::new().unwrap();
    let full = manifest_xml(1, &[("RAE", "LIVE ONE", "Active")]);
    let archive = manifest_xml(1, &[("ARCH1", "CLOSED LONG AGO", "Inactive")]);
    let zip_path = write_release_zip(tmp.path(), &full, &archive);
    let out_dir = tmp.path().join("out");
    // A directory where the file should go: creating relationships.parquet fails, the rest succeed.
    fs::create_dir_all(out_dir.join("relationships.parquet")).unwrap();

    let err = parquet::run(parquet::Args { input: Some(zip_path), output: Some(out_dir.clone()), ..Default::default() })
        .expect_err("a table that can't be written must fail the build");

    let msg = format!("{err:#}");
    assert!(msg.contains("writing relationships.parquet"), "names the table, got: {msg}");
    assert!(out_dir.join("orgs.parquet").is_file(), "the other tables were still written");
}

/// A release whose dates don't all parse: every start date is `2020-13-45`, every end date is
/// `2026-8-3` (which chrono reads as 2026-08-03, 20668 days since 1970), and `D1`'s last change
/// isn't a date at all. Returns the zip's path.
fn odd_dates_release(dir: &std::path::Path) -> std::path::PathBuf {
    let date = |t: &str, s: &str, e: &str| {
        format!(r#"<Date><Type value="{t}" /><Start value="{s}" /><End value="{e}" /></Date>"#)
    };
    // Start is not a date; End is one chrono accepts without zero padding (2026-08-03 is 20668 days).
    let bad = |t: &str| date(t, "2020-13-45", "2026-8-3");
    let org = format!(
        r#"<un:Organisation><un:Name>ODD DATES</un:Name>{op}<un:OrgId extension="D1" /><un:Status value="Inactive" /><un:LastChangeDate value="not-a-date" />
<un:Roles><un:Role id="RO177" uniqueRoleId="1" primaryRole="true">{role}<un:Status value="Active" /></un:Role></un:Roles>
<un:Rels><un:Rel id="RE4" uniqueRelId="1">{rel}<un:Status value="Active" /><un:Target><un:OrgId extension="D2" /></un:Target></un:Rel></un:Rels>
<un:Succs><un:Succ uniqueSuccId="1">{succ}<un:Type>Successor</un:Type><un:Target><un:OrgId extension="D2" /></un:Target></un:Succ></un:Succs></un:Organisation>"#,
        op = bad("Operational"),
        role = bad("Operational"),
        rel = bad("Operational"),
        succ = bad("Legal"),
    );
    let d2 = r#"<un:Organisation><un:Name>SUCCESSOR</un:Name><un:OrgId extension="D2" /><un:Status value="Active" /></un:Organisation>"#;
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<un:OrganisationManifest xmlns:un="http://refdata.hscic.gov.uk/org/v2-0-0"><un:ManifestHeader><un:Version value="2.0.0" /><un:RecordCount value="2" /></un:ManifestHeader><un:Organisations>{org}{d2}</un:Organisations></un:OrganisationManifest>"#
    );
    write_release_zip(dir, &xml, &manifest_xml(0, &[]))
}

/// D1: a date that doesn't parse stays null in every table, and one chrono accepts still parses.
#[test]
fn malformed_date_stays_null_in_every_table() {
    use ::arrow::array::{Array, Date32Array};
    use ::parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

    fn date_column(file: &std::path::Path, column: &str) -> Vec<Option<i32>> {
        let reader = ParquetRecordBatchReaderBuilder::try_new(fs::File::open(file).unwrap()).unwrap().build().unwrap();
        let mut out = Vec::new();
        for batch in reader {
            let batch = batch.unwrap();
            let col = batch.column(batch.schema().index_of(column).unwrap());
            let dates = col.as_any().downcast_ref::<Date32Array>().unwrap();
            out.extend((0..dates.len()).map(|i| dates.is_valid(i).then(|| dates.value(i))));
        }
        out
    }

    let tmp = TempDir::new().unwrap();
    let zip_path = odd_dates_release(tmp.path());
    let out = tmp.path().join("out");

    parquet::run(parquet::Args { input: Some(zip_path), output: Some(out.clone()), ..Default::default() }).unwrap();

    // orgs sort active first, so D2 is row 0 and D1 is row 1.
    assert_eq!(date_column(&out.join("orgs.parquet"), "operational_start"), [None, None]);
    assert_eq!(date_column(&out.join("orgs.parquet"), "operational_end"), [None, Some(20668)]);
    assert_eq!(date_column(&out.join("orgs.parquet"), "last_changed"), [None, None]);
    assert_eq!(date_column(&out.join("roles.parquet"), "operational_start"), [None]);
    assert_eq!(date_column(&out.join("roles.parquet"), "operational_end"), [Some(20668)]);
    assert_eq!(date_column(&out.join("relationships.parquet"), "operational_start"), [None]);
    assert_eq!(date_column(&out.join("relationships.parquet"), "operational_end"), [Some(20668)]);
    assert_eq!(date_column(&out.join("successions.parquet"), "legal_start"), [None]);
}

// ---------------------------------------------------------------------------
// The `ods make` report: what is printed, and what is warned about after it
// ---------------------------------------------------------------------------

/// An organisation with an operational period, as the real XML writes one, optionally a stub.
fn org_with_period(code: &str, name: &str, status: &str, ref_only: bool, period: Option<(&str, &str)>) -> String {
    let attr = if ref_only { r#" refOnly="true""# } else { "" };
    let date = period.map_or(String::new(), |(s, e)| {
        format!(r#"<un:Date><un:Type value="Operational" /><un:Start value="{s}" /><un:End value="{e}" /></un:Date>"#)
    });
    format!(
        r#"<un:Organisation{attr}><un:Name>{name}</un:Name>{date}<un:OrgId extension="{code}" /><un:Status value="{status}" /></un:Organisation>"#
    )
}

fn manifest_of(orgs: &[String]) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<un:OrganisationManifest xmlns:un="http://refdata.hscic.gov.uk/org/v2-0-0">
  <un:ManifestHeader><un:Version value="2.0.0" /><un:RecordCount value="{}" /></un:ManifestHeader>
  <un:Organisations>{}</un:Organisations>
</un:OrganisationManifest>"#,
        orgs.len(),
        orgs.concat()
    )
}

/// Runs `ods make parquet` on a zip and returns its stderr. The build must succeed.
fn make_stderr(zip: &std::path::Path, out: &std::path::Path, extra: &[&str]) -> String {
    let output = common::ods_cmd()
        .args(["make", "parquet", "--input"])
        .arg(zip)
        .arg("--output")
        .arg(out)
        .args(extra)
        .output()
        .unwrap();
    assert!(output.status.success(), "ods make failed:\n{}", String::from_utf8_lossy(&output.stderr));
    assert!(output.stdout.is_empty(), "ods make reports on stderr only");
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The `!` lines that name lost data, i.e. every one except the unverified-source line these
/// zips (which have no `_provenance.json`) always carry.
fn data_warnings(stderr: &str) -> Vec<&str> {
    stderr
        .lines()
        .filter(|l| {
            l.starts_with("! ")
                && !l.contains("isn't a TRUD release ods knows")
                && !l.contains("No provenance info found")
        })
        .collect()
}

/// O1: stubs the merge sets aside lose nothing, so the report doesn't warn about them; under
/// `--verbose` it counts them in a `stubs` row.
#[test]
fn make_report_does_not_warn_about_superseded_stubs_but_verbose_counts_them() {
    let tmp = TempDir::new().unwrap();
    let full = manifest_of(&[
        org_with_period("A1", "LIVE", "Active", false, Some(("2020-01-01", "2030-01-01"))),
        org_with_period("Z9", "CLOSED (STUB)", "Inactive", true, None),
    ]);
    let archive = manifest_of(&[org_with_period("Z9", "CLOSED", "Inactive", false, Some(("2001-01-01", "2010-01-01")))]);
    let zip = write_release_zip(tmp.path(), &full, &archive);

    let quiet = make_stderr(&zip, &tmp.path().join("out1"), &[]);
    let verbose = make_stderr(&zip, &tmp.path().join("out2"), &["--verbose"]);

    assert!(quiet.contains("  reading xml   ") && quiet.contains("3 records"), "the block is printed:\n{quiet}");
    assert_eq!(data_warnings(&quiet), Vec::<&str>::new(), "no warning for a stub the merge set aside:\n{quiet}");
    assert!(!quiet.contains("stubs"), "the stubs row is for --verbose:\n{quiet}");
    assert!(verbose.contains("  stubs         1 superseded"), "--verbose counts them:\n{verbose}");
}

/// O1: a record the merge dropped is data in the source that isn't in the tables, and is warned
/// about after the block, not above it.
#[test]
fn make_warns_after_the_block_only_for_dropped_records() {
    let tmp = TempDir::new().unwrap();
    let full = manifest_of(&[org_with_period("A1", "LIVE", "Active", false, Some(("2020-01-01", "2030-01-01")))]);
    let archive = manifest_of(&[
        org_with_period("T1", "LOGISTICS", "Inactive", false, Some(("2000-04-01", "2003-03-31"))),
        org_with_period("T1", "LOGISTICS", "Inactive", false, Some(("2003-04-01", "2004-09-30"))),
    ]);
    let zip = write_release_zip(tmp.path(), &full, &archive);

    let stderr = make_stderr(&zip, &tmp.path().join("out"), &[]);

    assert_eq!(
        data_warnings(&stderr),
        ["! T1 has two complete records in HSCOrgRefData_Archive_20260518.xml: kept 2003-04-01 to 2004-09-30, dropped 2000-04-01 to 2003-03-31"]
    );
    let block_end = stderr.find("  orgs          ").expect("the block's last row");
    assert!(stderr.find("! T1 has").unwrap() > block_end, "the warning follows the block:\n{stderr}");
}

/// D3: a date that couldn't be read is null in the table and named, with the first one, after the block.
#[test]
fn make_names_a_date_it_could_not_read_and_leaves_it_null() {
    let tmp = TempDir::new().unwrap();
    let zip = odd_dates_release(tmp.path());

    let stderr = make_stderr(&zip, &tmp.path().join("out"), &[]);

    // Four start dates (orgs, roles, relationships, successions) and one last-changed date don't
    // parse; the successions table comes first, so its date is the one named.
    assert_eq!(
        data_warnings(&stderr),
        [r#"! 5 dates could not be read and are null (first: legal_start "2020-13-45" on D1)"#]
    );
    assert!(stderr.find("! 5 dates").unwrap() > stderr.find("  orgs          ").unwrap(), "after the block:\n{stderr}");
}

/// O1: `--quiet` prints the report block and the warnings and nothing else: no `* Source:` line,
/// no `* Provenance:` line, no notes. Without it the source line opens the report.
#[test]
fn make_quiet_prints_the_block_and_the_warnings_and_nothing_else() {
    let tmp = TempDir::new().unwrap();
    let full = manifest_of(&[org_with_period("A1", "LIVE", "Active", false, Some(("2020-01-01", "2030-01-01")))]);
    let archive = manifest_of(&[
        org_with_period("T1", "LOGISTICS", "Inactive", false, Some(("2000-04-01", "2003-03-31"))),
        org_with_period("T1", "LOGISTICS", "Inactive", false, Some(("2003-04-01", "2004-09-30"))),
    ]);
    let zip = write_release_zip(tmp.path(), &full, &archive);

    let quiet = make_stderr(&zip, &tmp.path().join("out1"), &["--quiet"]);
    let loud = make_stderr(&zip, &tmp.path().join("out2"), &[]);

    // Six block rows, then the warning lines: the dropped record and the unverified source note.
    let lines: Vec<&str> = quiet.lines().collect();
    assert_eq!(lines.len(), 9, "the block and the warnings only:\n{quiet}");
    assert!(
        lines[0].starts_with("  reading xml   "),
        "the block comes first:\n{quiet}"
    );
    assert!(
        lines[6].starts_with("! T1 has two complete records"),
        "{quiet}"
    );
    assert!(
        lines[7].contains("isn't a TRUD release ods knows"),
        "{quiet}"
    );
    assert!(lines[8].contains("Built without provenance"), "{quiet}");
    assert!(
        !quiet
            .lines()
            .any(|l| l.starts_with("* ") || l.starts_with("✓ ")),
        "no info lines under --quiet:\n{quiet}"
    );
    assert!(
        loud.starts_with("* Source: "),
        "without --quiet the source line opens the report:\n{loud}"
    );
}

/// D1: two archives with the same file name and byte length do not collide in ODS_CACHE_DIR;
/// the second build reflects its own content, not stale cached XML from the first.
#[test]
fn same_name_same_length_zips_do_not_collide_in_xml_cache() {
    let cache_dir = TempDir::new().unwrap();
    let dir1 = TempDir::new().unwrap();
    let dir2 = TempDir::new().unwrap();

    let full1 = manifest_xml(1, &[("AAA1", "PRACTICE ONE", "Active")]);
    let archive1 = manifest_xml(0, &[]);
    let zip1 = write_release_zip(dir1.path(), &full1, &archive1);
    let len1 = fs::metadata(&zip1).unwrap().len();

    std::thread::sleep(std::time::Duration::from_millis(20));

    let mut zip2 = dir2.path().join("placeholder");
    for pad in 0..50 {
        let name = format!("PRACTICE TWO{}", " ".repeat(pad));
        let full2 = manifest_xml(1, &[("BBB2", &name, "Active")]);
        let z = write_release_zip(dir2.path(), &full2, &archive1);
        if fs::metadata(&z).unwrap().len() == len1 {
            zip2 = z;
            break;
        }
    }

    assert_eq!(
        fs::metadata(&zip1).unwrap().len(),
        fs::metadata(&zip2).unwrap().len(),
        "both zips must have the exact same byte length to test cache collision"
    );

    let out1 = dir1.path().join("out1");
    let out2 = dir2.path().join("out2");

    let run1 = common::ods_cmd()
        .env("ODS_CACHE_DIR", cache_dir.path())
        .args(["make", "-i", zip1.to_str().unwrap(), "-o", out1.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(run1.status.success(), "first make failed: {}", String::from_utf8_lossy(&run1.stderr));

    let run2 = common::ods_cmd()
        .env("ODS_CACHE_DIR", cache_dir.path())
        .args(["make", "-i", zip2.to_str().unwrap(), "-o", out2.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(run2.status.success(), "second make failed: {}", String::from_utf8_lossy(&run2.stderr));

    let rows1 = read_codes_and_statuses(&out1.join("orgs.parquet"));
    assert_eq!(rows1, [("AAA1".to_string(), "active".to_string())]);

    let rows2 = read_codes_and_statuses(&out2.join("orgs.parquet"));
    assert_eq!(
        rows2,
        [("BBB2".to_string(), "active".to_string())],
        "second build must reflect zip2 content rather than stale cache from zip1"
    );
}


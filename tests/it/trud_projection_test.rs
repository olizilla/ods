use crate::common;

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
  <un:ManifestHeader><un:Version value="2.0.0" /><un:PublicationDate value="2026-05-18" /><un:RecordCount value="{declared}" /></un:ManifestHeader>
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
    parquet::run(parquet::Args { input: Some(zip_path), output: Some(out_dir.clone()), force: true, ..Default::default() })
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

    let err = parquet::run(parquet::Args { input: Some(zip_path), output: Some(tmp.path().join("out")), force: true, ..Default::default() })
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

    let err = parquet::run(parquet::Args { input: Some(zip_path), output: Some(out_dir.clone()), force: true, ..Default::default() })
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
<un:OrganisationManifest xmlns:un="http://refdata.hscic.gov.uk/org/v2-0-0"><un:ManifestHeader><un:Version value="2.0.0" /><un:PublicationDate value="2026-05-18" /><un:RecordCount value="2" /></un:ManifestHeader><un:Organisations>{org}{d2}</un:Organisations></un:OrganisationManifest>"#
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

    parquet::run(parquet::Args { input: Some(zip_path), output: Some(out.clone()), force: true, ..Default::default() }).unwrap();

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

/// Every row of every table is dated by the XML's own PublicationDate: 2026-05-18 here, in a zip
/// TRUD would name 2026-05-29. The zip matches no release, so this is a `--force` build, which
/// dates its rows the same way a matched one does.
#[test]
fn make_dates_every_row_by_the_xml_publication_date() {
    use ::arrow::array::{Array, Date32Array};
    use ::parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

    let tmp = TempDir::new().unwrap();
    let zip_path = odd_dates_release(tmp.path());
    let out = tmp.path().join("out");
    parquet::run(parquet::Args { input: Some(zip_path), output: Some(out.clone()), force: true, ..Default::default() }).unwrap();

    // 2026-05-18 is 20591 days after 1970-01-01.
    for table in ["orgs", "roles", "relationships", "successions"] {
        let reader = ParquetRecordBatchReaderBuilder::try_new(fs::File::open(out.join(format!("{table}.parquet"))).unwrap())
            .unwrap()
            .build()
            .unwrap();
        let mut dates = Vec::new();
        for batch in reader {
            let batch = batch.unwrap();
            let col = batch.column(batch.schema().index_of("publication_date").unwrap());
            let col = col.as_any().downcast_ref::<Date32Array>().unwrap();
            dates.extend((0..col.len()).map(|i| col.value(i)));
        }
        assert!(!dates.is_empty() && dates.iter().all(|d| *d == 20591), "{table}: {dates:?}");
    }
}

/// The two XML files of a release must carry one PublicationDate. A release zip whose files
/// disagree, or where one carries none, refuses the build with a block naming each file and
/// what it says; so does a zip holding one XML file that carries none.
#[test]
fn make_refuses_xml_files_without_one_publication_date() {
    let tmp = TempDir::new().unwrap();
    let full = manifest_xml(1, &[("RAE", "LIVE ONE", "Active")]);
    let later = manifest_xml(1, &[("ARCH1", "CLOSED LONG AGO", "Inactive")]).replace("2026-05-18", "2026-05-25");
    let undate = |xml: &str| xml.replace(r#"<un:PublicationDate value="2026-05-18" />"#, "");
    let undated = undate(&manifest_xml(1, &[("ARCH1", "CLOSED LONG AGO", "Inactive")]));

    let refusal = |input: &std::path::Path, dir: &std::path::Path| {
        let output = common::ods_cmd()
            .args(["make", "--input"])
            .arg(input)
            .arg("--output")
            .arg(dir.join("out"))
            .arg("--force")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(!dir.join("out").join("orgs.parquet").exists(), "nothing is written");
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        stderr.lines().skip_while(|l| !l.starts_with('✖')).collect::<Vec<_>>().join("\n")
    };
    let zip_refusal = |archive: &str| {
        let dir = TempDir::new_in(tmp.path()).unwrap();
        let zip = write_release_zip(dir.path(), &full, archive);
        refusal(&zip, dir.path())
    };

    assert_eq!(
        zip_refusal(&later),
        "✖ hscorgrefdataxml_data_7.0.0_20260529000001.zip holds no full file and archive file with the same PublicationDate\n  \
         fullfile.zip  HSCOrgRefData_Full_20260518.xml     PublicationDate 2026-05-18\n  \
         archive.zip   HSCOrgRefData_Archive_20260518.xml  PublicationDate 2026-05-25\n  \
         Every row is dated as of NHS's publication date, so ods can't build this release until that's understood."
    );
    assert_eq!(
        zip_refusal(&undated),
        "✖ hscorgrefdataxml_data_7.0.0_20260529000001.zip holds no full file and archive file with the same PublicationDate\n  \
         fullfile.zip  HSCOrgRefData_Full_20260518.xml     PublicationDate 2026-05-18\n  \
         archive.zip   HSCOrgRefData_Archive_20260518.xml  no PublicationDate\n  \
         Every row is dated as of NHS's publication date, so ods can't build this release until that's understood."
    );

    // A zip holding one XML file directly, as a hand-made archive might.
    let dir = TempDir::new_in(tmp.path()).unwrap();
    let flat = dir.path().join("hscorgrefdataxml_data_7.0.0_20260529000001.zip");
    fs::write(&flat, create_inner_zip("HSCOrgRefData_Full_20260518.xml", undate(&full).as_bytes())).unwrap();
    assert_eq!(
        refusal(&flat, dir.path()),
        "✖ The release's XML doesn't say when NHS England published it\n  \
         HSCOrgRefData_Full_20260518.xml  no PublicationDate\n  \
         Every row is dated as of NHS's publication date, so ods can't build this release without one."
    );
}

/// A `--force` build doesn't need TRUD's name for the zip: nothing is read from the name, and the
/// rows are dated by the XML. `find` reads the result.
#[test]
fn force_builds_a_renamed_zip_and_find_reads_it() {
    let tmp = TempDir::new().unwrap();
    let full = manifest_xml(1, &[("RAE", "LIVE ONE", "Active")]);
    let archive = manifest_xml(1, &[("ARCH1", "CLOSED LONG AGO", "Inactive")]);
    let zip = write_release_zip(tmp.path(), &full, &archive);
    let renamed = tmp.path().join("archive.zip");
    fs::rename(&zip, &renamed).unwrap();
    let out = tmp.path().join("out");

    let make = common::ods_cmd().args(["make", "--input"]).arg(&renamed).arg("--output").arg(&out).arg("--force").output().unwrap();
    assert!(make.status.success(), "{}", String::from_utf8_lossy(&make.stderr));
    assert!(String::from_utf8_lossy(&make.stderr).contains("! archive.zip isn't a TRUD release ods knows"));

    let find = common::ods_cmd().args(["find", "--code", "RAE", "--plain", "-i"]).arg(&out).output().unwrap();
    assert!(find.status.success(), "{}", String::from_utf8_lossy(&find.stderr));
    assert!(String::from_utf8_lossy(&find.stdout).contains("LIVE ONE"));

    // Without --force it's refused as an archive ods doesn't know, not for its name.
    let refused = common::ods_cmd().args(["make", "--input"]).arg(&renamed).arg("--output").arg(tmp.path().join("out2")).output().unwrap();
    assert_eq!(refused.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(stderr.contains("✖ archive.zip isn't a TRUD release ods knows") && stderr.contains("--force"), "{stderr}");
}

/// A zip that doesn't hold a release's inner zips says what's missing, under `--force` too.
#[test]
fn force_refuses_a_zip_without_the_release_zips_naming_what_is_missing() {
    let tmp = TempDir::new().unwrap();
    let full = manifest_xml(1, &[("RAE", "LIVE ONE", "Active")]);
    let zip = tmp.path().join("something.zip");
    {
        let mut w = zip::ZipWriter::new(fs::File::create(&zip).unwrap());
        w.start_file("fullfile.zip", zip::write::SimpleFileOptions::default()).unwrap();
        std::io::Write::write_all(&mut w, &create_inner_zip("HSCOrgRefData_Full_20260518.xml", full.as_bytes())).unwrap();
        w.finish().unwrap();
    }
    let out = common::ods_cmd().args(["make", "--input"]).arg(&zip).arg("--output").arg(tmp.path().join("out")).arg("--force").output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("something.zip holds fullfile.zip but no archive.zip"), "{stderr}");
}

/// `ods trud diff` reads a release zip as `ods make` does: both files, and when `fullfile.zip` holds a
/// second full file, the one whose PublicationDate matches the archive's, naming the one skipped.
#[test]
fn diff_reads_the_pair_make_builds_from() {
    let tmp = TempDir::new().unwrap();
    let dated = |date: &str, orgs: &[(&str, &str, &str)]| manifest_xml(orgs.len(), orgs).replace("2026-05-18", date);
    let new_full = dated("2026-05-18", &[("RAE", "NEW NAME", "Active")]);
    let archive = create_inner_zip("HSCOrgRefData_Archive_20260518.xml", dated("2026-05-18", &[("ARCH1", "CLOSED LONG AGO", "Inactive")]).as_bytes());

    // fullfile.zip holding both full files, the older first.
    let mut two = std::io::Cursor::new(Vec::new());
    {
        let mut w = zip::ZipWriter::new(&mut two);
        for (name, bytes) in [("HSCOrgRefData_Full_20260511.xml", dated("2026-05-11", &[("RAE", "OLD NAME", "Active")])), ("HSCOrgRefData_Full_20260518.xml", new_full.clone())] {
            w.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            std::io::Write::write_all(&mut w, bytes.as_bytes()).unwrap();
        }
        w.finish().unwrap();
    }
    let target = tmp.path().join("hscorgrefdataxml_data_7.0.0_20260529000001.zip");
    create_nested_trud_zip(&target, &[("archive.zip", &archive), ("fullfile.zip", &two.into_inner())]);
    let baseline_dir = tmp.path().join("baseline");
    fs::create_dir_all(&baseline_dir).unwrap();
    let baseline = write_release_zip(&baseline_dir, &dated("2026-05-18", &[("RAE", "NEW NAME", "Active")]), &dated("2026-05-18", &[("ARCH1", "CLOSED LONG AGO", "Inactive")]));

    let out = common::ods_cmd().args(["trud", "diff"]).arg(&baseline).arg(&target).args(["--format", "markdown"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(
        stderr.contains("! hscorgrefdataxml_data_7.0.0_20260529000001.zip: fullfile.zip holds 2 full files; built from HSCOrgRefData_Full_20260518.xml (2026-05-18), skipped HSCOrgRefData_Full_20260511.xml (2026-05-11)"),
        "{stderr}"
    );
    // Both sides read the same pair, archive included: nothing changed.
    assert!(stdout.contains("Baseline: 2 records as of 2026-05-18  ➔  Target: 2 records as of 2026-05-18"), "{stdout}");
    assert!(stdout.contains("- ✎ Modified:        0 organisations"), "the older full file wasn't read: {stdout}");
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
  <un:ManifestHeader><un:Version value="2.0.0" /><un:PublicationDate value="2026-05-18" /><un:RecordCount value="{}" /></un:ManifestHeader>
  <un:Organisations>{}</un:Organisations>
</un:OrganisationManifest>"#,
        orgs.len(),
        orgs.concat()
    )
}

/// Runs `ods make parquet` on a zip and returns its stderr. The build must succeed. These zips
/// are synthetic fixtures with no matching release-index row, so `--force` is needed to build
/// them at all; the report and warnings under test are unaffected by it.
fn make_stderr(zip: &std::path::Path, out: &std::path::Path, extra: &[&str]) -> String {
    let output = common::ods_cmd()
        .args(["make", "parquet", "--input"])
        .arg(zip)
        .arg("--output")
        .arg(out)
        .arg("--force")
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
        .args(["make", "-i", zip1.to_str().unwrap(), "-o", out1.to_str().unwrap(), "--force"])
        .output()
        .unwrap();
    assert!(run1.status.success(), "first make failed: {}", String::from_utf8_lossy(&run1.stderr));

    let run2 = common::ods_cmd()
        .env("ODS_CACHE_DIR", cache_dir.path())
        .args(["make", "-i", zip2.to_str().unwrap(), "-o", out2.to_str().unwrap(), "--force"])
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


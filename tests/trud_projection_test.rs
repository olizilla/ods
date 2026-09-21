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
    parquet::run(parquet::Args { input: Some(zip_path), output: Some(out_dir.clone()) })
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

    let err = parquet::run(parquet::Args { input: Some(zip_path), output: Some(tmp.path().join("out")) })
        .expect_err("a manifest that declares more records than the file holds must fail the build");

    let msg = format!("{err:#}");
    assert!(msg.contains("declared 7 != read 3"), "names both totals, got: {msg}");
    assert!(msg.contains("HSCOrgRefData_Archive_20260518.xml: declared 5, read 1"), "names the file, got: {msg}");
}

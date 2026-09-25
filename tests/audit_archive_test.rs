//! The audit reads both XML files of a release, and holds the parquet to what they contain.

mod common;

use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

const HEADER: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<un:OrganisationManifest xmlns:un="http://refdata.hscic.gov.uk/org/v2-0-0">
  <un:ManifestHeader>
    <un:PublicationType value="Full" />
    <un:PublicationDate value="2026-07-31" />
    <un:PublicationSeqNum value="4700" />
    <un:PrimaryRoleScope>
      <un:PrimaryRole id="RO177" displayName="Prescribing Cost Centre" />
    </un:PrimaryRoleScope>
  </un:ManifestHeader>
  <un:CodeSystems>
    <un:CodeSystem name="ODS_Role" id="2.16.840.1.113883.2.1.3.2.4.17.507">
      <un:concept id="RO177" displayName="Prescribing Cost Centre" />
    </un:CodeSystem>
  </un:CodeSystems>
  <un:Organisations>"#;

const FOOTER: &str = "\n  </un:Organisations>\n</un:OrganisationManifest>";

const LIVE: &str = r#"
    <un:Organisation>
      <un:Name>LIVE PRACTICE</un:Name>
      <un:OrgId root="2.16.840.1.113883.2.1.3.2.4.18.48" extension="A100" />
      <un:Status value="Active" />
      <un:Date type="Operational"><un:Start value="2020-01-01" /></un:Date>
      <un:OrgRecordClass value="RC1" />
      <un:Role id="RO177" uniqueRoleId="1" primaryRole="true" status="Active" />
    </un:Organisation>"#;

/// The same closed organisation as the archive's complete record and as a stub in the full file.
fn closed(ref_only: bool) -> String {
    let attr = if ref_only { r#" refOnly="true""# } else { "" };
    format!(
        r#"
    <un:Organisation{attr}>
      <un:Name>CLOSED PRACTICE</un:Name>
      <un:OrgId root="2.16.840.1.113883.2.1.3.2.4.18.48" extension="Z900" />
      <un:Status value="Inactive" />
      <un:Date type="Operational"><un:Start value="2010-01-01" /><un:End value="2016-01-01" /></un:Date>
      <un:OrgRecordClass value="RC1" />
      <un:Role id="RO177" uniqueRoleId="9" primaryRole="true" status="Inactive" />
    </un:Organisation>"#
    )
}

struct Release {
    _tmp: TempDir,
    workspace: PathBuf,
    zip: PathBuf,
    release_dir: PathBuf,
}

/// A built release whose full file holds a stub of `Z900` and whose archive holds the complete record.
fn build_release() -> Release {
    build_release_from(
        &format!("{HEADER}{LIVE}{}{FOOTER}", closed(true)),
        &format!("{HEADER}{}{FOOTER}", closed(false)),
    )
}

fn build_release_from(full_xml: &str, archive_xml: &str) -> Release {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ods_data");
    let release_dir = workspace.join("releases").join("2026-07-31");
    let trud_dir = release_dir.join("trud");
    fs::create_dir_all(&trud_dir).unwrap();

    let full = common::create_inner_zip("HSCOrgRefData_Full_20260731.xml", full_xml.as_bytes());
    let archive = common::create_inner_zip("HSCOrgRefData_Archive_20260731.xml", archive_xml.as_bytes());
    let zip = trud_dir.join("hscorgrefdataxml_data_7.0.0_20260731000001.zip");
    common::create_nested_trud_zip(&zip, &[("fullfile.zip", &full), ("archive.zip", &archive)]);

    let prov = ods::provenance::OdsProvenance::from_trud_statement(
        "2026-07-31",
        &ods::provenance::compute_file_sha256(&zip).unwrap(),
        fs::metadata(&zip).unwrap().len(),
    );
    fs::write(
        release_dir.join(ods::provenance::PROVENANCE_FILENAME),
        serde_json::to_string_pretty(&prov).unwrap(),
    )
    .unwrap();

    ods::workspace::Workspace::open_or_create(Some(&workspace)).unwrap().set_active("2026-07-31").unwrap();
    ods::commands::parquet::run(ods::commands::parquet::Args {
        input: Some(release_dir.clone()),
        output: Some(release_dir.clone()), ..Default::default() })
    .unwrap();

    Release { _tmp: tmp, workspace, zip, release_dir }
}

fn audit(release: &Release) -> std::process::Output {
    common::ods_cmd()
        .args(["trud", "audit", "--full", "-w"])
        .arg(&release.workspace)
        .arg("-i")
        .arg(&release.zip)
        .output()
        .unwrap()
}

/// B4: a stub is set aside when the other file holds its complete record, so its role isn't counted twice.
#[test]
fn audit_counts_a_superseded_stub_once() {
    let release = build_release();

    let out = audit(&release);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "audit must pass on a correctly built release:\n{stdout}");
    assert!(stdout.contains("Entities: 2"), "two organisations, not three:\n{stdout}");
    assert!(stdout.contains("3 records in 2 files, 1 stubs superseded"), "names the count it compared:\n{stdout}");
    assert!(stdout.contains("Roles: 2"), "the stub's role isn't counted:\n{stdout}");
}

/// B4: an organisation the archive holds and `orgs.parquet` lacks fails the audit and names its code.
#[test]
fn audit_fails_naming_the_code_when_an_archived_organisation_is_missing() {
    let release = build_release();
    let live_only = ods::ods_xml::OdsRecord {
        ods_code: "A100".to_string(),
        name: "LIVE PRACTICE".to_string(),
        status: "active".to_string(),
        record_class: "org".to_string(),
        ..Default::default()
    };
    let prov = ods::provenance::OdsProvenance::load_from_dir(&release.release_dir).ok();
    let no_closures = std::collections::HashMap::new();
    ods::commands::parquet::export_orgs(&release.release_dir, &[live_only], &no_closures, &no_closures, prov.as_ref())
        .unwrap();

    let out = audit(&release);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "audit must fail:\n{stdout}");
    assert!(stdout.contains("Missing Row (Z900)"), "names the missing code:\n{stdout}");
}

/// An organisation with a child-element operational period, as the real XML writes one.
fn dated(code: &str, name: &str, status: &str, start: &str, end: Option<&str>, role_id: u32) -> String {
    let end = end.map_or(String::new(), |e| format!(r#"<un:End value="{e}" />"#));
    format!(
        r#"
    <un:Organisation orgRecordClass="RC1">
      <un:Name>{name}</un:Name>
      <un:Date><un:Type value="Operational" /><un:Start value="{start}" />{end}</un:Date>
      <un:OrgId root="2.16.840.1.113883.2.1.3.2.4.18.48" extension="{code}" />
      <un:Status value="{status}" />
      <un:OrgRecordClass value="RC1" />
      <un:Role id="RO177" uniqueRoleId="{role_id}" primaryRole="true" status="{status}" />
    </un:Organisation>"#
    )
}

/// B4: a code the source publishes twice resolves to one organisation, and the audit expects exactly that one.
#[test]
fn audit_counts_a_resolved_duplicate_once() {
    // R1 is reused: a live pharmacy in the full file, a laundry in the archive.
    // T1 is in the archive twice, one period after the other.
    let full = format!(
        "{HEADER}{}{}{FOOTER}",
        dated("A100", "LIVE PRACTICE", "Active", "2020-01-01", None, 1),
        dated("R1", "PHARMACY", "Active", "2025-04-23", None, 2)
    );
    let archive = format!(
        "{HEADER}{}{}{}{FOOTER}",
        dated("R1", "LAUNDRY", "Inactive", "1991-04-01", Some("1993-03-31"), 3),
        dated("T1", "LOGISTICS", "Inactive", "2000-04-01", Some("2003-03-31"), 4),
        dated("T1", "LOGISTICS", "Inactive", "2003-04-01", Some("2004-09-30"), 5)
    );
    let release = build_release_from(&full, &archive);

    let out = audit(&release);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "audit must pass on a correctly built release:\n{stdout}");
    assert!(stdout.contains("Entities: 3"), "A100, R1 and T1, once each:\n{stdout}");
    assert!(stdout.contains("5 records in 2 files, 0 stubs superseded, 2 duplicate records dropped"), "{stdout}");
    assert!(stdout.contains("Roles: 3"), "the dropped records' roles aren't counted:\n{stdout}");
}

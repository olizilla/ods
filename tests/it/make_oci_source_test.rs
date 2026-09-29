//! `ods make oci --source` packs a release's `trud/` into an OCI bundle of NHS's files, and
//! refuses, naming the release, the check and both values, whenever the files don't agree.
//! Protects R10: a source archive is kept whole, as NHS published it.

use crate::common;

use ods::oci::source;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn trud(release: &Path) -> std::path::PathBuf {
    release.join("trud")
}

fn refuses(release: &Path) -> source::Refusal {
    let refusal = source::pack(release).expect_err("the bundle must be refused");
    println!("{}", refusal.block());
    assert!(!trud(release).join("oci").exists(), "nothing is written when a check fails");
    refusal
}

#[test]
fn test_a_release_with_a_valid_source_packs() {
    let tmp = TempDir::new().unwrap();
    let release = common::create_source_release(tmp.path(), "2026-07-31");

    let bundle = source::pack(&release).unwrap();

    assert_eq!(bundle.layer_names().len(), 4);
    let blobs = trud(&release).join("oci/blobs/sha256");
    let zip_blob = fs::read_dir(&blobs)
        .unwrap()
        .flatten()
        .find(|e| e.path().is_symlink() && fs::read_link(e.path()).unwrap().to_string_lossy().ends_with(".zip"))
        .expect("the zip is a layer");
    assert_eq!(
        fs::read_link(zip_blob.path()).unwrap().parent().unwrap().to_string_lossy(),
        "../../..",
        "layer blobs are relative symlinks"
    );
    assert!(source::verify(&release).is_ok(), "a bundle verifies against its own files");
}

// The zip's filename date and TRUD's release date don't always agree (2022-05-30's zip is stamped
// 20220527); `_provenance.json` is TRUD's word, so it names the bundle.
#[test]
fn test_a_bundle_takes_its_date_from_provenance_not_the_zips_filename() {
    let tmp = TempDir::new().unwrap();
    let release = common::create_source_release_stamped(tmp.path(), "2022-05-30", "20220527");

    let bundle = source::pack(&release).expect("a filename date that differs from TRUD's is no refusal");

    assert_eq!(bundle.date, "2022-05-30");
    let manifest: serde_json::Value = serde_json::from_slice(&bundle.manifest_bytes).unwrap();
    assert_eq!(manifest["annotations"]["fyi.ods.trud-release-date"], "2022-05-30");
    let index = fs::read_to_string(trud(&release).join("oci/index.json")).unwrap();
    assert!(index.contains(r#""org.opencontainers.image.ref.name":"2022-05-30""#), "{index}");
    assert!(!index.contains("2022-05-27"), "{index}");
    assert!(source::verify(&release).is_ok());
}

// A refusal that comes before `_provenance.json` is read is labelled with the release directory.
#[test]
fn test_a_refusal_before_provenance_is_read_names_the_release_directory() {
    let tmp = TempDir::new().unwrap();
    let release = common::create_source_release_stamped(tmp.path(), "2022-05-30", "20220527");
    fs::write(trud(&release).join("hscorgrefdataxml_data_7.0.0_20220527000001.zip"), b"another zip").unwrap();

    let refusal = refuses(&release);

    assert_eq!(refusal.release, "2022-05-30", "the directory's name, not the filename's date: {}", refusal.block());
    assert!(refusal.block().starts_with("✖ 2022-05-30  "), "{}", refusal.block());
}

#[test]
fn test_a_missing_signature_is_refused() {
    let tmp = TempDir::new().unwrap();
    let release = common::create_source_release(tmp.path(), "2026-07-31");
    fs::remove_file(trud(&release).join("trud_hscorgrefdataxml_data_7.0.0_20260731000001.xml.asc")).unwrap();

    let refusal = refuses(&release);

    assert_eq!(refusal.release, "2026-07-31");
    assert!(refusal.what.contains("all four"), "{}", refusal.block());
    assert!(refusal.block().contains("signature"), "{}", refusal.block());
}

#[test]
fn test_a_checksum_that_is_not_the_zips_is_refused() {
    let tmp = TempDir::new().unwrap();
    let release = common::create_source_release(tmp.path(), "2026-07-31");
    // NHS's file, but the zip beside it has changed
    fs::write(trud(&release).join("hscorgrefdataxml_data_7.0.0_20260731000001.zip"), b"another zip").unwrap();

    let refusal = refuses(&release);

    let block = refusal.block();
    assert!(block.contains("SHA-1"), "{block}");
    assert!(block.contains("NHS's SHA-1") && block.contains("the zip's SHA-1"), "both values: {block}");
}

#[test]
fn test_a_checksum_naming_another_zip_is_refused() {
    let tmp = TempDir::new().unwrap();
    let release = common::create_source_release(tmp.path(), "2026-07-31");
    let xml = trud(&release).join("trud_hscorgrefdataxml_data_7.0.0_20260731000001.xml");
    let text = fs::read_to_string(&xml).unwrap();
    fs::write(&xml, text.replace("<name>hscorgrefdataxml_data_7.0.0_20260731000001.zip", "<name>some_other.zip")).unwrap();

    let block = refuses(&release).block();

    assert!(block.contains("some_other.zip") && block.contains("hscorgrefdataxml_data_7.0.0_20260731000001.zip"), "{block}");
}

#[test]
fn test_a_provenance_naming_a_different_archive_is_refused() {
    let tmp = TempDir::new().unwrap();
    let release = common::create_source_release(tmp.path(), "2026-07-31");
    let prov_path = ods::provenance::trud_archive_package_path(&release);
    let mut prov: serde_json::Value = serde_json::from_str(&fs::read_to_string(&prov_path).unwrap()).unwrap();
    prov["resources"][0]["hash"] = serde_json::json!(format!("sha256:{}", "1".repeat(64)));
    fs::write(&prov_path, serde_json::to_string_pretty(&prov).unwrap()).unwrap();

    let block = refuses(&release).block();

    assert!(block.contains("different archive"), "{block}");
    assert!(block.contains(&"1".repeat(64)) && block.contains("the zip's SHA-256"), "both values: {block}");
}

#[test]
fn test_a_missing_or_unreadable_provenance_is_refused_with_a_way_to_repair_it() {
    let tmp = TempDir::new().unwrap();
    let release = common::create_source_release(tmp.path(), "2026-07-31");
    fs::write(ods::provenance::trud_archive_package_path(&release), r#"{"name":"nhs-ods-xml","_type":"old"}"#).unwrap();

    let block = refuses(&release).block();

    assert!(block.contains("ods trud pull 2026-07-31"), "{block}");
}

// `--all` packs each release that holds a zip, carries on past a failure and exits 1 at the end;
// a second run skips what already verifies.
#[test]
fn test_all_packs_every_release_and_names_the_one_that_fails() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    for date in ["2026-07-31", "2026-06-26", "2026-05-29"] {
        common::create_source_release(&ws, date);
    }
    fs::remove_file(trud(&ws.join("releases/2026-06-26")).join("trud_hscorgrefdataxml_data_7.0.0_20260626000001.xml.asc")).unwrap();

    let run = || {
        common::ods_binary()
            .args(["make", "oci", "--source", "--all", "-w"])
            .arg(&ws)
            .output()
            .expect("run ods make oci --source --all")
    };
    let first = run();
    let stdout = String::from_utf8_lossy(&first.stdout).to_string();
    let stderr = String::from_utf8_lossy(&first.stderr).to_string();
    println!("$ ods make oci --source --all -w ods_data\n{stdout}{stderr}(exit {:?})", first.status.code());

    assert_eq!(first.status.code(), Some(1), "a failure in the batch exits 1");
    assert!(stdout.contains("✓ 2026-07-31") && stdout.contains("✓ 2026-05-29"), "{stdout}");
    assert!(stdout.contains("✖ 2 packed · 0 skipped · 1 failed"), "a batch with a failure ends in ✖: {stdout}");
    assert!(stderr.contains("✖ 2026-06-26"), "the failure is named: {stderr}");

    let second = run();
    let stdout = String::from_utf8_lossy(&second.stdout).to_string();
    println!("(again)\n{stdout}(exit {:?})", second.status.code());
    assert!(stdout.contains("✖ 0 packed · 2 skipped · 1 failed"), "{stdout}");
    assert!(!stdout.contains("✓ 2026-07-31"), "a bundle that verifies isn't listed again: {stdout}");
}

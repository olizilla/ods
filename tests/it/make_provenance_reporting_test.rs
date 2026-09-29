//! Acceptance tests for `.agents/briefs/make-names-its-provenance.md`.
//!
//! Verifies:
//! - Task 1: Report an unverified source when building from an archive file without
//!   `_provenance.json` above it: a `!` line after the report block (`make-output.md` holds
//!   warnings until the block has settled). Directory input keeps existing hard error.
//! - Task 2: Name the archive read (`* Source: <path>`) first, as `ods info` names its source,
//!   and the `_provenance.json` used (`* Provenance: <path>`) only when it isn't the release
//!   directory's own. Everything `ods make` prints goes to stderr; stdout stays empty.

use crate::common;

use common::{create_mock_trud_zip as create_mock_zip, ods_binary};
use std::fs;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Task 1: Report an unverified source at the start
// ---------------------------------------------------------------------------

#[test]
fn test_task1_unverified_archive_prints_warning_and_builds_successfully() {
    let tmp = TempDir::new().unwrap();
    let archive_dir = tmp.path().join("bare_archive");
    fs::create_dir_all(&archive_dir).unwrap();
    let zip_path = create_mock_zip(&archive_dir, "hscorgrefdataxml_data_8.0.0_20260828000001.zip");

    let out_dir = tmp.path().join("out");
    fs::create_dir_all(&out_dir).unwrap();

    let output = ods_binary()
        .current_dir(tmp.path())
        .arg("make")
        .arg("-i")
        .arg(&zip_path)
        .arg("-o")
        .arg(&out_dir)
        .arg("--force")
        .output()
        .expect("execute ods make");

    assert!(output.status.success(), "ods make must succeed on bare archive");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    // 1. Warning line on stderr naming the input path
    let filename = zip_path.file_name().unwrap().to_str().unwrap();
    let expected_warning = format!("! {} isn't a TRUD release ods knows", filename);
    assert!(
        stderr.contains(&expected_warning),
        "stderr must contain unverified warning, got:\n{}",
        stderr
    );
    assert!(
        stderr.contains("Built without provenance. You can explore it with find, info and role, but not cite or publish it."),
        "stderr must note built without provenance, got:\n{}",
        stderr
    );

    // 2. The warning waits until the report block has settled
    let warn_pos = stderr.find(&expected_warning).unwrap();
    let last_row_pos = stderr.find("  orgs          ").expect("the block's last row must appear in stderr");
    assert!(
        warn_pos > last_row_pos,
        "the unverified warning must come after the block, got:\n{}",
        stderr
    );

    // 3. stdout has no warning or provenance line
    assert!(!stdout.contains("! No provenance info found"), "stdout must not contain warning");
    assert!(!stdout.contains("* Provenance:"), "stdout must not claim provenance file exists");
}

#[test]
fn test_directory_input_without_provenance_fails_with_hard_error() {
    let tmp = TempDir::new().unwrap();
    let input_dir = tmp.path().join("dir_without_prov");
    fs::create_dir_all(&input_dir).unwrap();
    create_mock_zip(&input_dir, "hscorgrefdataxml_data_8.0.0_20260828000001.zip");

    let out_dir = tmp.path().join("out");
    fs::create_dir_all(&out_dir).unwrap();

    let output = ods_binary()
        .current_dir(tmp.path())
        .arg("make")
        .arg("-i")
        .arg(&input_dir)
        .arg("-o")
        .arg(&out_dir)
        .output()
        .expect("execute ods make");

    assert!(!output.status.success(), "directory input without provenance must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Missing trud/datapackage.json in input directory") && stderr.contains("Did you run 'ods trud pull' first?"),
        "must bail with existing directory error, got:\n{}",
        stderr
    );
    assert!(
        !stderr.contains("! No provenance info found"),
        "must not print unverified warning for directory input"
    );
}

// ---------------------------------------------------------------------------
// Task 2: Name the file it used
// ---------------------------------------------------------------------------

#[test]
fn test_make_names_its_source_and_says_nothing_of_the_release_dirs_own_provenance() {
    let tmp = TempDir::new().unwrap();
    let ws_root = tmp.path().join("workspace");
    fs::create_dir_all(&ws_root).unwrap();

    // Setup releases/2026-08-28 directory with trud zip and its TRUD archive package
    let rel_dir = ws_root.join("releases").join("2026-08-28");
    let trud_dir = rel_dir.join("trud");
    fs::create_dir_all(&trud_dir).unwrap();
    let zip_path = create_mock_zip(&trud_dir, "hscorgrefdataxml_data_8.0.0_20260828000001.zip");
    let zip_sha256 = ods::provenance::compute_file_sha256(&zip_path).unwrap();

    ods::provenance::write_trud_archive_package(
        &rel_dir,
        "2026-08-28",
        "hscorgrefdataxml_data_8.0.0_20260828000001.zip",
        &zip_sha256,
        fs::metadata(&zip_path).unwrap().len(),
        &[],
    )
    .unwrap();

    // Open/create workspace and pin active release to 2026-08-28
    let ws = ods::workspace::Workspace::open_or_create(Some(&ws_root)).unwrap();
    ws.set_active("2026-08-28").unwrap();

    // Run `ods make` from workspace root with no arguments
    let output = ods_binary()
        .current_dir(&ws_root)
        .arg("make")
        .output()
        .expect("execute ods make");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "ods make must succeed, stderr:\n{}", stderr);

    // 1. The first line names the source, relative to the workspace, as `ods info` names its own
    assert_eq!(
        stderr.lines().next(),
        Some("* Source: releases/2026-08-28/trud/hscorgrefdataxml_data_8.0.0_20260828000001.zip"),
        "the first line names the archive read, got:\n{}",
        stderr
    );
    assert!(!stderr.contains("→"), "no '<date> (current) → <dir>' line, got:\n{}", stderr);

    // 2. The release directory's own TRUD archive package is the expected one, so it goes unnamed
    assert!(
        ws_root.join("releases/2026-08-28/trud/datapackage.json").exists(),
        "the release's own TRUD archive package exists on disk"
    );
    assert!(!stderr.contains("* Provenance:"), "the release's own provenance isn't named, got:\n{}", stderr);

    // 3. The report block follows the source line
    assert!(stderr.contains("  reading xml   "), "stderr must contain the report block, got:\n{}", stderr);

    // 4. Bare `ods make` writes the Parquet files and the datapackage.json view, and packs
    // nothing: `ods make oci` does that, for publishing. Stdout stays empty.
    assert!(stdout.trim().is_empty(), "stdout must be empty, got:\n{}", stdout);
    assert!(rel_dir.join("datapackage.json").exists(), "the view is written");
    assert!(!rel_dir.join("oci").exists(), "nothing is packed");

    // 5. Stderr does not contain ! warning
    assert!(
        !stderr.contains("! No provenance info found"),
        "stderr must not contain ! warning when provenance exists"
    );
}

#[test]
fn test_make_after_trud_pull_prints_no_warning() {
    let tmp = TempDir::new().unwrap();
    // A directory holding the zip and its record side by side, as an explicit `-i <dir>` can.
    let zip_dir = tmp.path().join("prov_dir");
    fs::create_dir_all(&zip_dir).unwrap();
    let zip_path = create_mock_zip(&zip_dir, "hscorgrefdataxml_data_8.0.0_20260828000001.zip");
    let zip_sha256 = ods::provenance::compute_file_sha256(&zip_path).unwrap();

    let record = ods::provenance::TrudArchivePackage::for_trud_release(
        "2026-08-28",
        "hscorgrefdataxml_data_8.0.0_20260828000001.zip",
        &zip_sha256,
        fs::metadata(&zip_path).unwrap().len(),
        &[],
    )
    .unwrap();
    fs::write(zip_dir.join(ods::provenance::TRUD_ARCHIVE_PACKAGE_FILENAME), record.to_json_string().unwrap()).unwrap();

    let out_dir = tmp.path().join("out");
    fs::create_dir_all(&out_dir).unwrap();

    let output = ods_binary()
        .current_dir(tmp.path())
        .arg("make")
        .arg("-i")
        .arg(&zip_dir)
        .arg("-o")
        .arg(&out_dir)
        .output()
        .expect("execute ods make");

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    // A provenance file from outside the release being built is named, after the source.
    assert!(stderr.contains("* Provenance:"), "a provenance from elsewhere is named:\n{stderr}");
    assert!(
        stderr.find("* Source:").unwrap() < stderr.find("* Provenance:").unwrap(),
        "the source comes first:\n{stderr}"
    );
    assert!(stdout.trim().is_empty());
    assert!(!stderr.contains("! No provenance info found"));
}

/// A release an older `ods trud pull` left holds `trud/_provenance.json` and no TRUD archive package:
/// `ods make` names it and the command that replaces it, without reading it.
#[test]
fn test_directory_with_an_older_record_names_the_repair() {
    let tmp = TempDir::new().unwrap();
    let release_dir = tmp.path().join("releases").join("2026-08-28");
    let trud_dir = release_dir.join("trud");
    fs::create_dir_all(&trud_dir).unwrap();
    create_mock_zip(&trud_dir, "hscorgrefdataxml_data_8.0.0_20260828000001.zip");
    // Only the file's presence matters: nothing reads what it holds.
    fs::write(trud_dir.join("_provenance.json"), "{}").unwrap();

    let output = ods_binary()
        .current_dir(tmp.path())
        .arg("make")
        .arg("-i")
        .arg(&release_dir)
        .arg("-o")
        .arg(&release_dir)
        .output()
        .expect("execute ods make");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("✖ releases/2026-08-28 holds trud/_provenance.json, the record an older ods wrote, and no trud/datapackage.json"),
        "got:\n{stderr}"
    );
    assert!(stderr.contains("ods trud pull 2026-08-28 --force && ods make"), "got:\n{stderr}");
}

/// A release directory's zip is the one its TRUD archive package names, whatever date TRUD stamped in its
/// name: 2022-05-30's zip is `…_20220527000001.zip`. The SHA-256 and size tie it to the record.
#[test]
fn test_release_dir_builds_the_zip_its_record_names_whatever_date_its_name_carries() {
    let tmp = TempDir::new().unwrap();
    let release_dir = tmp.path().join("releases").join("2022-05-30");
    let trud_dir = release_dir.join("trud");
    fs::create_dir_all(&trud_dir).unwrap();
    let zip = create_mock_zip(&trud_dir, "hscorgrefdataxml_data_5.0.0_20220527000001.zip");
    let sha = ods::provenance::compute_file_sha256(&zip).unwrap();
    let bytes = fs::metadata(&zip).unwrap().len();
    ods::provenance::write_trud_archive_package(&release_dir, "2022-05-30", "hscorgrefdataxml_data_5.0.0_20220527000001.zip", &sha, bytes, &[]).unwrap();

    let out = tmp.path().join("out");
    let output = ods_binary().arg("make").arg("-i").arg(&release_dir).arg("-o").arg(&out).output().expect("execute ods make");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "ods make must build the zip the record names, got:\n{stderr}");

    let facts = ods::provenance::read_release(&out).unwrap();
    let facts = facts.facts().expect("the build carries provenance");
    assert_eq!(facts.release_date, "2022-05-30", "the release is TRUD's date, not the name's");
}

/// A record whose archive isn't on disk is refused, naming the file and the repair.
#[test]
fn test_release_dir_refuses_a_record_naming_a_missing_zip_with_the_repair() {
    let tmp = TempDir::new().unwrap();
    let release_dir = tmp.path().join("releases").join("2022-05-30");
    fs::create_dir_all(release_dir.join("trud")).unwrap();
    ods::provenance::write_trud_archive_package(
        &release_dir,
        "2022-05-30",
        "hscorgrefdataxml_data_5.0.0_20220527000001.zip",
        "CA0FEE7512F593ADA1FA9B95BF1372B41911167DA463A98FECF33ADFD86697E5",
        38138574,
        &[],
    )
    .unwrap();

    let output = ods_binary()
        .current_dir(tmp.path())
        .arg("make")
        .arg("-i")
        .arg(&release_dir)
        .arg("-o")
        .arg(tmp.path().join("out"))
        .output()
        .expect("execute ods make");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("names the archive hscorgrefdataxml_data_5.0.0_20220527000001.zip, and it isn't in")
            && stderr.contains("Download it again: ods trud pull 2022-05-30 --force"),
        "got:\n{stderr}"
    );
    assert!(!tmp.path().join("out").join("orgs.parquet").exists());
}

// ---------------------------------------------------------------------------
// `ods make -i <release dir>` writes into that release
// ---------------------------------------------------------------------------

/// A workspace holding two pulled, unbuilt releases with the pin on the older one.
fn workspace_with_two_pulled_releases(tmp: &TempDir) -> std::path::PathBuf {
    let ws_root = tmp.path().join("ods_data");
    for (date, stamp) in [("2026-08-28", "20260828"), ("2026-09-25", "20260925")] {
        let rel_dir = ws_root.join("releases").join(date);
        let trud_dir = rel_dir.join("trud");
        fs::create_dir_all(&trud_dir).unwrap();
        let name = format!("hscorgrefdataxml_data_8.0.0_{stamp}000001.zip");
        let zip_path = create_mock_zip(&trud_dir, &name);
        let sha = ods::provenance::compute_file_sha256(&zip_path).unwrap();
        ods::provenance::write_trud_archive_package(&rel_dir, date, &name, &sha, fs::metadata(&zip_path).unwrap().len(), &[]).unwrap();
    }
    let ws = ods::workspace::Workspace::open_or_create(Some(&ws_root)).unwrap();
    ws.set_active("2026-08-28").unwrap();
    ws_root
}

#[test]
fn test_make_with_a_release_dir_builds_into_that_release_not_the_pinned_one() {
    let tmp = TempDir::new().unwrap();
    let ws_root = workspace_with_two_pulled_releases(&tmp);
    let newer = ws_root.join("releases").join("2026-09-25");
    let pinned = ws_root.join("releases").join("2026-08-28");

    let output = ods_binary().current_dir(tmp.path()).arg("make").arg("-i").arg(&newer).output().expect("execute ods make");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "ods make -i <release dir> must build, got:\n{stderr}");

    assert!(newer.join("orgs.parquet").is_file(), "the named release is built, got:\n{stderr}");
    assert!(!pinned.join("orgs.parquet").exists(), "the pinned release is left alone");
    assert!(!pinned.join("datapackage.json").exists(), "nothing is written into the pinned release");
    let stray: Vec<_> = fs::read_dir(&pinned).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
    assert_eq!(stray, ["trud"], "the pinned release still holds only trud/");
}

#[test]
fn test_make_with_a_release_trud_dir_builds_into_the_release_holding_it() {
    let tmp = TempDir::new().unwrap();
    let ws_root = workspace_with_two_pulled_releases(&tmp);
    let newer = ws_root.join("releases").join("2026-09-25");
    let pinned = ws_root.join("releases").join("2026-08-28");

    let output = ods_binary().current_dir(tmp.path()).arg("make").arg("-i").arg(newer.join("trud")).output().expect("execute ods make");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "ods make -i <release>/trud must build, got:\n{stderr}");

    assert!(newer.join("orgs.parquet").is_file(), "the release holding trud/ is built, got:\n{stderr}");
    assert!(!newer.join("trud").join("orgs.parquet").exists(), "nothing lands inside trud/");
    assert!(!pinned.join("orgs.parquet").exists(), "the pinned release is left alone");
}

#[test]
fn test_make_with_a_relative_trud_dir_builds_into_the_release_holding_it() {
    let tmp = TempDir::new().unwrap();
    let ws_root = workspace_with_two_pulled_releases(&tmp);
    let newer = ws_root.join("releases").join("2026-09-25");

    // From inside trud/, `-i .` names it with no parent component to strip.
    let output = ods_binary().current_dir(newer.join("trud")).arg("make").arg("-i").arg(".").output().expect("execute ods make");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "ods make -i . from trud/ must build, got:\n{stderr}");
    assert!(newer.join("orgs.parquet").is_file(), "the release holding trud/ is built, got:\n{stderr}");
    assert!(!newer.join("trud").join("orgs.parquet").exists(), "nothing lands inside trud/");
}

#[test]
fn test_make_output_flag_still_wins_over_the_release_dir() {
    let tmp = TempDir::new().unwrap();
    let ws_root = workspace_with_two_pulled_releases(&tmp);
    let newer = ws_root.join("releases").join("2026-09-25");
    let out = tmp.path().join("out");

    let output = ods_binary().current_dir(tmp.path()).arg("make").arg("-i").arg(&newer).arg("-o").arg(&out).output().expect("execute ods make");
    assert!(output.status.success(), "got:\n{}", String::from_utf8_lossy(&output.stderr));
    assert!(out.join("orgs.parquet").is_file(), "-o names where the files go");
    assert!(!newer.join("orgs.parquet").exists(), "the release dir isn't written when -o is given");
}

#[test]
fn test_make_with_no_input_still_builds_the_active_release() {
    let tmp = TempDir::new().unwrap();
    let ws_root = workspace_with_two_pulled_releases(&tmp);

    let output = ods_binary().current_dir(&ws_root).arg("make").output().expect("execute ods make");
    assert!(output.status.success(), "got:\n{}", String::from_utf8_lossy(&output.stderr));
    assert!(ws_root.join("releases/2026-08-28/orgs.parquet").is_file(), "the pinned release is built");
    assert!(!ws_root.join("releases/2026-09-25/orgs.parquet").exists(), "the other release is left alone");
}

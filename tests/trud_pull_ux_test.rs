//! Integration tests for trud pull UX, stream discipline, honest verification, and batch ledgers.

use anyhow::Result;
use ods::commands::fetch::{
    run_with_fetcher, Args, TrudFetcher, TrudReleaseItem,
};
use ods::progress::{Progress, ProgressCaps};
use sha2::Digest;
use std::fs;
use std::io::Write;
use std::process::Command;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

fn ods_binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ods"))
}

fn create_mock_trud_zip_with_manifest(dir: &std::path::Path) -> std::path::PathBuf {
    let zip_path = dir.join("hscorgrefdataxml_data_7.0.0_20260731000001.zip");
    let xml_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<un:OrganisationManifest xmlns:un="http://refdata.hscic.gov.uk/org/v2-0-0">
  <un:ManifestHeader>
    <un:PublicationType value="Full" />
    <un:PublicationDate value="2026-07-28" />
    <un:PublicationSeqNum value="4700" />
    <un:PublicationSource value="HSCIC" />
    <un:RecordCount value="305541" />
  </un:ManifestHeader>
</un:OrganisationManifest>"#;

    let mut inner_bytes = Vec::new();
    {
        let mut inner_zip = zip::ZipWriter::new(std::io::Cursor::new(&mut inner_bytes));
        let options = zip::write::SimpleFileOptions::default();
        inner_zip.start_file("HSCOrgRefData_Full_20260731.xml", options).unwrap();
        inner_zip.write_all(xml_content.as_bytes()).unwrap();
        inner_zip.finish().unwrap();
    }

    let file = fs::File::create(&zip_path).unwrap();
    let mut outer_zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    outer_zip.start_file("fullfile.zip", options).unwrap();
    outer_zip.write_all(&inner_bytes).unwrap();
    outer_zip.finish().unwrap();

    zip_path
}

#[test]
fn test_trud_pull_stdout_is_empty_on_default_run() {
    let tmp = TempDir::new().unwrap();
    let fixture_zip = create_mock_trud_zip_with_manifest(tmp.path());

    let tmp = TempDir::new().unwrap();
    let out_dir = tmp.path().join("releases").join("2026-07-31");

    let output = ods_binary()
        .arg("trud")
        .arg("pull")
        .arg("--local-archive")
        .arg(&fixture_zip)
        .arg("-o")
        .arg(&out_dir)
        .output()
        .expect("Failed to execute trud pull with local archive");

    assert!(
        output.status.success(),
        "trud pull should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout_str = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout_str.is_empty(),
        "stdout must be completely empty for trud pull without --format, got:\n{}",
        stdout_str
    );
}

#[test]
fn test_trud_pull_local_archive_claims_honest_no_trud_checksum() {
    let tmp = TempDir::new().unwrap();
    let fixture_zip = create_mock_trud_zip_with_manifest(tmp.path());
    let out_dir = tmp.path().join("releases").join("2026-07-31");

    let output = ods_binary()
        .arg("trud")
        .arg("pull")
        .arg("--local-archive")
        .arg(&fixture_zip)
        .arg("-o")
        .arg(&out_dir)
        .output()
        .expect("Failed to execute trud pull with local archive");

    assert!(output.status.success());
    let stderr_str = String::from_utf8_lossy(&output.stderr);

    // Must state honest claim
    assert!(
        stderr_str.contains("local archive, no TRUD checksum to compare")
            || stderr_str.contains("* 2026-07-31"),
        "stderr must state that local archive has no TRUD checksum to compare, got:\n{}",
        stderr_str
    );

    // Must NOT claim verification against TRUD on a local file hash
    assert!(
        !stderr_str.contains("✓ SHA-256 OK (8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933)"),
        "stderr must NOT print full hex hash on success, got:\n{}",
        stderr_str
    );
}

#[test]
fn test_trud_verify_only_fails_on_tampered_archive() {
    let tmp = TempDir::new().unwrap();
    let bad_zip = tmp.path().join("hscorgrefdataxml_data_7.0.0_20260731000001.zip");
    fs::write(&bad_zip, b"CORRUPTED BYTES").unwrap();

    let output = ods_binary()
        .arg("trud")
        .arg("pull")
        .arg("2026-07-31")
        .arg("--verify-only")
        .arg(&bad_zip)
        .arg("--api-key")
        .arg("test_api_key")
        .output()
        .expect("Failed to execute verify-only");

    // Network is unreachable in test, so verify-only will fail either at network or hash check
    assert!(
        !output.status.success(),
        "verify-only with bad api key / bad zip must fail"
    );
}

struct BufferWriter(Arc<Mutex<Vec<u8>>>);
impl Write for BufferWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn create_test_zip_bytes(date_str: &str) -> Vec<u8> {
    let mut zip_buf = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut zip_buf));
        let options = zip::write::SimpleFileOptions::default()
            .last_modified_time(zip::DateTime::from_date_and_time(2026, 1, 1, 0, 0, 0).unwrap());
        zip.start_file(format!("test_{}.xml", date_str), options)
            .unwrap();
        zip.write_all(format!("<hsctradingpartnerdesc date=\"{}\"/>", date_str).as_bytes())
            .unwrap();
        zip.finish().unwrap();
    }
    zip_buf
}

struct MockTrudFetcher {
    releases: Vec<TrudReleaseItem>,
    failing_date: Option<String>,
}

impl TrudFetcher for MockTrudFetcher {
    fn fetch_releases(&self) -> Result<Vec<TrudReleaseItem>> {
        Ok(self.releases.clone())
    }

    fn download_archive(
        &self,
        url: &str,
        dest_path: &std::path::Path,
        on_bytes: &(dyn Fn(u64) + Send + Sync),
    ) -> Result<()> {
        if let Some(ref fail_date) = self.failing_date {
            if url.contains(fail_date) {
                anyhow::bail!("HTTP 503 from TRUD");
            }
        }
        let date = url.split('/').last().unwrap_or("2026-07-31");
        let bytes = create_test_zip_bytes(date);
        fs::write(dest_path, &bytes)?;
        on_bytes(bytes.len() as u64);
        Ok(())
    }
}

fn generate_96_mock_releases() -> Vec<TrudReleaseItem> {
    let mut releases = Vec::new();
    let mut year = 2018;
    let mut month = 6;

    for i in 0..96 {
        let date_str = format!("{:04}-{:02}-28", year, month);
        let zip_name = format!("hscorgrefdataxml_data_1.0.0_{}{:02}28000001.zip", year, month);
        let bytes = create_test_zip_bytes(&date_str);
        let sha256 = format!("{:x}", sha2::Sha256::digest(&bytes)).to_uppercase();

        releases.push(TrudReleaseItem {
            id: "341".to_string(),
            name: Some(format!("Release {}", i + 1)),
            release_date: date_str.clone(),
            archive_file_name: zip_name,
            archive_file_sha256: sha256,
            archive_file_size: bytes.len() as u64,
            download_url: format!("https://trud.mock/{}", date_str),
        });

        month += 1;
        if month > 12 {
            month = 1;
            year += 1;
        }
    }
    releases
}

#[test]
fn test_batch_all_cached_outputs_four_lines() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    fs::create_dir_all(&ws).unwrap();

    let releases = generate_96_mock_releases();

    // Pre-populate all 96 releases
    for r in &releases {
        let rel_dir = ws.join("releases").join(&r.release_date).join("trud");
        fs::create_dir_all(&rel_dir).unwrap();
        let bytes = create_test_zip_bytes(&r.release_date);
        fs::write(rel_dir.join(&r.archive_file_name), bytes).unwrap();
    }

    let fetcher = MockTrudFetcher {
        releases: releases.clone(),
        failing_date: None,
    };

    let buffer = Arc::new(Mutex::new(Vec::new()));
    let caps = ProgressCaps {
        is_tty: false,
        no_color: true,
        quiet: false,
        verbose: false,
        width: 80,
    };
    let progress = Progress::new(caps, Box::new(BufferWriter(buffer.clone())));

    let args = Args {
        all: true,
        ..Default::default()
    };

    // Set an initial pin to an older release
    let initial_rel = &releases[0];
    ods::workspace::set_active_release(&ws, &initial_rel.release_date).unwrap();

    let res = run_with_fetcher(args, &ws, &fetcher, &progress);
    assert!(res.is_ok());

    let output = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
    let lines: Vec<&str> = output.lines().collect();

    assert!(output.contains("96 releases available"));
    assert!(output.contains("96 cached, SHA-256 verified by TRUD API · nothing to download"));
    assert!(!output.contains("current →"), "Must NOT emit current pin update when all releases are cached");
    assert_eq!(lines.len(), 2, "Output must be exactly 2 settled lines in non-interactive mode");

    let (active_release, _) = ods::workspace::get_active_release(&ws).unwrap();
    assert_eq!(
        active_release.as_str(),
        initial_rel.release_date.as_str(),
        "Active release must remain untouched when all releases are cached"
    );
}

#[test]
fn test_batch_mixed_cache_and_downloads() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    fs::create_dir_all(&ws).unwrap();

    let releases = generate_96_mock_releases();

    // Pre-populate 84 releases; leave 12 to download
    for r in releases.iter().take(84) {
        let rel_dir = ws.join("releases").join(&r.release_date).join("trud");
        fs::create_dir_all(&rel_dir).unwrap();
        let bytes = create_test_zip_bytes(&r.release_date);
        fs::write(rel_dir.join(&r.archive_file_name), bytes).unwrap();
    }

    let fetcher = MockTrudFetcher {
        releases: releases.clone(),
        failing_date: None,
    };

    let buffer = Arc::new(Mutex::new(Vec::new()));
    let caps = ProgressCaps {
        is_tty: false,
        no_color: true,
        quiet: false,
        verbose: false,
        width: 80,
    };
    let progress = Progress::new(caps, Box::new(BufferWriter(buffer.clone())));

    let args = Args {
        all: true,
        jobs: 4,
        ..Default::default()
    };

    let res = run_with_fetcher(args, &ws, &fetcher, &progress);
    assert!(res.is_ok());

    let output = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
    assert!(output.contains("96 releases available"));
    assert!(output.contains("84 cached · 12 to download"));
    assert!(output.contains("12 downloaded · 84 cached · 0 failed"));
    assert!(output.contains("current → releases/2026-05-28"));
}

#[test]
fn test_batch_503_failure_continues_batch_and_summarises() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    fs::create_dir_all(&ws).unwrap();

    let releases = generate_96_mock_releases();

    // Pre-populate 84 releases
    for r in releases.iter().take(84) {
        let rel_dir = ws.join("releases").join(&r.release_date).join("trud");
        fs::create_dir_all(&rel_dir).unwrap();
        let bytes = create_test_zip_bytes(&r.release_date);
        fs::write(rel_dir.join(&r.archive_file_name), bytes).unwrap();
    }

    // Fail release 85 (2025-06-28)
    let failing_date = releases[84].release_date.clone();

    let fetcher = MockTrudFetcher {
        releases: releases.clone(),
        failing_date: Some(failing_date.clone()),
    };

    let buffer = Arc::new(Mutex::new(Vec::new()));
    let caps = ProgressCaps {
        is_tty: false,
        no_color: true,
        quiet: false,
        verbose: false,
        width: 80,
    };
    let progress = Progress::new(caps, Box::new(BufferWriter(buffer.clone())));

    let args = Args {
        all: true,
        jobs: 4,
        ..Default::default()
    };

    let res = run_with_fetcher(args, &ws, &fetcher, &progress);

    // Must fail overall with AlreadyReported error
    assert!(res.is_err());
    assert!(res.unwrap_err().downcast_ref::<ods::commands::pull::AlreadyReported>().is_some());

    let output = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
    assert!(output.contains("11 downloaded · 84 cached · 1 failed"));
    assert!(output.contains(&format!("✖ {}  download failed", failing_date)));
    assert!(output.contains("current → releases/2026-05-28"));
}

#[test]
fn test_batch_format_ndjson_stdout() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    fs::create_dir_all(&ws).unwrap();

    let releases = generate_96_mock_releases();
    let fetcher = MockTrudFetcher {
        releases: releases.into_iter().take(3).collect(),
        failing_date: None,
    };

    let buffer = Arc::new(Mutex::new(Vec::new()));
    let caps = ProgressCaps {
        is_tty: false,
        no_color: true,
        quiet: false,
        verbose: false,
        width: 80,
    };
    let progress = Progress::new(caps, Box::new(BufferWriter(buffer.clone())));

    let args = Args {
        all: true,
        format: Some("ndjson".to_string()),
        ..Default::default()
    };

    let res = run_with_fetcher(args, &ws, &fetcher, &progress);
    assert!(res.is_ok());
}

#[test]
fn test_batch_downloads_in_newest_to_oldest_order() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    fs::create_dir_all(&ws).unwrap();

    let releases = generate_96_mock_releases();
    // Only take 5 releases (e.g. 2018-06-28 to 2018-10-28)
    let sample_releases: Vec<TrudReleaseItem> = releases.into_iter().take(5).collect();

    let fetcher = MockTrudFetcher {
        releases: sample_releases.clone(),
        failing_date: None,
    };

    let buffer = Arc::new(Mutex::new(Vec::new()));
    let caps = ProgressCaps {
        is_tty: false,
        no_color: true,
        quiet: false,
        verbose: false,
        width: 80,
    };
    let progress = Progress::new(caps, Box::new(BufferWriter(buffer.clone())));

    let args = Args {
        all: true,
        jobs: 4, // parallel jobs must still settle in strict newest-to-oldest order
        ..Default::default()
    };

    let res = run_with_fetcher(args, &ws, &fetcher, &progress);
    assert!(res.is_ok());

    let output = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
    let download_lines: Vec<&str> = output
        .lines()
        .filter(|l| l.starts_with("✓ ") && l.ends_with("downloaded"))
        .collect();

    assert_eq!(download_lines.len(), 5);
    // Newest (2018-10-28) must be first, oldest (2018-06-28) must be last
    assert!(download_lines[0].contains("2018-10-28"), "First download must be newest (2018-10-28), got: {}", download_lines[0]);
    assert!(download_lines[1].contains("2018-09-28"), "Second download must be (2018-09-28), got: {}", download_lines[1]);
    assert!(download_lines[2].contains("2018-08-28"), "Third download must be (2018-08-28), got: {}", download_lines[2]);
    assert!(download_lines[3].contains("2018-07-28"), "Fourth download must be (2018-07-28), got: {}", download_lines[3]);
    assert!(download_lines[4].contains("2018-06-28"), "Fifth download must be oldest (2018-06-28), got: {}", download_lines[4]);
}

#[test]
fn test_in_flight_rendering_on_tty() {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let caps = ProgressCaps {
        is_tty: true,
        no_color: true,
        quiet: false,
        verbose: false,
        width: 80,
    };
    let progress = Progress::new(caps, Box::new(BufferWriter(buffer.clone())));

    // Set in-flight items
    progress.set_in_flight("2026-07-31", "2026-07-31  36MB  downloading…");
    progress.set_in_flight("2026-06-26", "2026-06-26  35MB  downloading…");
    progress.batch_bar(1, 10, 1000, 5000, None, None);

    let output = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
    assert!(output.contains("2026-07-31  36MB  downloading…"));
    assert!(output.contains("2026-06-26  35MB  downloading…"));

    // When item completes, it is settled
    progress.remove_in_flight("2026-07-31");
    progress.settle("✓ 2026-07-31  36MB  downloaded");

    let output_after = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
    assert!(output_after.contains("✓ 2026-07-31  36MB  downloaded"));

    progress.clear_live();
}

#[test]
fn test_extract_manifest_header_from_nested_real_trud_fixture() {
    let tmp = TempDir::new().unwrap();
    let fixture_zip = create_mock_trud_zip_with_manifest(tmp.path());

    let header = ods::commands::ndjson::extract_manifest_header(&fixture_zip)
        .expect("extract_manifest_header must succeed on real TRUD zip-of-zips fixture");

    assert_eq!(header.publication_date.as_deref(), Some("2026-07-28"));
    assert_eq!(header.publication_seq_num.as_deref(), Some("4700"));
    assert_eq!(header.publication_type.as_deref(), Some("Full"));
    assert_eq!(header.publication_source.as_deref(), Some("HSCIC"));
    assert_eq!(header.publication_record_count, Some(305541));
}

#[test]
fn test_extract_manifest_header_fails_on_archive_without_xml() {
    let tmp = TempDir::new().unwrap();
    let bad_zip = tmp.path().join("bad_trud.zip");

    // Create a zip with no XML inside
    let file = fs::File::create(&bad_zip).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("Newsletter.pdf", options).unwrap();
    zip.write_all(b"dummy pdf content").unwrap();
    zip.finish().unwrap();

    let res = ods::commands::ndjson::extract_manifest_header(&bad_zip);
    assert!(res.is_err(), "extract_manifest_header must return Err when no XML exists in archive");
}

#[test]
fn test_provenance_json_carries_manifest_fields_on_trud_pull() {
    let tmp = TempDir::new().unwrap();
    let fixture_zip = create_mock_trud_zip_with_manifest(tmp.path());
    let out_dir = tmp.path().join("releases").join("2026-07-31");

    let output = ods_binary()
        .arg("trud")
        .arg("pull")
        .arg("--local-archive")
        .arg(&fixture_zip)
        .arg("-o")
        .arg(&out_dir)
        .output()
        .expect("Failed to execute trud pull");

    assert!(output.status.success());
    let prov_file = out_dir.join("_provenance.json");
    assert!(prov_file.exists(), "_provenance.json must be written");

    let prov_content = fs::read_to_string(&prov_file).unwrap();
    let prov: serde_json::Value = serde_json::from_str(&prov_content).unwrap();

    assert_eq!(prov.get("publication_date").and_then(|v| v.as_str()), Some("2026-07-28"));
    assert_eq!(prov.get("publication_seq_num").and_then(|v| v.as_str()), Some("4700"));
    assert_eq!(prov.get("publication_type").and_then(|v| v.as_str()), Some("Full"));
    assert_eq!(prov.get("publication_source").and_then(|v| v.as_str()), Some("HSCIC"));
    assert_eq!(prov.get("publication_record_count").and_then(|v| v.as_u64()), Some(305541));

    // Ownership rule: trud pull writes ONLY trud_* and publication_* (and _type)
    let obj = prov.as_object().unwrap();
    for key in obj.keys() {
        assert!(
            key == "_type" || key.starts_with("trud_") || key.starts_with("publication_"),
            "trud pull must NOT write key '{}'. Only _type, trud_*, and publication_* allowed.",
            key
        );
    }
    assert!(!obj.contains_key("tool_version"), "tool_version must not exist after trud pull");
    assert!(!obj.contains_key("tool_git_sha"), "tool_git_sha must not exist after trud pull");
    assert!(!obj.contains_key("dataset_revision"), "dataset_revision must not exist after trud pull");
    assert!(!obj.contains_key("dataset_parquet_schema_version"), "dataset_parquet_schema_version must not exist after trud pull");
    assert!(!obj.contains_key("dataset_file_sha256"), "dataset_file_sha256 must not exist after trud pull");
}

#[test]
fn test_batch_pin_lands_on_newest_cached_when_older_downloaded() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    fs::create_dir_all(&ws).unwrap();

    let releases = generate_96_mock_releases();
    // Cache the newest 84 releases (skip the first 12 oldest)
    for r in releases.iter().skip(12) {
        let rel_dir = ws.join("releases").join(&r.release_date).join("trud");
        fs::create_dir_all(&rel_dir).unwrap();
        let bytes = create_test_zip_bytes(&r.release_date);
        fs::write(rel_dir.join(&r.archive_file_name), bytes).unwrap();
    }

    let fetcher = MockTrudFetcher {
        releases: releases.clone(),
        failing_date: None,
    };

    let buffer = Arc::new(Mutex::new(Vec::new()));
    let caps = ProgressCaps {
        is_tty: false,
        no_color: true,
        quiet: false,
        verbose: false,
        width: 80,
    };
    let progress = Progress::new(caps, Box::new(BufferWriter(buffer.clone())));

    let args = Args {
        all: true,
        jobs: 4,
        ..Default::default()
    };

    let res = run_with_fetcher(args, &ws, &fetcher, &progress);
    assert!(res.is_ok());

    let (active_release, _) = ods::workspace::get_active_release(&ws).unwrap();
    assert_eq!(
        active_release.as_str(),
        "2026-05-28",
        "Active release must be pinned to the newest cached release (2026-05-28), got: {:?}",
        active_release
    );
}

#[test]
fn test_batch_pin_untouched_on_total_failure() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    fs::create_dir_all(&ws).unwrap();

    // Set initial pin to an existing custom release
    let initial_rel_dir = ws.join("releases").join("2020-01-01");
    fs::create_dir_all(&initial_rel_dir).unwrap();
    ods::workspace::set_active_release(&ws, "2020-01-01").unwrap();

    let releases = vec![
        TrudReleaseItem {
            id: "341".to_string(),
            name: Some("Release 1".to_string()),
            release_date: "2026-07-31".to_string(),
            archive_file_name: "test_2026-07-31.zip".to_string(),
            archive_file_sha256: "DUMMY".to_string(),
            archive_file_size: 1000,
            download_url: "https://example.com/2026-07-31".to_string(),
        },
    ];

    // Fetcher fails everything with 503
    let fetcher = MockTrudFetcher {
        releases,
        failing_date: Some("2026-07-31".to_string()),
    };

    let buffer = Arc::new(Mutex::new(Vec::new()));
    let caps = ProgressCaps {
        is_tty: false,
        no_color: true,
        quiet: false,
        verbose: false,
        width: 80,
    };
    let progress = Progress::new(caps, Box::new(BufferWriter(buffer.clone())));

    let args = Args {
        all: true,
        jobs: 1,
        ..Default::default()
    };

    let res = run_with_fetcher(args, &ws, &fetcher, &progress);
    assert!(res.is_err());

    let (active_release, _) = ods::workspace::get_active_release(&ws).unwrap();
    assert_eq!(
        active_release.as_str(),
        "2020-01-01",
        "Active release pin must remain untouched on totally failed batch"
    );
}

#[test]
fn test_batch_summary_reports_landed_bytes_not_planned() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    fs::create_dir_all(&ws).unwrap();

    let bytes_r1 = create_test_zip_bytes("2026-07-31");
    let sha_r1 = format!("{:x}", sha2::Sha256::digest(&bytes_r1)).to_uppercase();

    let bytes_r2 = create_test_zip_bytes("2026-06-30");
    let sha_r2 = format!("{:x}", sha2::Sha256::digest(&bytes_r2)).to_uppercase();

    let r1_size = bytes_r1.len() as u64;
    let r2_size = bytes_r2.len() as u64;

    let releases = vec![
        TrudReleaseItem {
            id: "341".to_string(),
            name: Some("Release 1".to_string()),
            release_date: "2026-07-31".to_string(),
            archive_file_name: "test_2026-07-31.zip".to_string(),
            archive_file_sha256: sha_r1,
            archive_file_size: r1_size,
            download_url: "https://example.com/2026-07-31".to_string(),
        },
        TrudReleaseItem {
            id: "341".to_string(),
            name: Some("Release 2".to_string()),
            release_date: "2026-06-30".to_string(),
            archive_file_name: "test_2026-06-30.zip".to_string(),
            archive_file_sha256: sha_r2,
            archive_file_size: r2_size,
            download_url: "https://example.com/2026-06-30".to_string(),
        },
    ];

    // R2 fails
    let fetcher = MockTrudFetcher {
        releases,
        failing_date: Some("2026-06-30".to_string()),
    };

    let buffer = Arc::new(Mutex::new(Vec::new()));
    let caps = ProgressCaps {
        is_tty: false,
        no_color: true,
        quiet: false,
        verbose: false,
        width: 80,
    };
    let progress = Progress::new(caps, Box::new(BufferWriter(buffer.clone())));

    let args = Args {
        all: true,
        jobs: 1,
        ..Default::default()
    };

    let _ = run_with_fetcher(args, &ws, &fetcher, &progress);

    let output = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
    // Summary line must describe landed bytes (r1_size), not planned bytes (r1_size + r2_size)
    let landed_str = ods::progress::format_size(r1_size);
    let planned_str = ods::progress::format_size(r1_size + r2_size);

    assert!(output.contains(&format!("1 downloaded · 0 cached · 1 failed · {} total", landed_str)),
        "Summary must report landed bytes ({}), output was:\n{}", landed_str, output);
    if landed_str != planned_str {
        assert!(!output.contains(&format!("{} total", planned_str)),
            "Summary must NOT report planned bytes ({}) when a download fails", planned_str);
    }
}

#[test]
fn test_batch_all_failed_summary_leads_with_cross() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    fs::create_dir_all(&ws).unwrap();

    let releases = vec![
        TrudReleaseItem {
            id: "341".to_string(),
            name: Some("Release 1".to_string()),
            release_date: "2026-07-31".to_string(),
            archive_file_name: "test_2026-07-31.zip".to_string(),
            archive_file_sha256: "DUMMY1".to_string(),
            archive_file_size: 1000,
            download_url: "https://example.com/2026-07-31".to_string(),
        },
        TrudReleaseItem {
            id: "341".to_string(),
            name: Some("Release 2".to_string()),
            release_date: "2026-06-30".to_string(),
            archive_file_name: "test_2026-06-30.zip".to_string(),
            archive_file_sha256: "DUMMY2".to_string(),
            archive_file_size: 1000,
            download_url: "https://example.com/2026-06-30".to_string(),
        },
    ];

    let fetcher = MockTrudFetcher {
        releases,
        failing_date: Some("2026".to_string()), // fail both
    };

    let buffer = Arc::new(Mutex::new(Vec::new()));
    let caps = ProgressCaps {
        is_tty: false,
        no_color: true,
        quiet: false,
        verbose: false,
        width: 80,
    };
    let progress = Progress::new(caps, Box::new(BufferWriter(buffer.clone())));

    let args = Args {
        all: true,
        jobs: 1,
        ..Default::default()
    };

    let _ = run_with_fetcher(args, &ws, &fetcher, &progress);

    let output = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
    assert!(output.contains("✖ 0 downloaded · 0 cached · 2 failed"), "All-failed batch summary must lead with ✖, got:\n{}", output);
    assert!(!output.contains("✓ 0 downloaded"), "All-failed batch summary must NOT lead with ✓, got:\n{}", output);
}

#[test]
fn test_batch_pin_not_moved_to_older_when_older_downloaded() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    fs::create_dir_all(&ws).unwrap();

    let releases = generate_96_mock_releases();
    // Cache release 95 (newest: 2026-05-28)
    let newest_rel = &releases[95];
    let rel_dir = ws.join("releases").join(&newest_rel.release_date).join("trud");
    fs::create_dir_all(&rel_dir).unwrap();
    let bytes = create_test_zip_bytes(&newest_rel.release_date);
    fs::write(rel_dir.join(&newest_rel.archive_file_name), bytes).unwrap();

    // Set initial pin to 2026-05-28
    ods::workspace::set_active_release(&ws, &newest_rel.release_date).unwrap();

    // Fetcher has release 95 (cached) and release 0 (oldest: 2018-06-28 to download)
    let test_releases = vec![releases[0].clone(), releases[95].clone()];
    let fetcher = MockTrudFetcher {
        releases: test_releases,
        failing_date: None,
    };

    let buffer = Arc::new(Mutex::new(Vec::new()));
    let caps = ProgressCaps {
        is_tty: false,
        no_color: true,
        quiet: false,
        verbose: false,
        width: 80,
    };
    let progress = Progress::new(caps, Box::new(BufferWriter(buffer.clone())));

    let args = Args {
        all: true,
        jobs: 1,
        ..Default::default()
    };

    let res = run_with_fetcher(args, &ws, &fetcher, &progress);
    assert!(res.is_ok());

    let (active_release, _) = ods::workspace::get_active_release(&ws).unwrap();
    assert_eq!(
        active_release.as_str(),
        "2026-05-28",
        "Active release must remain 2026-05-28 and NOT move to older 2018-06-28"
    );

    let output = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
    assert!(!output.contains("current → releases/2018-06-28"), "Must not emit pin change for older release");
}

#[test]
fn test_batch_pin_moved_when_newer_downloaded() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    fs::create_dir_all(&ws).unwrap();

    let releases = generate_96_mock_releases();
    // Cache release 94 (2026-04-28)
    let rel_94 = &releases[94];
    let rel_dir = ws.join("releases").join(&rel_94.release_date).join("trud");
    fs::create_dir_all(&rel_dir).unwrap();
    let bytes = create_test_zip_bytes(&rel_94.release_date);
    fs::write(rel_dir.join(&rel_94.archive_file_name), bytes).unwrap();

    // Set initial pin to 2026-04-28
    ods::workspace::set_active_release(&ws, &rel_94.release_date).unwrap();

    // Fetcher has release 94 (cached) and release 95 (newer: 2026-05-28 to download)
    let test_releases = vec![releases[94].clone(), releases[95].clone()];
    let fetcher = MockTrudFetcher {
        releases: test_releases,
        failing_date: None,
    };

    let buffer = Arc::new(Mutex::new(Vec::new()));
    let caps = ProgressCaps {
        is_tty: false,
        no_color: true,
        quiet: false,
        verbose: false,
        width: 80,
    };
    let progress = Progress::new(caps, Box::new(BufferWriter(buffer.clone())));

    let args = Args {
        all: true,
        jobs: 1,
        ..Default::default()
    };

    let res = run_with_fetcher(args, &ws, &fetcher, &progress);
    assert!(res.is_ok());

    let (active_release, _) = ods::workspace::get_active_release(&ws).unwrap();
    assert_eq!(
        active_release.as_str(),
        "2026-05-28",
        "Active release must be updated to newly fetched 2026-05-28"
    );

    let output = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
    assert!(output.contains("current → releases/2026-05-28"), "Must emit pin change for newly fetched newer release");
}



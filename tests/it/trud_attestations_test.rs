use crate::common;

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

use ods::commands::fetch::{
    run_with_fetcher, Args, TrudApiResponse, TrudFetcher, TrudReleaseItem,
};
use ods::progress::{Progress, ProgressCaps};

struct BufferWriter(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for BufferWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn create_test_zip(path: &Path, date_str: &str) {
    let file = fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::from_date_and_time(2026, 1, 1, 0, 0, 0).unwrap());
    zip.start_file(
        format!("trud_hscorgrefdataxml_data_7.0.0_{}000001.xml", date_str.replace('-', "")),
        options,
    )
    .unwrap();
    use std::io::Write;
    zip.write_all(format!("<hsctradingpartnerdesc date=\"{}\"/>", date_str).as_bytes())
        .unwrap();
    zip.finish().unwrap();
}

struct MockAttestationFetcher {
    releases: Vec<TrudReleaseItem>,
    raw_json: Option<String>,
}

impl TrudFetcher for MockAttestationFetcher {
    fn fetch_releases(&self) -> anyhow::Result<Vec<TrudReleaseItem>> {
        Ok(self.releases.clone())
    }

    fn releases_raw_json(&self) -> Option<String> {
        self.raw_json.clone()
    }

    fn download_archive(
        &self,
        _url: &str,
        dest_path: &Path,
        on_bytes: &(dyn Fn(u64) + Send + Sync),
    ) -> anyhow::Result<()> {
        let date = dest_path
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .unwrap_or("2026-07-31");
        create_test_zip(dest_path, date);
        let len = fs::metadata(dest_path)?.len();
        on_bytes(len);
        Ok(())
    }

    fn download_file(&self, url: &str, dest_path: &Path) -> anyhow::Result<()> {
        let content = if url.ends_with(".xml") {
            b"<FCIV><FILE_ENTRY><name>archive.zip</name></FILE_ENTRY></FCIV>".to_vec()
        } else if url.ends_with(".asc") || url.ends_with(".sig") {
            b"-----BEGIN PGP SIGNATURE-----\nmock\n-----END PGP SIGNATURE-----".to_vec()
        } else if url.ends_with(".pgp") {
            b"-----BEGIN PGP PUBLIC KEY BLOCK-----\nmock\n-----END PGP PUBLIC KEY BLOCK-----".to_vec()
        } else {
            b"mock".to_vec()
        };
        fs::write(dest_path, content)?;
        Ok(())
    }
}

#[test]
fn test_deserialize_attestation_fields_from_fixture() {
    let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/trud_releases_response.json");
    let json_text = fs::read_to_string(&fixture_path).expect("fixture must exist");
    let resp: TrudApiResponse = serde_json::from_str(&json_text).expect("must parse");

    assert_eq!(resp.releases.len(), 96);
    for r in &resp.releases {
        assert!(r.checksum_file_url.is_some(), "checksum_file_url missing for {}", r.release_date);
        assert!(r.checksum_file_name.is_some(), "checksum_file_name missing for {}", r.release_date);
        assert!(r.signature_file_url.is_some(), "signature_file_url missing for {}", r.release_date);
        assert!(r.signature_file_name.is_some(), "signature_file_name missing for {}", r.release_date);
        assert!(r.public_key_file_url.is_some(), "public_key_file_url missing for {}", r.release_date);
        assert!(r.public_key_file_name.is_some(), "public_key_file_name missing for {}", r.release_date);
    }
}

#[test]
fn test_trud_pull_captures_all_attestations_offline() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");

    let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/trud_releases_response.json");
    let json_text = fs::read_to_string(&fixture_path).unwrap();
    let resp: TrudApiResponse = serde_json::from_str(&json_text).unwrap();

    let mut target_release = resp.releases[0].clone();
    // Use test zip hash
    let dummy_zip_path = tmp.path().join("dummy.zip");
    create_test_zip(&dummy_zip_path, &target_release.release_date);
    let real_sha = ods::provenance::compute_file_sha256(&dummy_zip_path).unwrap();
    let real_size = fs::metadata(&dummy_zip_path).unwrap().len();
    target_release.archive_file_sha256 = real_sha;
    target_release.archive_file_size = real_size;

    let fetcher = MockAttestationFetcher {
        releases: vec![target_release.clone()],
        raw_json: Some(json_text),
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
        release_date: Some("2026-07-31".to_string()),
        ..Default::default()
    };

    let res = run_with_fetcher(args, &ws, &fetcher, &progress);
    assert!(res.is_ok(), "Pull must succeed: {:?}", res.err());

    let trud_dir = ws.join("releases/2026-07-31/trud");
    assert!(trud_dir.join("hscorgrefdataxml_data_7.0.0_20260731000001.zip").exists());
    assert!(trud_dir.join("trud_hscorgrefdataxml_data_7.0.0_20260731000001.xml").exists());
    assert!(trud_dir.join("trud_hscorgrefdataxml_data_7.0.0_20260731000001.xml.asc").exists());
    assert!(trud_dir.join("trud-public-key-2013-04-01.pgp").exists());
    // R4: `trud/` holds what NHS published, the zip, and ods's own pull record
    // (`trud/datapackage.json`), never any other listing saved from the pull.
    let mut names: Vec<String> = fs::read_dir(&trud_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "datapackage.json",
            "hscorgrefdataxml_data_7.0.0_20260731000001.zip",
            "trud-public-key-2013-04-01.pgp",
            "trud_hscorgrefdataxml_data_7.0.0_20260731000001.xml",
            "trud_hscorgrefdataxml_data_7.0.0_20260731000001.xml.asc",
        ]
    );

    let out = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
    assert!(out.contains("5 files"), "Output was: {}", out);
}

#[test]
fn test_trud_pull_handles_missing_signature_gracefully() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");

    let dummy_zip_path = tmp.path().join("dummy.zip");
    create_test_zip(&dummy_zip_path, "2019-03-29");
    let real_sha = ods::provenance::compute_file_sha256(&dummy_zip_path).unwrap();
    let real_size = fs::metadata(&dummy_zip_path).unwrap().len();

    let release = TrudReleaseItem {
        id: "341".to_string(),
        name: Some("Release 1.0.0".to_string()),
        release_date: "2019-03-29".to_string(),
        archive_file_name: "hscorgrefdataxml_data_1.0.0_20190329000001.zip".to_string(),
        archive_file_sha256: real_sha,
        archive_file_size: real_size,
        download_url: "https://example.com/test.zip".to_string(),
        checksum_file_url: Some("https://example.com/test.xml".to_string()),
        checksum_file_name: Some("test.xml".to_string()),
        signature_file_url: None, // signature missing!
        signature_file_name: None,
        public_key_file_url: Some("https://example.com/pub.pgp".to_string()),
        public_key_file_name: Some("pub.pgp".to_string()),
    };

    let fetcher = MockAttestationFetcher {
        releases: vec![release],
        raw_json: Some("{\"releases\":[]}".to_string()),
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
        release_date: Some("2019-03-29".to_string()),
        ..Default::default()
    };

    let res = run_with_fetcher(args, &ws, &fetcher, &progress);
    assert!(res.is_ok(), "Must succeed even without signature: {:?}", res.err());

    let out = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
    assert!(out.contains("4 files"), "Output was: {}", out);
}

#[test]
fn test_trud_pull_local_archive_states_no_attestations() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    let zip_path = tmp.path().join("hscorgrefdataxml_data_7.0.0_20260731000001.zip");
    create_test_zip(&zip_path, "2026-07-31");

    let buffer = Arc::new(Mutex::new(Vec::new()));
    let caps = ProgressCaps {
        is_tty: false,
        no_color: true,
        quiet: false,
        verbose: false,
        width: 80,
    };
    let progress = Progress::new(caps, Box::new(BufferWriter(buffer.clone())));

    let index_path = common::write_index_for_zip(tmp.path(), "2026-07-31", &zip_path);

    let args = Args {
        local_archive: Some(zip_path),
        workspace: Some(ws),
        index: Some(index_path.to_str().unwrap().to_string()),
        ..Default::default()
    };

    let res = ods::commands::fetch::run_local_archive_with_progress(&args, args.workspace.as_ref().unwrap(), args.local_archive.as_ref().unwrap(), &progress);
    assert!(res.is_ok(), "{:?}", res.err());

    let out = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
    assert!(out.contains("attestations: none — no API response to fetch them from"), "Output was: {}", out);
}

#[test]
fn test_provenance_json_untouched_by_attestations() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");

    let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/trud_releases_response.json");
    let json_text = fs::read_to_string(&fixture_path).unwrap();
    let resp: TrudApiResponse = serde_json::from_str(&json_text).unwrap();

    let mut target_release = resp.releases[0].clone();
    let dummy_zip_path = tmp.path().join("dummy.zip");
    create_test_zip(&dummy_zip_path, &target_release.release_date);
    let real_sha = ods::provenance::compute_file_sha256(&dummy_zip_path).unwrap();
    let real_size = fs::metadata(&dummy_zip_path).unwrap().len();
    target_release.archive_file_sha256 = real_sha;
    target_release.archive_file_size = real_size;

    let fetcher = MockAttestationFetcher {
        releases: vec![target_release.clone()],
        raw_json: Some(json_text),
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
        release_date: Some("2026-07-31".to_string()),
        ..Default::default()
    };

    let res = run_with_fetcher(args, &ws, &fetcher, &progress);
    assert!(res.is_ok());

    let prov_path = ods::provenance::pull_record_path(&ws.join("releases/2026-07-31"));
    assert!(prov_path.exists());
    let prov_text = fs::read_to_string(prov_path).unwrap();
    let prov: serde_json::Value = serde_json::from_str(&prov_text).unwrap();

    // The pull record lists NHS's files by name, hash and size, and never TRUD's URLs
    assert_eq!(prov["$schema"], ods::datapackage::DATAPACKAGE_SCHEMA_V1_URL);
    assert_eq!(prov["version"], "2026-07-31");
    assert!(!prov_text.contains("FileUrl") && !prov_text.contains("/download/"), "no TRUD URL: {prov_text}");
    let names: Vec<&str> = prov["resources"].as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["archive", "checksum", "signature", "key"]);
}

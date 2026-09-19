use anyhow::Result;
use ods::commands::cite::Args as CiteArgs;
use ods::provenance::{compute_file_sha256, OdsProvenance, PROVENANCE_FILENAME};
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use tempfile::TempDir;

fn setup_test_release_for_cite(withdrawn_reason: Option<&str>) -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let rel_dir = tmp.path().join("releases").join("2026-08-31");
    let trud_dir = rel_dir.join("trud");
    fs::create_dir_all(&trud_dir).unwrap();

    let outer_zip_path = trud_dir.join("hscorgrefdataxml_data_7.0.0_20260831000001.zip");
    {
        let outer_file = File::create(&outer_zip_path).unwrap();
        let mut outer_zip = zip::ZipWriter::new(outer_file);
        let options = zip::write::SimpleFileOptions::default();
        outer_zip.start_file("dummy.txt", options).unwrap();
        outer_zip.write_all(b"dummy source zip").unwrap();
        outer_zip.finish().unwrap();
    }
    let zip_sha256 = compute_file_sha256(&outer_zip_path).unwrap();

    fs::write(rel_dir.join("orgs.parquet"), b"dummy orgs content").unwrap();

    let mut prov = OdsProvenance::default();
    prov.trud_release_name = Some("Release 7.0.0".to_string());
    prov.trud_release_date = Some("2026-08-31".to_string());
    prov.trud_release_file = Some("hscorgrefdataxml_data_7.0.0_20260831000001.zip".to_string());
    prov.trud_release_filesize_bytes = Some(37_983_173);
    prov.trud_release_sha256 = Some(zip_sha256.clone());
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);
    prov.publication_date = Some("2026-08-28".to_string());
    prov.publication_seq_num = Some("4700".to_string());
    prov.publication_type = Some("Full".to_string());
    prov.publication_record_count = Some(2);
    prov.tool_version = Some("0.4.3".to_string());
    prov.tool_git_sha = Some("ab4332f4d75bfdc01814e03458d9dc4db20494cb".to_string());
    prov.tool_git_dirty = Some(false);
    prov.dataset_version = Some("1.0.1".to_string());

    let prov_path = rel_dir.join(PROVENANCE_FILENAME);
    fs::write(&prov_path, serde_json::to_string_pretty(&prov).unwrap()).unwrap();

    let (manifest, _) = ods::commands::make_oci::build_manifest_from_dir(&rel_dir, &prov, "1.0.1").unwrap();
    let manifest_digest = manifest.digest().unwrap();

    let release_entry = ods::index::ReleaseIndexEntry {
        trud_release_date: "2026-08-31".to_string(),
        dataset_version: "1.0.1".to_string(),
        tag: "2026-08-31_1.0.1".to_string(),
        manifest_digest,
        trud_release_sha256: zip_sha256,
        tool_version: "0.4.3".to_string(),
        dataset_doi: None,
        withdrawn: withdrawn_reason.map(|s| s.to_string()),
    };

    let index = ods::index::OdsReleaseIndex {
        type_tag: "ods_release_index".to_string(),
        index_version: 2,
        concept_doi: None,
        mirrors: vec![],
        releases: vec![release_entry],
    };
    let index_bytes = serde_json::to_vec_pretty(&index).unwrap();
    ods::index::OdsReleaseIndex::save_to_workspace_bytes(&index_bytes, tmp.path()).unwrap();

    (tmp, rel_dir)
}

struct MockCiteFetcher {
    pub remote_index: Option<ods::index::OdsReleaseIndex>,
}

impl ods::commands::pull::OciBlobFetcher for MockCiteFetcher {
    fn fetch_bytes(&self, _url: &str) -> Result<Vec<u8>> {
        anyhow::bail!("fetch_bytes not implemented for mock")
    }

    fn fetch_release_index(&self) -> Result<Option<ods::index::OdsReleaseIndex>> {
        Ok(self.remote_index.clone())
    }
}

#[test]
fn test_cite_all_formats_output_dataset_version_and_manifest_digest() -> Result<()> {
    let (tmp, rel_dir) = setup_test_release_for_cite(None);
    let fetcher = MockCiteFetcher { remote_index: None };

    // 1. Text format
    let mut text_buf = Vec::new();
    ods::commands::cite::run_with_writer_and_fetcher(
        CiteArgs {
            format: "text".to_string(),
            input: Some(rel_dir.clone()),
        },
        &mut text_buf,
        &fetcher,
        tmp.path(),
    )?;
    let text_out = String::from_utf8(text_buf)?;
    assert!(text_out.contains("Dataset version:    v1.0.1"));
    assert!(text_out.contains("Manifest digest:    sha256:"));
    assert!(text_out.contains("ods: NHS Organisation Data as verifiable Parquet files,"));
    assert!(text_out.contains("ods (Version 1.0.1) [Computer software]"));
    assert!(text_out.contains("NHS Organisation Data Service XML Data, release 2026-08-31"));
    assert!(!text_out.contains("Dataset DOI:")); // DOI absent, no fallback

    // 2. BibTeX format
    let mut bib_buf = Vec::new();
    ods::commands::cite::run_with_writer_and_fetcher(
        CiteArgs {
            format: "bibtex".to_string(),
            input: Some(rel_dir.clone()),
        },
        &mut bib_buf,
        &fetcher,
        tmp.path(),
    )?;
    let bib_out = String::from_utf8(bib_buf)?;
    assert!(bib_out.contains("@misc{ods-2026-08-31-v1.0.1,"));
    assert!(bib_out.contains("title = {ods: NHS Organisation Data as verifiable Parquet files, release 2026-08-31}"));
    assert!(bib_out.contains("title = {ods}"));
    assert!(bib_out.contains("howpublished = {Computer software}"));
    assert!(bib_out.contains("title = {NHS Organisation Data Service XML Data, release 2026-08-31}"));
    assert!(bib_out.contains("version = {1.0.1}"));
    assert!(bib_out.contains("note = {Manifest: sha256:"));
    assert!(!bib_out.contains("doi =")); // DOI absent, no fallback

    // 3. APA format
    let mut apa_buf = Vec::new();
    ods::commands::cite::run_with_writer_and_fetcher(
        CiteArgs {
            format: "apa".to_string(),
            input: Some(rel_dir.clone()),
        },
        &mut apa_buf,
        &fetcher,
        tmp.path(),
    )?;
    let apa_out = String::from_utf8(apa_buf)?;
    assert!(apa_out.contains("ods: NHS Organisation Data as verifiable Parquet files, release 2026-08-31"));
    assert!(apa_out.contains("ods (Version 1.0.1) [Computer software]"));
    assert!(apa_out.contains("NHS Organisation Data Service XML Data, release 2026-08-31"));
    assert!(!apa_out.contains("https://doi.org/"));

    // 4. CSL-JSON format
    let mut csl_buf = Vec::new();
    ods::commands::cite::run_with_writer_and_fetcher(
        CiteArgs {
            format: "csljson".to_string(),
            input: Some(rel_dir.clone()),
        },
        &mut csl_buf,
        &fetcher,
        tmp.path(),
    )?;
    let csl_out = String::from_utf8(csl_buf)?;
    assert!(csl_out.contains("\"title\": \"ods: NHS Organisation Data as verifiable Parquet files, release 2026-08-31\""));
    assert!(csl_out.contains("\"title\": \"ods\""));
    assert!(csl_out.contains("\"type\": \"software\""));
    assert!(csl_out.contains("\"title\": \"NHS Organisation Data Service XML Data, release 2026-08-31\""));
    assert!(csl_out.contains("\"version\": \"1.0.1\""));
    assert!(csl_out.contains("\"note\": \"Manifest: sha256:"));
    assert!(!csl_out.contains("\"DOI\":"));

    Ok(())
}

#[test]
fn test_cite_refuses_when_release_is_withdrawn() {
    let (tmp, rel_dir) = setup_test_release_for_cite(Some(
        "roles table truncated at 65535 rows by a bad build",
    ));
    let fetcher = MockCiteFetcher { remote_index: None };

    let mut buf = Vec::new();
    let res = ods::commands::cite::run_with_writer_and_fetcher(
        CiteArgs {
            format: "text".to_string(),
            input: Some(rel_dir),
        },
        &mut buf,
        &fetcher,
        tmp.path(),
    );

    assert!(res.is_err());
    let err = res.unwrap_err().to_string();
    assert!(err.contains("Refusing to cite 2026-08-31 v1.0.1"));
    assert!(err.contains("roles table truncated at 65535 rows by a bad build"));
    assert!(err.contains("Update to a valid release: ods pull 2026-08-31"));
}

#[test]
fn test_pull_refuses_when_release_is_withdrawn() {
    let json = r#"{
      "_type": "ods_release_index",
      "index_version": 2,
      "mirrors": [],
      "releases": [
        {
          "trud_release_date": "2026-07-31",
          "dataset_version": "1.0.0",
          "tag": "2026-07-31_1.0.0",
          "manifest_digest": "sha256:0f2a000000000000000000000000000000000000000000000000000000000000",
          "trud_release_sha256": "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
          "tool_version": "0.4.3",
          "withdrawn": "roles table truncated at 65535 rows by a bad build"
        }
      ]
    }"#;
    let index: ods::index::OdsReleaseIndex = serde_json::from_str(json).unwrap();

    let res = index.resolve(Some("2026-07-31"));
    assert!(res.is_err());
    let err = res.unwrap_err().to_string();
    assert!(err.contains("2026-07-31 has no valid release"));
    assert!(err.contains("1.0.0 was withdrawn: roles table truncated at 65535 rows by a bad build"));
}

#[test]
fn test_cite_offline_cached_index_disclosure() -> Result<()> {
    let (tmp, rel_dir) = setup_test_release_for_cite(None);

    let fetcher = MockCiteFetcher {
        remote_index: None,
    };

    let mut buf = Vec::new();
    ods::commands::cite::run_with_writer_and_fetcher(
        CiteArgs {
            format: "text".to_string(),
            input: Some(rel_dir),
        },
        &mut buf,
        &fetcher,
        tmp.path(),
    )?;

    let out = String::from_utf8(buf)?;
    assert!(out.contains("* Source: releases/2026-08-31 (1.0.1)"));

    Ok(())
}

struct RequestRecordingFetcher {
    pub requests: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl ods::commands::pull::OciBlobFetcher for RequestRecordingFetcher {
    fn fetch_bytes(&self, _url: &str) -> Result<Vec<u8>> {
        self.requests.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        anyhow::bail!("fetch_bytes should not be called")
    }

    fn fetch_release_index(&self) -> Result<Option<ods::index::OdsReleaseIndex>> {
        self.requests.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(None)
    }

    fn fetch_release_index_raw(&self) -> Result<Option<Vec<u8>>> {
        self.requests.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(None)
    }
}

#[test]
fn test_cite_reads_workspace_index_and_never_fetches_or_writes() -> Result<()> {
    // Run from a temp directory holding ods_data/
    let (tmp, rel_dir) = setup_test_release_for_cite(None);
    let ws_root = tmp.path().join("ods_data");
    fs::create_dir_all(ws_root.join("releases"))?;

    let moved_rel = ws_root.join("releases").join("2026-08-31");
    fs::rename(&rel_dir, &moved_rel)?;

    let index_file = ws_root.join("_releases.json");
    let initial_index_bytes = fs::read(tmp.path().join("_releases.json"))?;
    fs::write(&index_file, &initial_index_bytes)?;
    let _ = fs::remove_file(tmp.path().join("_releases.json"));

    let mtime_before = fs::metadata(&index_file)?.modified()?;

    let requests = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let fetcher = RequestRecordingFetcher {
        requests: requests.clone(),
    };

    let mut buf = Vec::new();
    let res = ods::commands::cite::run_with_writer_and_fetcher(
        CiteArgs {
            format: "text".to_string(),
            input: Some(moved_rel),
        },
        &mut buf,
        &fetcher,
        tmp.path(),
    );

    res?;

    assert_eq!(
        requests.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "ods cite must make zero network requests"
    );
    assert!(
        !tmp.path().join("_releases.json").exists(),
        "ods cite must NOT create _releases.json in parent directory"
    );

    let mtime_after = fs::metadata(&index_file)?.modified()?;
    assert_eq!(
        mtime_before, mtime_after,
        "_releases.json mtime must remain unchanged"
    );

    let current_bytes = fs::read(&index_file)?;
    assert_eq!(
        initial_index_bytes, current_bytes,
        "_releases.json content must remain byte-identical"
    );

    Ok(())
}

#[test]
fn test_cite_honours_withdrawn_release_from_cached_workspace_index() -> Result<()> {
    let tmp = TempDir::new()?;
    let ws_root = tmp.path().join("ods_data");
    let rel_dir = ws_root.join("releases").join("2026-08-31");
    let trud_dir = rel_dir.join("trud");
    fs::create_dir_all(&trud_dir)?;

    let outer_zip_path = trud_dir.join("hscorgrefdataxml_data_7.0.0_20260831000001.zip");
    {
        let outer_file = File::create(&outer_zip_path)?;
        let mut outer_zip = zip::ZipWriter::new(outer_file);
        let options = zip::write::SimpleFileOptions::default();
        outer_zip.start_file("dummy.txt", options)?;
        outer_zip.write_all(b"dummy source zip")?;
        outer_zip.finish()?;
    }
    let zip_sha256 = compute_file_sha256(&outer_zip_path)?;

    fs::write(rel_dir.join("orgs.parquet"), b"dummy orgs content")?;

    let mut prov = OdsProvenance::default();
    prov.trud_release_name = Some("Release 7.0.0".to_string());
    prov.trud_release_date = Some("2026-08-31".to_string());
    prov.trud_release_file = Some("hscorgrefdataxml_data_7.0.0_20260831000001.zip".to_string());
    prov.trud_release_filesize_bytes = Some(37_983_173);
    prov.trud_release_sha256 = Some(zip_sha256.clone());
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);
    prov.publication_date = Some("2026-08-28".to_string());
    prov.publication_seq_num = Some("4700".to_string());
    prov.publication_type = Some("Full".to_string());
    prov.publication_record_count = Some(2);
    prov.tool_version = Some("0.4.3".to_string());
    prov.tool_git_sha = Some("ab4332f4d75bfdc01814e03458d9dc4db20494cb".to_string());
    prov.tool_git_dirty = Some(false);
    prov.dataset_version = Some("1.0.1".to_string());

    let prov_path = rel_dir.join(PROVENANCE_FILENAME);
    fs::write(&prov_path, serde_json::to_string_pretty(&prov)?)?;

    let (manifest, _) = ods::commands::make_oci::build_manifest_from_dir(&rel_dir, &prov, "1.0.1")?;
    let manifest_digest = manifest.digest()?;

    let release_entry = ods::index::ReleaseIndexEntry {
        trud_release_date: "2026-08-31".to_string(),
        dataset_version: "1.0.1".to_string(),
        tag: "2026-08-31_1.0.1".to_string(),
        manifest_digest,
        trud_release_sha256: zip_sha256,
        tool_version: "0.4.3".to_string(),
        dataset_doi: None,
        withdrawn: Some("critical schema defect discovered in release".to_string()),
    };

    let index = ods::index::OdsReleaseIndex {
        type_tag: "ods_release_index".to_string(),
        index_version: 2,
        concept_doi: None,
        mirrors: vec![],
        releases: vec![release_entry],
    };
    let index_bytes = serde_json::to_vec_pretty(&index)?;
    ods::index::OdsReleaseIndex::save_to_workspace_bytes(&index_bytes, &ws_root)?;

    let mut buf = Vec::new();
    let res = ods::commands::cite::run_with_writer(
        CiteArgs {
            format: "text".to_string(),
            input: Some(rel_dir),
        },
        &mut buf,
    );

    assert!(res.is_err(), "cite must refuse withdrawn release");
    let err = res.unwrap_err().to_string();
    assert!(err.contains("This release was withdrawn: critical schema defect discovered in release"));
    assert!(err.contains("Update to a valid release: ods pull 2026-08-31"));

    Ok(())
}

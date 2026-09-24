mod common;

use common::{make_v1_index, setup_test_release_for_cite};
use anyhow::Result;
use ods::commands::cite::Args as CiteArgs;
use ods::provenance::{compute_file_sha256, OdsProvenance, PROVENANCE_FILENAME};
use std::fs::{self, File};
use std::io::Write;
use tempfile::TempDir;

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
    assert!(text_out.contains("ods-data/2026-08-31_1.0.1"));
    assert!(text_out.contains("Data availability"));
    assert!(text_out.contains("sha256:"));
    assert!(text_out.contains("ods: NHS Organisation Data as verifiable Parquet files,"));
    assert!(text_out.contains("ods (Version 0.4.3) [Computer software]"));
    assert!(text_out.contains("NHS Organisation Data Service XML Data, release 2026-08-31"));
    assert!(!text_out.contains("Dataset version:"));
    assert!(!text_out.contains("Format: Apache Parquet"));
    assert!(!text_out.contains("Dataset DOI:")); // DOI absent, no fallback

    let pos_text_src = text_out.find("The source:").expect("The source: present");
    let pos_text_data = text_out.find("The data:").expect("The data: present");
    let pos_text_tool = text_out.find("The tool:").expect("The tool: present");
    assert!(pos_text_src < pos_text_data && pos_text_data < pos_text_tool);

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
    assert!(bib_out.contains("@misc{ods-data/2026-08-31_1.0.1,"));
    assert!(bib_out.contains("title = {ods: NHS Organisation Data as verifiable Parquet files, release 2026-08-31}"));
    assert!(bib_out.contains("title = {ods}"));
    assert!(bib_out.contains("howpublished = {Computer software}"));
    assert!(bib_out.contains("title = {NHS Organisation Data Service XML Data, release 2026-08-31}"));
    assert!(bib_out.contains("version = {1.0.1}"));
    assert!(bib_out.contains("note = {Release 2026-08-31, SHA-256 "));
    assert!(!bib_out.contains("doi =")); // DOI absent, no fallback

    let pos_bib_src = bib_out.find("@misc{nhs-ods-xml/").expect("source key present");
    let pos_bib_data = bib_out.find("@misc{ods-data/").expect("data key present");
    let pos_bib_tool = bib_out.find("@misc{ods/v").expect("tool key present");
    assert!(pos_bib_src < pos_bib_data && pos_bib_data < pos_bib_tool);

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
    assert!(apa_out.contains("ods (Version 0.4.3) [Computer software]"));
    assert!(apa_out.contains("NHS Organisation Data Service XML Data, release 2026-08-31"));
    assert!(!apa_out.contains("https://doi.org/"));

    let pos_apa_src = apa_out.find("NHS England.").expect("source present");
    let pos_apa_data = apa_out.find("Evans, O. (2026). ods: NHS").expect("data present");
    let pos_apa_tool = apa_out.find("Evans, O. (2026). ods (Version").expect("tool present");
    assert!(pos_apa_src < pos_apa_data && pos_apa_data < pos_apa_tool);
    assert_eq!(apa_out.lines().filter(|l| !l.is_empty()).count(), 3);
    assert_eq!(apa_out.lines().count(), 5);

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
    assert!(csl_out.contains("\"note\": \"Release 2026-08-31, SHA-256 "));
    assert!(!csl_out.contains("\"DOI\":"));

    let pos_csl_src = csl_out.find("\"id\": \"nhs-ods-xml/").expect("src id present");
    let pos_csl_data = csl_out.find("\"id\": \"ods-data/").expect("data id present");
    let pos_csl_tool = csl_out.find("\"id\": \"ods/v").expect("tool id present");
    assert!(pos_csl_src < pos_csl_data && pos_csl_data < pos_csl_tool);

    Ok(())
}

#[test]
fn test_cite_withdrawn_release_delivers_citation_and_warns() -> Result<()> {
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

    assert!(res.is_err(), "withdrawn release citation must return error exit 1");
    let err = res.unwrap_err();
    assert!(
        err.chain().any(|c| c.downcast_ref::<ods::commands::pull::AlreadyReported>().is_some()),
        "must exit via AlreadyReported"
    );

    let out = String::from_utf8(buf)?;
    assert!(out.contains("* Source: releases/2026-08-31 (1.0.1)"));
    assert!(out.contains("How to Cite"));

    Ok(())
}

#[test]
fn test_cite_withdrawn_release_bibtex_acceptance() -> Result<()> {
    let (tmp, rel_dir) = setup_test_release_for_cite(Some(
        "roles table truncated at 65535 rows by a bad build",
    ));
    let fetcher = MockCiteFetcher { remote_index: None };

    let mut buf = Vec::new();
    let res = ods::commands::cite::run_with_writer_and_fetcher(
        CiteArgs {
            format: "bibtex".to_string(),
            input: Some(rel_dir),
        },
        &mut buf,
        &fetcher,
        tmp.path(),
    );

    assert!(res.is_err(), "withdrawn release bibtex citation must return error exit 1");
    let err = res.unwrap_err();
    assert!(
        err.chain().any(|c| c.downcast_ref::<ods::commands::pull::AlreadyReported>().is_some()),
        "must exit via AlreadyReported"
    );

    let out = String::from_utf8(buf)?;
    let line_count = out.lines().count();
    assert_eq!(line_count, 29);
    assert!(out.contains("@misc{ods-data/2026-08-31_1.0.1,"));
    assert!(out.contains("@misc{ods/v0.4.3,"));
    assert!(out.contains("@misc{nhs-ods-xml/2026-08-31,"));

    Ok(())
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

    fn fetch_release_index_raw(&self) -> Result<Option<(Vec<u8>, String)>> {
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
    fs::write(rel_dir.join("datapackage.json"), b"{\"name\": \"ods\", \"version\": \"1.0.1\"}")?;
    let prov = OdsProvenance {
        trud_release_date: Some("2026-08-31".to_string()),
        trud_release_filesize_bytes: Some(37_983_173),
        trud_release_sha256: Some(zip_sha256.clone()),
        trud_release_sha256_verified: Some(ods::provenance::TrudVerificationSource::TrudApi),
        tool_version: Some("0.4.3".to_string()),
        tool_git_sha: Some("ab4332f4d75bfdc01814e03458d9dc4db20494cb".to_string()),
        tool_git_dirty: Some(false),
        ..Default::default()
    };

    let prov_path = rel_dir.join(PROVENANCE_FILENAME);
    fs::write(&prov_path, serde_json::to_string_pretty(&prov)?)?;

    let (manifest, _) = ods::commands::make_oci::build_manifest_from_dir(&rel_dir, &prov, "1.0.1")?;
    let manifest_digest = manifest.digest()?;

    let mut index = make_v1_index(&[(
        "2026-08-31",
        &zip_sha256,
        1_000_000,
        &[("1.0.1", &manifest_digest)],
    )]);
    index.releases[0].datasets[0].withdrawn =
        Some("critical schema defect discovered in release".to_string());
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

    assert!(res.is_err(), "cite must return error (exit 1) for withdrawn release");
    let err = res.unwrap_err();
    assert!(
        err.chain().any(|c| c.downcast_ref::<ods::commands::pull::AlreadyReported>().is_some()),
        "must exit via AlreadyReported"
    );

    let out = String::from_utf8(buf)?;
    assert!(out.contains("* Source: releases/2026-08-31 (1.0.1)"));
    assert!(out.contains("How to Cite"));

    Ok(())
}

#[test]
fn test_cite_with_invalid_workspace_marker_stops_command() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ods_data");
    let rel_dir = ws.join("releases").join("2026-08-31");
    fs::create_dir_all(&rel_dir).unwrap();

    let prov = OdsProvenance {
        trud_release_date: Some("2026-08-31".to_string()),
        trud_release_sha256: Some("8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933".to_string()),
        trud_release_sha256_verified: Some(ods::provenance::TrudVerificationSource::TrudApi),
        ..Default::default()
    };
    fs::write(
        rel_dir.join(PROVENANCE_FILENAME),
        serde_json::to_string_pretty(&prov).unwrap(),
    ).unwrap();
    fs::write(rel_dir.join("orgs.parquet"), b"dummy content").unwrap();
    fs::write(rel_dir.join("datapackage.json"), b"{\"name\": \"ods\", \"version\": \"0.1.0\"}").unwrap();

    // Invalid marker in enclosing workspace
    let marker_path = ws.join("_releases.json");
    fs::write(&marker_path, b"{\"bad\":\"marker\"}").unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ods"))
        .current_dir(tmp.path())
        .args(["cite", "-i", rel_dir.to_str().unwrap()])
        .output()
        .expect("execute ods cite");

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    let count = stderr.matches("! Ignoring").count();
    assert_eq!(
        count, 1,
        "expected ! Ignoring notice to appear exactly once, but got {count}:\n{stderr}"
    );
    let expected_notice = format!(
        "! Ignoring {}: it isn't a release index this ods can read\n  Using the index built into ods. The next ods pull will replace it.",
        marker_path.display()
    );
    assert!(
        stderr.contains(&expected_notice),
        "stderr must contain exact two-line notice:\n{}\nGot:\n{}",
        expected_notice,
        stderr
    );
}

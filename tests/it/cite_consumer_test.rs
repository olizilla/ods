use crate::common;

use common::{make_v1_index, setup_cite_case_workspace, setup_test_release_for_cite};
use anyhow::Result;
use ods::commands::cite::Args as CiteArgs;
use ods::provenance::compute_file_sha256;
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

    common::write_fixture_parquet(&rel_dir.join("orgs.parquet"), "2026-08-31", &zip_sha256, "1.0.1", "dummy orgs content");
    // A datapackage.json view that says something else entirely: nothing reads it.
    fs::write(rel_dir.join("datapackage.json"), b"{\"name\": \"ods\", \"version\": \"9.9.9\"}")?;

    let (manifest, _) = ods::commands::make_oci::build_manifest_from_dir(&rel_dir)?;
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

    common::write_fixture_parquet(
        &rel_dir.join("orgs.parquet"),
        "2026-08-31",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        "0.1.0",
        "dummy content",
    );

    // Invalid marker in enclosing workspace
    let marker_path = ws.join("_releases.json");
    fs::write(&marker_path, b"{\"bad\":\"marker\"}").unwrap();

    let output = common::ods_cmd()
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

#[test]
fn test_cite_case_a_bibtex_omits_howpublished_and_url() -> Result<()> {
    let (tmp, rel_dir) = setup_cite_case_workspace("A");
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

    assert!(res.is_ok(), "case A cite must succeed with exit 0");

    let out = String::from_utf8(buf)?;
    assert!(out.contains("@misc{ods-data/2026-08-28_0.1.0,"));
    assert!(out.contains("title = {ods: NHS Organisation Data as verifiable Parquet files, release 2026-08-28}"));
    assert!(out.contains("version = {0.1.0}"));

    // Extract the data entry block
    let data_block = out
        .split("@misc{ods-data/2026-08-28_0.1.0,")
        .nth(1)
        .expect("data block exists")
        .split("}\n")
        .next()
        .expect("data block close");

    assert!(
        !data_block.contains("howpublished"),
        "unpublished data entry in bibtex must not contain howpublished, got:\n{}",
        data_block
    );
    assert!(
        !data_block.contains("url ="),
        "unpublished data entry in bibtex must not contain url, got:\n{}",
        data_block
    );

    // Upstream source still has howpublished and url
    assert!(out.contains("howpublished = {NHS TRUD}"));
    assert!(out.contains("url = {https://isd.digital.nhs.uk/trud}"));

    // Tool still has howpublished and url
    assert!(out.contains("howpublished = {Computer software}"));
    assert!(out.contains("url = {https://github.com/olizilla/ods}"));

    Ok(())
}

#[test]
fn test_cite_exit_codes_and_warnings_for_all_cases() {
    // Case A: exit 0
    let (tmp_a, dir_a) = setup_cite_case_workspace("A");
    let out_a = common::ods_cmd()
        .current_dir(tmp_a.path())
        .args(["cite", "-i", dir_a.to_str().unwrap()])
        .output()
        .expect("run ods cite case A");
    assert_eq!(out_a.status.code(), Some(0), "Case A must exit 0");
    let err_a = String::from_utf8_lossy(&out_a.stderr);
    assert!(err_a.contains("! 2026-08-28 isn't in the cached release index from (ods_data/_releases.json)"));
    assert!(err_a.contains("To update the release index run: ods pull"));

    // Case B: exit 1
    let (tmp_b, dir_b) = setup_cite_case_workspace("B");
    let out_b = common::ods_cmd()
        .current_dir(tmp_b.path())
        .args(["cite", "-i", dir_b.to_str().unwrap()])
        .output()
        .expect("run ods cite case B");
    assert_eq!(out_b.status.code(), Some(1), "Case B must exit 1");
    let stdout_b = String::from_utf8_lossy(&out_b.stdout);
    assert!(stdout_b.contains("How to Cite"), "Case B must deliver citation");
    let err_b = String::from_utf8_lossy(&out_b.stderr);
    assert!(err_b.contains("✖ releases/2026-08-28 was built from a different TRUD archive than the published 2026-08-28"));
    assert!(err_b.contains("this build  sha256 1111111111111111111111111111111111111111111111111111111111111111"));
    assert!(err_b.contains("published   sha256 ABDD194B1569D5FF3CDD81D618847F05642BD43C5B15D6CD43D8289B7466D801"));
    assert!(err_b.contains("Cite the published release: ods pull 2026-08-28"));

    // Case C: exit 0
    let (tmp_c, dir_c) = setup_cite_case_workspace("C");
    let out_c = common::ods_cmd()
        .current_dir(tmp_c.path())
        .args(["cite", "-i", dir_c.to_str().unwrap()])
        .output()
        .expect("run ods cite case C");
    assert_eq!(out_c.status.code(), Some(0), "Case C must exit 0");
    let stdout_c = String::from_utf8_lossy(&out_c.stdout);
    assert!(stdout_c.contains("How to Cite"), "Case C must deliver citation");
    let err_c = String::from_utf8_lossy(&out_c.stderr);
    assert!(err_c.contains("! dataset 0.3.0 was never published for 2026-08-28. Published: 0.1.0"));

    // Case D: exit 1
    let (tmp_d, dir_d) = setup_cite_case_workspace("D");
    let out_d = common::ods_cmd()
        .current_dir(tmp_d.path())
        .args(["cite", "-i", dir_d.to_str().unwrap()])
        .output()
        .expect("run ods cite case D");
    assert_eq!(out_d.status.code(), Some(1), "Case D must exit 1");
    let stdout_d = String::from_utf8_lossy(&out_d.stdout);
    assert!(!stdout_d.contains("How to Cite"), "Case D must refuse citation");
    let err_d = String::from_utf8_lossy(&out_d.stderr);
    // The files don't agree on what release they are: refused, naming each file.
    assert!(err_d.contains("✖ The Parquet files in "), "{err_d}");
    assert!(err_d.contains("don't carry the same provenance"), "{err_d}");
    assert!(err_d.contains("orgs.parquet   2026-08-28_0.1.0"), "{err_d}");
    assert!(err_d.contains("roles.parquet  2026-08-28_9.9.9"), "{err_d}");
    assert!(err_d.contains("Pull it again with `ods pull --force`"), "{err_d}");

    // Case F: exit 1
    let (tmp_f, dir_f) = setup_cite_case_workspace("F");
    let out_f = common::ods_cmd()
        .current_dir(tmp_f.path())
        .args(["cite", "-i", dir_f.to_str().unwrap()])
        .output()
        .expect("run ods cite case F");
    assert_eq!(out_f.status.code(), Some(1), "Case F must exit 1");
    let stdout_f = String::from_utf8_lossy(&out_f.stdout);
    assert!(stdout_f.contains("How to Cite"), "Case F must deliver citation");
    let err_f = String::from_utf8_lossy(&out_f.stderr);
    assert!(err_f.contains("✖ releases/2026-08-28 doesn't match the published ods-data/2026-08-28_0.1.0"));
    assert!(err_f.contains("Cite the published release: ods pull 2026-08-28"));

    // No provenance: exit 1
    let (tmp_np, dir_np) = setup_cite_case_workspace("no_prov");
    let out_np = common::ods_cmd()
        .current_dir(tmp_np.path())
        .args(["cite", "-i", dir_np.to_str().unwrap()])
        .output()
        .expect("run ods cite no provenance");
    assert_eq!(out_np.status.code(), Some(1), "No provenance must exit 1");
    let stdout_np = String::from_utf8_lossy(&out_np.stdout);
    assert!(!stdout_np.contains("How to Cite"), "No provenance must refuse citation");
    let err_np = String::from_utf8_lossy(&out_np.stderr);
    // Genuinely no provenance anywhere (not just unpacked) gets the specific wording back: `ods
    // make oci` wouldn't be a fix here, since there's nothing to pack from.
    assert!(err_np.contains("has no provenance: it was built from an archive ods couldn't match to a TRUD release"));
    assert!(err_np.contains("To cite or publish it, get the archive through ods trud pull."));
}

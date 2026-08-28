use anyhow::Result;
use ods::commands::cite::{run_with_writer as cite_run, Args as CiteArgs};
use ods::provenance::{compute_file_sha256, OdsProvenance, PROVENANCE_FILENAME};
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use tempfile::TempDir;

fn setup_test_release_for_cite(withdrawn_reason: Option<&str>) -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let rel_dir = tmp.path().join("releases").join("2026-07-31");
    let trud_dir = rel_dir.join("trud");
    fs::create_dir_all(&trud_dir).unwrap();

    let outer_zip_path = trud_dir.join("hscorgrefdataxml_data_7.0.0_20260731000001.zip");
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
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_file = Some("hscorgrefdataxml_data_7.0.0_20260731000001.zip".to_string());
    prov.trud_release_filesize_bytes = Some(37_983_173);
    prov.trud_release_sha256 = Some(zip_sha256.clone());
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);
    prov.publication_date = Some("2026-07-28".to_string());
    prov.publication_seq_num = Some("4700".to_string());
    prov.publication_type = Some("Full".to_string());
    prov.publication_record_count = Some(2);
    prov.tool_version = Some("0.4.3".to_string());
    prov.tool_git_sha = Some("ab4332f4d75bfdc01814e03458d9dc4db20494cb".to_string());
    prov.tool_git_dirty = Some(false);
    prov.dataset_version = Some("1.0.1".to_string());

    let prov_path = rel_dir.join(PROVENANCE_FILENAME);
    fs::write(&prov_path, serde_json::to_string_pretty(&prov).unwrap()).unwrap();

    // Write _release.json
    let mut release_obj = serde_json::json!({
        "trud_release_date": "2026-07-31",
        "dataset_version": "1.0.1",
        "tag": "2026-07-31_1.0.1",
        "manifest_digest": "sha256:0f2a000000000000000000000000000000000000000000000000000000000000",
        "trud_release_sha256": zip_sha256,
        "tool_version": "0.4.3"
    });
    if let Some(reason) = withdrawn_reason {
        release_obj["withdrawn"] = serde_json::json!(reason);
    }
    fs::write(
        rel_dir.join("_release.json"),
        serde_json::to_string_pretty(&release_obj).unwrap(),
    )
    .unwrap();

    (tmp, rel_dir)
}

#[test]
fn test_cite_all_formats_output_dataset_version_and_manifest_digest() -> Result<()> {
    let (_tmp, rel_dir) = setup_test_release_for_cite(None);

    // 1. Text format
    let mut text_buf = Vec::new();
    cite_run(
        CiteArgs {
            format: "text".to_string(),
            input: Some(rel_dir.clone()),
        },
        &mut text_buf,
    )?;
    let text_out = String::from_utf8(text_buf)?;
    assert!(text_out.contains("Dataset version:    v1.0.1"));
    assert!(text_out.contains("Manifest digest:    sha256:0f2a"));
    assert!(text_out.contains("NHS ODS Dataset (2026-07-31 cut, v1.0.1)"));
    assert!(!text_out.contains("Dataset DOI:")); // DOI absent, no fallback

    // 2. BibTeX format
    let mut bib_buf = Vec::new();
    cite_run(
        CiteArgs {
            format: "bibtex".to_string(),
            input: Some(rel_dir.clone()),
        },
        &mut bib_buf,
    )?;
    let bib_out = String::from_utf8(bib_buf)?;
    assert!(bib_out.contains("@misc{ods-2026-07-31-v1.0.1,"));
    assert!(bib_out.contains("title = {NHS Organisation Data Service (2026-07-31 cut, v1.0.1)}"));
    assert!(bib_out.contains("version = {1.0.1}"));
    assert!(bib_out.contains("note = {Manifest: sha256:0f2a"));
    assert!(!bib_out.contains("doi =")); // DOI absent, no fallback

    // 3. APA format
    let mut apa_buf = Vec::new();
    cite_run(
        CiteArgs {
            format: "apa".to_string(),
            input: Some(rel_dir.clone()),
        },
        &mut apa_buf,
    )?;
    let apa_out = String::from_utf8(apa_buf)?;
    assert!(apa_out.contains("Organisation Data Service (2026-07-31 cut, v1.0.1)"));
    assert!(!apa_out.contains("https://doi.org/"));

    // 4. CSL-JSON format
    let mut csl_buf = Vec::new();
    cite_run(
        CiteArgs {
            format: "csljson".to_string(),
            input: Some(rel_dir.clone()),
        },
        &mut csl_buf,
    )?;
    let csl_out = String::from_utf8(csl_buf)?;
    assert!(csl_out.contains("\"version\": \"1.0.1\""));
    assert!(csl_out.contains("\"note\": \"Manifest: sha256:0f2a"));
    assert!(!csl_out.contains("\"DOI\":"));

    Ok(())
}

#[test]
fn test_cite_refuses_when_release_is_withdrawn() {
    let (_tmp, rel_dir) = setup_test_release_for_cite(Some(
        "roles table truncated at 65535 rows by a bad build",
    ));

    let mut buf = Vec::new();
    let res = cite_run(
        CiteArgs {
            format: "text".to_string(),
            input: Some(rel_dir),
        },
        &mut buf,
    );

    assert!(res.is_err());
    let err = res.unwrap_err().to_string();
    assert!(err.contains("Refusing to cite 2026-07-31 v1.0.1"));
    assert!(err.contains("roles table truncated at 65535 rows by a bad build"));
    assert!(err.contains("Update to a valid release: ods pull 2026-07-31"));
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

use ods::commands::fetch::{
    run_local_archive_with_fetchers, Args as FetchArgs, TrudFetcher, TrudReleaseItem,
};
use ods::commands::parquet;
use ods::commands::pull::OciBlobFetcher;
use ods::progress::{Progress, ProgressCaps};
use ods::provenance::compute_file_sha256;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

mod common;
use common::{create_mock_trud_zip, make_v1_index, ods_binary};

fn find_real_trud_zip() -> Option<PathBuf> {
    if let Ok(env_path) = std::env::var("TRUD_XML_PATH") {
        let path = PathBuf::from(env_path);
        if path.exists() {
            return Some(path);
        }
    }

    None
}

fn create_mock_trud_zip_from_xml(dir: &std::path::Path, xml_path: &std::path::Path) -> PathBuf {
    use std::io::Write;
    let zip_path = dir.join("hscorgrefdataxml_data_7.0.0_20260731000001.zip");
    let zip_file = std::fs::File::create(&zip_path).unwrap();
    let mut zip_writer = zip::ZipWriter::new(zip_file);
    let options = zip::write::SimpleFileOptions::default();
    zip_writer.start_file("HSCOrgRefData_Full_mock.xml", options).unwrap();
    let xml_content = std::fs::read_to_string(xml_path).unwrap();
    zip_writer.write_all(xml_content.as_bytes()).unwrap();
    zip_writer.finish().unwrap();
    zip_path
}

#[test]
fn test_synthetic_parquet_hash_stability() {
    let xml_path = PathBuf::from("tests/fixtures/mock_hscorgrefdata.xml");
    assert!(xml_path.exists(), "mock fixture missing");

    let tmp1 = TempDir::new().unwrap();
    let tmp2 = TempDir::new().unwrap();

    let zip_path = create_mock_trud_zip_from_xml(tmp1.path(), &xml_path);

    parquet::run(parquet::Args {
        input: Some(zip_path.clone()),
        output: Some(tmp1.path().to_path_buf()), force: true, ..Default::default() })
    .expect("run 1 should succeed");

    parquet::run(parquet::Args {
        input: Some(zip_path),
        output: Some(tmp2.path().to_path_buf()), force: true, ..Default::default() })
    .expect("run 2 should succeed");

    let parquet_files = vec![
        "orgs.parquet",
        "roles.parquet",
        "relationships.parquet",
        "successions.parquet",
    ];

    for file_name in parquet_files {
        let f1 = tmp1.path().join(file_name);
        let f2 = tmp2.path().join(file_name);

        assert!(f1.exists(), "{} missing in run 1", file_name);
        assert!(f2.exists(), "{} missing in run 2", file_name);

        let hash1 = compute_file_sha256(&f1).unwrap();
        let hash2 = compute_file_sha256(&f2).unwrap();

        assert_eq!(
            hash1, hash2,
            "Parquet file {} SHA-256 hash is not stable across runs: {} vs {}",
            file_name, hash1, hash2
        );
    }
}

#[test]
#[ignore]
fn test_real_trud_parquet_hash_stability() {
    let zip_path = find_real_trud_zip().expect(
        "TRUD_XML_PATH environment variable must point to an existing TRUD zip archive to run this test",
    );

    println!("Testing real TRUD archive reproducibility against {}", zip_path.display());

    let tmp1 = TempDir::new().unwrap();
    let tmp2 = TempDir::new().unwrap();

    parquet::run(parquet::Args {
        input: Some(zip_path.clone()),
        output: Some(tmp1.path().to_path_buf()), ..Default::default() })
    .expect("run 1 on real TRUD zip should succeed");

    parquet::run(parquet::Args {
        input: Some(zip_path.clone()),
        output: Some(tmp2.path().to_path_buf()), ..Default::default() })
    .expect("run 2 on real TRUD zip should succeed");

    // The four Parquet tables are the dataset: each carries the release's provenance.
    let all_files = vec!["orgs.parquet", "roles.parquet", "relationships.parquet", "successions.parquet"];

    for file_name in &all_files {
        let f1 = tmp1.path().join(file_name);
        let f2 = tmp2.path().join(file_name);

        assert!(f1.exists(), "{} missing in run 1", file_name);
        assert!(f2.exists(), "{} missing in run 2", file_name);

        let hash1 = compute_file_sha256(&f1).unwrap();
        let hash2 = compute_file_sha256(&f2).unwrap();

        println!("File {:20} -> SHA256: {}", file_name, hash1);
        assert_eq!(
            hash1, hash2,
            "Real TRUD output file {} SHA-256 hash is not stable across runs: {} vs {}",
            file_name, hash1, hash2
        );
    }

    // Context 2: Copy archive alone to an isolated TempDir without a pull-time record
    let isolated_zip_dir = TempDir::new().unwrap();
    let isolated_zip = isolated_zip_dir.path().join(zip_path.file_name().unwrap());
    std::fs::copy(&zip_path, &isolated_zip).unwrap();

    let tmp_isolated = TempDir::new().unwrap();
    parquet::run(parquet::Args {
        input: Some(isolated_zip),
        output: Some(tmp_isolated.path().to_path_buf()), ..Default::default() })
    .expect("context 2 make should succeed");

    // Every file must match across contexts, provenance included: the embedded object holds
    // only the source release's facts and the dataset version, so a bare archive and a pulled
    // one give the same bytes.
    for file_name in all_files.iter() {
        let f1 = tmp1.path().join(file_name);
        let f2 = tmp_isolated.path().join(file_name);
        let hash1 = compute_file_sha256(&f1).unwrap();
        let hash2 = compute_file_sha256(&f2).unwrap();
        assert_eq!(hash1, hash2, "File {} diverged across contexts", file_name);
    }

    // Compare built manifest digest with the digest recorded in data/releases.json
    let record = ods::provenance::read_release(tmp1.path()).unwrap();
    let facts = record.facts().expect("the build carries provenance");
    let date = facts.release_date.as_str();
    let version = facts.dataset_version.clone();

    let releases_json_path = PathBuf::from("data/releases.json");
    let index = if releases_json_path.exists() {
        let content = std::fs::read_to_string(&releases_json_path).unwrap();
        serde_json::from_str::<ods::index::OdsReleaseIndex>(&content).ok()
    } else {
        ods::index::OdsReleaseIndex::baked().ok()
    };

    let (manifest, _) = ods::commands::make_oci::build_manifest_from_dir(tmp1.path()).unwrap();
    let built_digest = manifest.digest().unwrap();
    println!("Built manifest digest: {}", built_digest);

    if let Some(ref idx) = index {
        let published_dataset = idx
            .releases
            .iter()
            .find(|r| r.trud_release_date == date)
            .and_then(|r| r.datasets.iter().find(|d| d.dataset_version == version));

        if let Some(published) = published_dataset {
            if built_digest != published.manifest_digest {
                let mut moved_file = None;
                let fetcher = ods::commands::pull::HttpOciFetcher;
                for mirror in &idx.mirrors {
                    let manifest_url = mirror.manifest_url(&published.manifest_digest);
                    if let Ok(manifest_bytes) = fetcher.fetch_bytes(&manifest_url) {
                        if let Ok(pub_manifest) =
                            serde_json::from_slice::<ods::oci::OciManifest>(&manifest_bytes)
                        {
                            for layer in &manifest.layers {
                                if let Some(title) = layer
                                    .annotations
                                    .as_ref()
                                    .and_then(|a| a.get(ods::oci::ANNOTATION_TITLE))
                                {
                                    if let Some(pub_layer) = pub_manifest.layers.iter().find(|l| {
                                        l.annotations
                                            .as_ref()
                                            .and_then(|a| a.get(ods::oci::ANNOTATION_TITLE))
                                            == Some(title)
                                    }) {
                                        if layer.digest != pub_layer.digest {
                                            moved_file = Some(title.clone());
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if moved_file.is_some() {
                        break;
                    }
                }
                if moved_file.is_none() {
                    let local_release = PathBuf::from("ods_data/releases").join(date);
                    if local_release.exists() {
                        for layer in &manifest.layers {
                            if let Some(title) = layer
                                .annotations
                                .as_ref()
                                .and_then(|a| a.get(ods::oci::ANNOTATION_TITLE))
                            {
                                let local_file = local_release.join(title);
                                if local_file.exists() {
                                    let built_file = tmp1.path().join(title);
                                    if let (Ok(h1), Ok(h2)) = (
                                        compute_file_sha256(&built_file),
                                        compute_file_sha256(&local_file),
                                    ) {
                                        if h1 != h2 {
                                            moved_file = Some(title.clone());
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                let file_name = moved_file.unwrap_or_else(|| "dataset files".to_string());
                panic!(
                    "Built manifest digest {} does not match published manifest digest {} for release {} v{}.\nFile '{}' changed. The fix is a dataset_version bump.",
                    built_digest, published.manifest_digest, date, version, file_name
                );
            }
        }
    }
}

/// The dataset version is in the Parquet files, inside `version`, so relabelling a release
/// changes every file's bytes and the manifest's layer digests, while the data stays the same.
#[test]
fn test_relabel_dataset_version_changes_parquet_bytes_not_data() {
    use ::parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

    let embedded = |version: &str| {
        ods::provenance::PullRecord::for_trud_release(
            "2026-07-31",
            "hscorgrefdataxml_data_7.0.0_20260731000001.zip",
            "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
            37_983_173,
            &[],
        )
        .unwrap()
        .embedded(version)
        .unwrap()
    };
    let (_tmp1, dir1) = common::setup_find_test_workspace_embedded(&embedded("0.1.0"));
    let (_tmp2, dir2) = common::setup_find_test_workspace_embedded(&embedded("1.0.0"));

    let read_rows = |path: &Path| -> Vec<arrow::record_batch::RecordBatch> {
        let file = std::fs::File::open(path).unwrap();
        ParquetRecordBatchReaderBuilder::try_new(file).unwrap().build().unwrap().map(|b| b.unwrap()).collect()
    };

    let parquet_files = ["orgs.parquet", "roles.parquet", "relationships.parquet", "successions.parquet"];
    for file_name in &parquet_files {
        let f1 = dir1.join(file_name);
        let f2 = dir2.join(file_name);
        assert_ne!(
            compute_file_sha256(&f1).unwrap(),
            compute_file_sha256(&f2).unwrap(),
            "{} must differ between dataset versions: it carries the version",
            file_name
        );
        assert_eq!(read_rows(&f1), read_rows(&f2), "{} must hold the same data in both versions", file_name);
    }

    let (manifest1, _) = ods::commands::make_oci::build_manifest_from_dir(&dir1).unwrap();
    let (manifest2, _) = ods::commands::make_oci::build_manifest_from_dir(&dir2).unwrap();
    assert_ne!(manifest1.digest().unwrap(), manifest2.digest().unwrap(), "manifest digests must differ across dataset versions");
    for (layer1, layer2) in manifest1.layers.iter().zip(manifest2.layers.iter()) {
        assert_ne!(layer1.digest, layer2.digest, "every layer digest changes on a relabel");
    }
}

struct MockTrudApiFetcher {
    releases: Vec<TrudReleaseItem>,
}

impl TrudFetcher for MockTrudApiFetcher {
    fn fetch_releases(&self) -> anyhow::Result<Vec<TrudReleaseItem>> {
        Ok(self.releases.clone())
    }

    fn download_archive(
        &self,
        _url: &str,
        _dest_path: &Path,
        _on_bytes: &(dyn Fn(u64) + Send + Sync),
    ) -> anyhow::Result<()> {
        anyhow::bail!("download_archive is not used by a local archive");
    }
}

struct DiscardWriter;
impl std::io::Write for DiscardWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Pulls `zip` as a local archive into `ws`, then runs `ods make` on the pulled release, and
/// returns the release directory. `confirmed_by` says which route vouches for the zip.
fn pull_and_make(zip: &Path, ws: &Path, confirmed_by: Route) -> PathBuf {
    let sha256 = compute_file_sha256(zip).unwrap();
    let file_size = std::fs::metadata(zip).unwrap().len();
    let dataset = &[("0.1.0", "sha256:0000000000000000000000000000000000000000000000000000000000000000")];

    let progress = Progress::new(
        ProgressCaps { is_tty: false, no_color: true, quiet: true, verbose: false, width: 80 },
        Box::new(DiscardWriter),
    );
    let oci_fetcher = ods::commands::pull::HttpOciFetcher;

    // The index is handed over as a file. When it holds the zip's row it vouches for the zip;
    // when it holds only another date, the mock TRUD API has to.
    let (index_row_date, index_row_sha, trud_fetcher) = match confirmed_by {
        Route::ReleaseIndex => ("2026-07-31", sha256.as_str(), None),
        Route::TrudApi => (
            "2026-06-26",
            "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
            Some(MockTrudApiFetcher {
                releases: vec![TrudReleaseItem {
                    id: "341".to_string(),
                    name: Some("Release 7.0.0".to_string()),
                    release_date: "2026-07-31".to_string(),
                    archive_file_name: "hscorgrefdataxml_data_7.0.0_20260731000001.zip".to_string(),
                    archive_file_sha256: sha256.clone(),
                    archive_file_size: file_size,
                    download_url: "https://example.com/zip".to_string(),
                    ..Default::default()
                }],
            }),
        ),
    };
    let index = make_v1_index(&[(index_row_date, index_row_sha, file_size, dataset)]);
    let index_file = ws.parent().unwrap().join("index.json");
    std::fs::write(&index_file, serde_json::to_string_pretty(&index).unwrap()).unwrap();

    let args = FetchArgs {
        local_archive: Some(zip.to_path_buf()),
        workspace: Some(ws.to_path_buf()),
        index: Some(index_file.to_str().unwrap().to_string()),
        api_key: trud_fetcher.as_ref().map(|_| "dummy_key".to_string()),
        ..Default::default()
    };
    run_local_archive_with_fetchers(&args, ws, zip, &progress, trud_fetcher.as_ref(), Some(&oci_fetcher))
        .expect("trud pull --local-archive should succeed");

    let release_dir = ws.join("releases").join("2026-07-31");
    let output = ods_binary()
        .current_dir(ws)
        .arg("make")
        .arg("--input")
        .arg(&release_dir)
        .output()
        .expect("execute ods make");
    assert!(output.status.success(), "ods make failed:\n{}", String::from_utf8_lossy(&output.stderr));
    release_dir
}

#[derive(Clone, Copy)]
enum Route {
    ReleaseIndex,
    TrudApi,
}

// D1: a dataset's identity is the archive and the dataset version, not how the builder checked
// the archive. One zip, confirmed once by the release index and once by the TRUD API, must
// give the same pull record, the same embedded object and the same manifest digest.
#[test]
fn test_same_zip_gives_same_provenance_and_manifest_however_it_was_checked() {
    let tmp = TempDir::new().unwrap();
    let zip_dir = tmp.path().join("zip");
    std::fs::create_dir_all(&zip_dir).unwrap();
    let zip = create_mock_trud_zip(&zip_dir, "hscorgrefdataxml_data_7.0.0_20260731000001.zip");

    let via_index = tmp.path().join("via_index");
    let via_api = tmp.path().join("via_api");
    std::fs::create_dir_all(&via_index).unwrap();
    std::fs::create_dir_all(&via_api).unwrap();
    let dir_index = pull_and_make(&zip, &via_index.join("ods_data"), Route::ReleaseIndex);
    let dir_api = pull_and_make(&zip, &via_api.join("ods_data"), Route::TrudApi);

    let prov_index = std::fs::read_to_string(ods::provenance::pull_record_path(&dir_index)).unwrap();
    let prov_api = std::fs::read_to_string(ods::provenance::pull_record_path(&dir_api)).unwrap();
    assert_eq!(
        prov_index, prov_api,
        "the pull-time record depends on how the zip was checked:\n--- index ---\n{}\n--- TRUD API ---\n{}",
        prov_index, prov_api
    );

    let digest = |dir: &Path| {
        let (manifest, _) = ods::commands::make_oci::build_manifest_from_dir(dir).unwrap();
        manifest.digest().unwrap()
    };
    assert_eq!(
        digest(&dir_index),
        digest(&dir_api),
        "the manifest digest depends on how the zip was checked"
    );
}

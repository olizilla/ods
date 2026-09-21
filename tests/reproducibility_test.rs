use ods::commands::parquet;
use ods::provenance::compute_file_sha256;
use std::path::PathBuf;
use tempfile::TempDir;

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
        output: Some(tmp1.path().to_path_buf()),
    })
    .expect("run 1 should succeed");

    parquet::run(parquet::Args {
        input: Some(zip_path),
        output: Some(tmp2.path().to_path_buf()),
    })
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
        output: Some(tmp1.path().to_path_buf()),
    })
    .expect("run 1 on real TRUD zip should succeed");

    parquet::run(parquet::Args {
        input: Some(zip_path.clone()),
        output: Some(tmp2.path().to_path_buf()),
    })
    .expect("run 2 on real TRUD zip should succeed");

    let all_files = vec![
        "orgs.parquet",
        "roles.parquet",
        "relationships.parquet",
        "successions.parquet",
        "datapackage.json",
        "_provenance.json",
    ];

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

    // Context 2: Copy archive alone to an isolated TempDir without _provenance.json
    let isolated_zip_dir = TempDir::new().unwrap();
    let isolated_zip = isolated_zip_dir.path().join(zip_path.file_name().unwrap());
    std::fs::copy(&zip_path, &isolated_zip).unwrap();

    let tmp_isolated = TempDir::new().unwrap();
    parquet::run(parquet::Args {
        input: Some(isolated_zip),
        output: Some(tmp_isolated.path().to_path_buf()),
    })
    .expect("context 2 make should succeed");

    // Parquet and datapackage must match across contexts
    for file_name in all_files.iter().filter(|f| **f != "_provenance.json") {
        let f1 = tmp1.path().join(file_name);
        let f2 = tmp_isolated.path().join(file_name);
        let hash1 = compute_file_sha256(&f1).unwrap();
        let hash2 = compute_file_sha256(&f2).unwrap();
        assert_eq!(hash1, hash2, "File {} diverged across contexts", file_name);
    }

    // Provenance verification difference
    let prov1_file = tmp1.path().join("_provenance.json");
    let prov2_file = tmp_isolated.path().join("_provenance.json");
    let prov1: ods::provenance::OdsProvenance =
        serde_json::from_str(&std::fs::read_to_string(&prov1_file).unwrap()).unwrap();
    let prov2: ods::provenance::OdsProvenance =
        serde_json::from_str(&std::fs::read_to_string(&prov2_file).unwrap()).unwrap();
    assert_eq!(
        prov1.trud_release_sha256_verified,
        Some(ods::provenance::TrudVerificationSource::TrudApi),
        "Context 1 provenance must be verified by TRUD API"
    );
    assert_eq!(
        prov2.trud_release_sha256_verified,
        Some(ods::provenance::TrudVerificationSource::Unverified),
        "Context 2 provenance must be unverified"
    );
}

#[test]
fn test_relabel_dataset_version_leaves_parquet_bytes_unchanged() {
    let xml_path = PathBuf::from("tests/fixtures/mock_hscorgrefdata.xml");
    assert!(xml_path.exists(), "mock fixture missing");

    let tmp1 = TempDir::new().unwrap();
    let tmp2 = TempDir::new().unwrap();

    let zip1 = create_mock_trud_zip_from_xml(tmp1.path(), &xml_path);
    let zip2 = create_mock_trud_zip_from_xml(tmp2.path(), &xml_path);

    // Setup _provenance.json in both directories
    let mut prov = ods::provenance::OdsProvenance::default();
    prov.trud_release_date = Some("2026-07-31".to_string());
    prov.trud_release_sha256_verified = Some(ods::provenance::TrudVerificationSource::TrudApi);

    std::fs::write(
        tmp1.path().join(ods::provenance::PROVENANCE_FILENAME),
        serde_json::to_string_pretty(&prov).unwrap(),
    )
    .unwrap();

    std::fs::write(
        tmp2.path().join(ods::provenance::PROVENANCE_FILENAME),
        serde_json::to_string_pretty(&prov).unwrap(),
    )
    .unwrap();

    // Run parquet generation on both releases
    parquet::run(parquet::Args {
        input: Some(zip1),
        output: Some(tmp1.path().to_path_buf()),
    })
    .expect("run 1 should succeed");

    parquet::run(parquet::Args {
        input: Some(zip2),
        output: Some(tmp2.path().to_path_buf()),
    })
    .expect("run 2 should succeed");

    // Relabel dataset_version in tmp2's datapackage.json: 0.1.0 -> 1.0.0
    let mut dp2: serde_json::Value =
        serde_json::from_reader(std::fs::File::open(tmp2.path().join("datapackage.json")).unwrap()).unwrap();
    dp2["version"] = serde_json::json!("1.0.0");
    std::fs::write(
        tmp2.path().join("datapackage.json"),
        serde_json::to_string_pretty(&dp2).unwrap(),
    )
    .unwrap();

    // 1. The five Parquet files must be byte-identical
    let parquet_files = vec![
        "orgs.parquet",
        "roles.parquet",
        "relationships.parquet",
        "successions.parquet",
    ];

    for file_name in &parquet_files {
        let f1 = tmp1.path().join(file_name);
        let f2 = tmp2.path().join(file_name);

        let hash1 = compute_file_sha256(&f1).unwrap();
        let hash2 = compute_file_sha256(&f2).unwrap();

        assert_eq!(
            hash1, hash2,
            "Parquet file {} differs between dataset versions ({} vs {})",
            file_name, hash1, hash2
        );
    }

    // 2. _provenance.json is unchanged; datapackage.json differs
    let prov1_bytes = std::fs::read(tmp1.path().join(ods::provenance::PROVENANCE_FILENAME)).unwrap();
    let prov2_bytes = std::fs::read(tmp2.path().join(ods::provenance::PROVENANCE_FILENAME)).unwrap();
    assert_eq!(prov1_bytes, prov2_bytes, "_provenance.json must not change on relabel");

    let dp1_bytes = std::fs::read(tmp1.path().join("datapackage.json")).unwrap();
    let dp2_bytes = std::fs::read(tmp2.path().join("datapackage.json")).unwrap();
    assert_ne!(dp1_bytes, dp2_bytes, "datapackage.json must differ across dataset versions");

    // 3. The two manifest digests differ, and share five layer digests
    let prov1_loaded = ods::provenance::OdsProvenance::load_from_dir(tmp1.path()).unwrap();
    let prov2_loaded = ods::provenance::OdsProvenance::load_from_dir(tmp2.path()).unwrap();

    let (manifest1, _) = ods::commands::make_oci::build_manifest_from_dir(tmp1.path(), &prov1_loaded, "0.1.0").unwrap();
    let (manifest2, _) = ods::commands::make_oci::build_manifest_from_dir(tmp2.path(), &prov2_loaded, "1.0.0").unwrap();

    let digest1 = manifest1.digest().unwrap();
    let digest2 = manifest2.digest().unwrap();
    assert_ne!(digest1, digest2, "Manifest digests must differ across dataset versions");

    // Find and compare layer digests for the five Parquet files
    for file_name in &parquet_files {
        let layer1 = manifest1
            .layers
            .iter()
            .find(|l| {
                l.annotations
                    .as_ref()
                    .and_then(|a| a.get("org.opencontainers.image.title"))
                    .map(|t| t == file_name)
                    .unwrap_or(false)
            })
            .unwrap_or_else(|| panic!("layer for {} missing in manifest 1", file_name));

        let layer2 = manifest2
            .layers
            .iter()
            .find(|l| {
                l.annotations
                    .as_ref()
                    .and_then(|a| a.get("org.opencontainers.image.title"))
                    .map(|t| t == file_name)
                    .unwrap_or(false)
            })
            .unwrap_or_else(|| panic!("layer for {} missing in manifest 2", file_name));

        assert_eq!(
            layer1.digest, layer2.digest,
            "Manifest layer digest for {} must match across dataset versions: {} vs {}",
            file_name, layer1.digest, layer2.digest
        );
    }
}


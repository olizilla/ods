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
        "orgs_all.parquet",
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
        "orgs_all.parquet",
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
    for file_name in &all_files[..6] {
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

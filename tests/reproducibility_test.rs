use ods::commands::parquet;
use ods::provenance::compute_file_sha256;
use std::path::PathBuf;
use tempfile::TempDir;

fn find_real_trud_zip() -> Option<PathBuf> {
    let candidates = vec![
        ".local/hscorgrefdataxml_data_7.0.0_20260731000001.zip",
        "./ods_data_good/releases/2026-07-31/trud/hscorgrefdataxml_data_7.0.0_20260731000001.zip",
        "./ods_data/releases/2026-07-31/trud/hscorgrefdataxml_data_7.0.0_20260731000001.zip",
        "./ods_data/current/trud/hscorgrefdataxml_data_7.0.0_20260731000001.zip",
    ];

    for c in candidates {
        let path = PathBuf::from(c);
        if path.exists() {
            return Some(path);
        }
    }

    if let Ok(env_path) = std::env::var("TRUD_XML_PATH") {
        let path = PathBuf::from(env_path);
        if path.exists() {
            return Some(path);
        }
    }

    None
}

#[test]
fn test_synthetic_parquet_hash_stability() {
    let xml_path = PathBuf::from("tests/fixtures/mock_hscorgrefdata.xml");
    assert!(xml_path.exists(), "mock fixture missing");

    let tmp1 = TempDir::new().unwrap();
    let tmp2 = TempDir::new().unwrap();

    parquet::run(parquet::Args {
        input: xml_path.clone(),
        output: tmp1.path().to_path_buf(),
    })
    .expect("run 1 should succeed");

    parquet::run(parquet::Args {
        input: xml_path,
        output: tmp2.path().to_path_buf(),
    })
    .expect("run 2 should succeed");

    let parquet_files = vec![
        "orgs.parquet",
        "orgs_all.parquet",
        "org_roles.parquet",
        "roles.parquet",
        "rels.parquet",
        "successors.parquet",
        "category_rules.json",
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
    let zip_path = match find_real_trud_zip() {
        Some(path) => path,
        None => {
            eprintln!("Skipping test_real_trud_parquet_hash_stability: no real TRUD zip found");
            return;
        }
    };

    println!("Testing real TRUD archive reproducibility against {}", zip_path.display());

    let tmp1 = TempDir::new().unwrap();
    let tmp2 = TempDir::new().unwrap();

    parquet::run(parquet::Args {
        input: zip_path.clone(),
        output: tmp1.path().to_path_buf(),
    })
    .expect("run 1 on real TRUD zip should succeed");

    parquet::run(parquet::Args {
        input: zip_path,
        output: tmp2.path().to_path_buf(),
    })
    .expect("run 2 on real TRUD zip should succeed");

    let parquet_files = vec![
        "orgs.parquet",
        "orgs_all.parquet",
        "org_roles.parquet",
        "roles.parquet",
        "rels.parquet",
        "successors.parquet",
        "category_rules.json",
    ];

    for file_name in parquet_files {
        let f1 = tmp1.path().join(file_name);
        let f2 = tmp2.path().join(file_name);

        assert!(f1.exists(), "{} missing in run 1", file_name);
        assert!(f2.exists(), "{} missing in run 2", file_name);

        let hash1 = compute_file_sha256(&f1).unwrap();
        let hash2 = compute_file_sha256(&f2).unwrap();

        println!("File {:20} -> SHA256: {}", file_name, hash1);
        assert_eq!(
            hash1, hash2,
            "Real TRUD Parquet file {} SHA-256 hash is not stable across runs: {} vs {}",
            file_name, hash1, hash2
        );
    }
}

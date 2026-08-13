use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn find_cargo_lock(start_dir: &Path) -> Option<PathBuf> {
    let mut curr = start_dir.to_path_buf();
    loop {
        let candidate = curr.join("Cargo.lock");
        if candidate.exists() {
            return Some(candidate);
        }
        if !curr.pop() {
            break;
        }
    }
    None
}

fn extract_version(lock_path: &Path, pkg_name: &str) -> Option<String> {
    let content = fs::read_to_string(lock_path).ok()?;
    let mut current_name = String::new();
    for line in content.lines() {
        let line = line.trim();
        if line == "[[package]]" {
            current_name.clear();
        } else if line.starts_with("name = ") {
            current_name = line
                .trim_start_matches("name = ")
                .trim_matches('"')
                .to_string();
        } else if line.starts_with("version = ") && current_name == pkg_name {
            let ver = line
                .trim_start_matches("version = ")
                .trim_matches('"')
                .to_string();
            return Some(ver);
        }
    }
    None
}

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set");
    let manifest_path = PathBuf::from(&manifest_dir);

    if let Some(lock_path) = find_cargo_lock(&manifest_path) {
        println!("cargo:rerun-if-changed={}", lock_path.display());

        let parquet_ver = extract_version(&lock_path, "parquet").unwrap_or_else(|| "unknown".to_string());
        let arrow_ver = extract_version(&lock_path, "arrow").unwrap_or_else(|| "unknown".to_string());

        println!("cargo:rustc-env=ODS_TOOL_PARQUET_VERSION={}", parquet_ver);
        println!("cargo:rustc-env=ODS_TOOL_ARROW_VERSION={}", arrow_ver);
    } else {
        println!("cargo:rustc-env=ODS_TOOL_PARQUET_VERSION=unknown");
        println!("cargo:rustc-env=ODS_TOOL_ARROW_VERSION=unknown");
    }
}

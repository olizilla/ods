//! `scripts/mirror-to-ods-fyi.sh` is the one place ods.fyi's bucket keys are laid out
//! (ci-builds-datasets.md, Task 8 and 9). This is the two-sided contract check that used to live
//! in `tests/make_release_test.rs` against `ods make release`'s removed `dist/` staging tree:
//! the script's dry-run key list, for a real compiled release, must exactly match
//! `worker/test/fixtures/expected-keys.json`. `scripts/generate-expected-keys.sh` and
//! `scripts/bump-version.sh` regenerate that fixture by running this test with
//! `UPDATE_EXPECTED_KEYS=1`.

mod common;
use common::setup_synthetic_repo_and_release;
use std::path::Path;
use std::process::Command;

fn jq_available() -> bool {
    Command::new("jq").arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

fn require_jq() -> bool {
    if jq_available() {
        return true;
    }
    if std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true") {
        panic!("jq is required in CI, but was not found on PATH");
    }
    eprintln!("Skipping test: jq not found on PATH");
    false
}

#[test]
fn mirror_dry_run_keys_match_expected_keys_json() {
    if !require_jq() {
        return;
    }

    let (tmp, rel_dir) = setup_synthetic_repo_and_release();
    let version = ods::datapackage::DATASET_VERSION;
    let oci_dir = rel_dir.join("oci");
    assert!(oci_dir.join("index.json").exists(), "setup_synthetic_repo_and_release must pack oci/");

    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts").join("mirror-to-ods-fyi.sh");
    let out = Command::new(&script)
        .arg("2026-07-31")
        .arg(version)
        .arg("--repository")
        .arg("ods-data")
        .arg("--from-oci-layout")
        .arg(&oci_dir)
        .current_dir(tmp.path())
        .output()
        .expect("run scripts/mirror-to-ods-fyi.sh");

    assert!(
        out.status.success(),
        "mirror-to-ods-fyi.sh failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut actual_keys: Vec<String> = Vec::new();
    let mut in_order_block = false;
    for line in stdout.lines() {
        if line.trim() == "* upload order:" {
            in_order_block = true;
            continue;
        }
        if in_order_block {
            let trimmed = line.trim();
            if trimmed.starts_with("v2/") {
                actual_keys.push(trimmed.to_string());
            } else {
                break;
            }
        }
    }
    assert!(!actual_keys.is_empty(), "no keys parsed from mirror-to-ods-fyi.sh's output:\n{}", stdout);
    actual_keys.sort();

    let expected_keys_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("worker")
        .join("test")
        .join("fixtures")
        .join("expected-keys.json");

    if std::env::var("UPDATE_EXPECTED_KEYS").is_ok() || std::env::var("UPDATE_FIXTURES").is_ok() {
        let json = serde_json::to_string_pretty(&actual_keys).unwrap();
        std::fs::write(&expected_keys_path, format!("{}\n", json)).unwrap();
    }

    let expected_keys_content = std::fs::read_to_string(&expected_keys_path)
        .expect("worker/test/fixtures/expected-keys.json must exist");
    let mut expected_keys: Vec<String> = serde_json::from_str(&expected_keys_content)
        .expect("expected-keys.json must be valid JSON array of strings");
    expected_keys.sort();

    assert_eq!(
        actual_keys, expected_keys,
        "keys laid out by scripts/mirror-to-ods-fyi.sh must exactly match worker/test/fixtures/expected-keys.json"
    );
}

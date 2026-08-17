use anyhow::Result;
use ods::commands::pull::{
    run_list, run_with_fetcher, AlreadyReported, Args, GithubAsset, GithubRelease, ReleaseFetcher,
};
use sha2::Digest;
use std::fs;
use tempfile::TempDir;

struct MockReleaseFetcher {
    pub releases: Vec<GithubRelease>,
    pub file_contents: std::collections::BTreeMap<String, Vec<u8>>,
}

impl ReleaseFetcher for MockReleaseFetcher {
    fn fetch_releases(&self) -> Result<Vec<GithubRelease>> {
        Ok(self.releases.clone())
    }

    fn download_asset(&self, url: &str) -> Result<Vec<u8>> {
        for (key, val) in &self.file_contents {
            if url.contains(key) {
                return Ok(val.clone());
            }
        }
        anyhow::bail!("Mock asset not found for URL: {}", url)
    }
}

fn create_mock_dataset() -> (Vec<GithubRelease>, std::collections::BTreeMap<String, Vec<u8>>) {
    let mock_files = vec![
        ("orgs.parquet", b"mock orgs parquet content".to_vec()),
        ("orgs_all.parquet", b"mock orgs all parquet content".to_vec()),
        ("org_roles.parquet", b"mock org roles content".to_vec()),
        ("roles.parquet", b"mock roles content".to_vec()),
        ("relationships.parquet", b"mock rels content".to_vec()),
        ("successions.parquet", b"mock successions content".to_vec()),
        ("category_rules.json", b"mock category rules json".to_vec()),
        ("_provenance.json", b"{\"_type\":\"ods_provenance\"}".to_vec()),
    ];

    let mut contents = std::collections::BTreeMap::new();
    let mut sha_lines = Vec::new();

    for (name, bytes) in &mock_files {
        let hash = format!("{:x}", sha2::Sha256::digest(bytes));
        sha_lines.push(format!("{}  {}", hash, name));
        contents.insert(name.to_string(), bytes.clone());
    }

    let sha_bytes = (sha_lines.join("\n") + "\n").into_bytes();
    contents.insert("SHA256SUMS".to_string(), sha_bytes.clone());

    let assets: Vec<GithubAsset> = contents
        .keys()
        .map(|name| GithubAsset {
            name: name.clone(),
            browser_download_url: format!("https://github.com/mock/download/{}", name),
            size: contents.get(name).unwrap().len() as u64,
        })
        .collect();

    let release = GithubRelease {
        tag_name: "data/2026-07-31".to_string(),
        name: Some("ODS Release 2026-07-31".to_string()),
        draft: false,
        prerelease: false,
        assets,
        body: Some("DOI: 10.5281/zenodo.123456".to_string()),
    };

    (vec![release], contents)
}

#[test]
fn test_pull_latest_release_downloads_verifies_and_links() {
    let (releases, contents) = create_mock_dataset();
    let fetcher = MockReleaseFetcher {
        releases,
        file_contents: contents,
    };

    let tmp = TempDir::new().unwrap();
    let workspace_root = tmp.path().join("ods_data");

    run_with_fetcher(Args::default(), &workspace_root, &fetcher)
        .expect("pull latest should succeed");

    let current_dir = workspace_root.join("current");
    assert!(current_dir.exists(), "ods_data/current symlink must exist");
    assert!(
        current_dir.join("orgs.parquet").exists(),
        "orgs.parquet must exist in current"
    );
    assert!(
        current_dir.join("_provenance.json").exists(),
        "_provenance.json must exist in current"
    );
    assert!(
        current_dir.join("SHA256SUMS").exists(),
        "SHA256SUMS must exist in current"
    );

    let prov_str = std::fs::read_to_string(current_dir.join("_provenance.json")).unwrap();
    assert!(
        prov_str.contains("10.5281/zenodo.123456"),
        "dataset_doi must be populated from release metadata"
    );
}

#[test]
fn test_pull_idempotent_cache_hit() {
    let (releases, contents) = create_mock_dataset();
    let fetcher = MockReleaseFetcher {
        releases,
        file_contents: contents,
    };

    let tmp = TempDir::new().unwrap();
    let workspace_root = tmp.path().join("ods_data");

    run_with_fetcher(Args::default(), &workspace_root, &fetcher).unwrap();
    // Second run should use cached release without error
    run_with_fetcher(Args::default(), &workspace_root, &fetcher).unwrap();
}

#[test]
fn test_pull_corrupted_file_appends_bad_sha() {
    let (releases, mut contents) = create_mock_dataset();
    // Tamper orgs.parquet bytes so hash mismatch occurs
    contents.insert("orgs.parquet".to_string(), b"CORRUPTED BYTES".to_vec());

    let fetcher = MockReleaseFetcher {
        releases,
        file_contents: contents,
    };

    let tmp = TempDir::new().unwrap();
    let workspace_root = tmp.path().join("ods_data");

    let res = run_with_fetcher(Args::default(), &workspace_root, &fetcher);
    assert!(res.is_err(), "pull must fail on checksum mismatch");

    let err_msg = res.unwrap_err().to_string();
    assert!(
        err_msg.contains("Checksum verification failed"),
        "Error message must mention checksum failure"
    );
    assert!(
        err_msg.contains(".bad-sha"),
        "Error message must mention .bad-sha filename"
    );

    let current_dir = workspace_root.join("current");
    assert!(
        !current_dir.exists(),
        "current symlink must NOT be updated on failure"
    );
}

#[test]
fn test_pull_list_stdout_redirection() {
    let (releases, contents) = create_mock_dataset();
    let fetcher = MockReleaseFetcher {
        releases,
        file_contents: contents,
    };

    let tmp = TempDir::new().unwrap();
    let workspace_root = tmp.path().join("ods_data");

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    run_list(&workspace_root, &fetcher, &mut stdout, &mut stderr).unwrap();

    let stdout_str = String::from_utf8(stdout).unwrap();
    let stderr_str = String::from_utf8(stderr).unwrap();

    assert!(
        !stdout_str.is_empty(),
        "stdout must contain table rows for piping/redirection"
    );
    assert!(
        stdout_str.contains("2026-07-31"),
        "stdout must include 2026-07-31"
    );
    assert!(
        stdout_str.contains("○ remote"),
        "stdout must mark un-pulled release as ○ remote"
    );

    assert!(
        stderr_str.contains("Querying available ODS dataset releases"),
        "stderr must contain progress logs"
    );
    assert!(
        stderr_str.contains("Legend:"),
        "stderr must contain Legend"
    );
}

#[test]
fn test_pull_nonexistent_release_lists_available() {
    let (releases, contents) = create_mock_dataset();
    let fetcher = MockReleaseFetcher {
        releases,
        file_contents: contents,
    };

    let tmp = TempDir::new().unwrap();
    let workspace_root = tmp.path().join("ods_data");

    let args = Args {
        release_date: Some("1999-01-01".to_string()),
        ..Default::default()
    };

    let res = run_with_fetcher(args, &workspace_root, &fetcher);
    assert!(res.is_err(), "pull for 1999-01-01 must fail");
    let err_msg = res.unwrap_err().to_string();

    assert!(
        err_msg.contains("Release 1999-01-01 not found upstream"),
        "Error message must state release 1999-01-01 not found"
    );
    assert!(
        err_msg.contains("2026-07-31"),
        "Error message must list available releases"
    );
}

struct FailingMockReleaseFetcher;

impl ReleaseFetcher for FailingMockReleaseFetcher {
    fn fetch_releases(&self) -> Result<Vec<GithubRelease>> {
        anyhow::bail!("✖ Could not reach GitHub API (HTTP 404)")
    }

    fn download_asset(&self, _url: &str) -> Result<Vec<u8>> {
        anyhow::bail!("✖ Could not reach GitHub API")
    }
}

#[test]
fn test_pull_list_reports_api_fetch_error() {
    let fetcher = FailingMockReleaseFetcher;
    let tmp = TempDir::new().unwrap();
    let workspace_root = tmp.path().join("ods_data");

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let res = run_list(&workspace_root, &fetcher, &mut stdout, &mut stderr);

    let stderr_str = String::from_utf8(stderr).unwrap();
    assert!(
        stderr_str.contains("✖ Could not reach GitHub API (HTTP 404)"),
        "stderr must report the GitHub API failure message"
    );
    assert!(
        stderr_str.contains("(No local dataset releases cached)"),
        "stderr must clarify no local dataset releases are cached"
    );

    // An unreachable release index is a failure even though the command
    // printed something useful — the exit status is the only signal a script
    // can read, and it must not present a partial listing as a whole one.
    let err = res.expect_err("unreachable release index must be a non-zero exit");
    assert!(
        err.downcast_ref::<AlreadyReported>().is_some(),
        "should signal AlreadyReported so main exits non-zero without reprinting, got: {err}"
    );
}

/// The same failure with a release already cached locally: the local row is
/// still listed, and it still exits non-zero, because the upstream half of the
/// answer is missing and a newer release may exist.
#[test]
fn test_pull_list_errors_on_unreachable_index_even_with_local_release() {
    let fetcher = FailingMockReleaseFetcher;
    let tmp = TempDir::new().unwrap();
    let workspace_root = tmp.path().join("ods_data");
    fs::create_dir_all(workspace_root.join("releases").join("2026-07-31")).unwrap();

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let res = run_list(&workspace_root, &fetcher, &mut stdout, &mut stderr);

    let stdout_str = String::from_utf8(stdout).unwrap();
    assert!(
        stdout_str.contains("2026-07-31"),
        "the locally cached release must still be listed, got: {stdout_str:?}"
    );

    let err = res.expect_err("unreachable index is a failure even with local releases");
    assert!(err.downcast_ref::<AlreadyReported>().is_some());
}

#[test]
fn test_pull_specific_local_release_succeeds_offline_without_network() {
    let tmp = TempDir::new().unwrap();
    let workspace_root = tmp.path().join("ods_data");

    // Pre-populate a valid local release
    let rel_dir = workspace_root.join("releases").join("2026-05-29");
    fs::create_dir_all(&rel_dir).unwrap();
    fs::write(rel_dir.join("orgs.parquet"), b"mock parquet").unwrap();
    let hash = format!("{:x}", sha2::Sha256::digest(b"mock parquet"));
    fs::write(rel_dir.join("SHA256SUMS"), format!("{}  orgs.parquet\n", hash)).unwrap();

    let fetcher = FailingMockReleaseFetcher;
    let args = Args {
        release_date: Some("2026-05-29".to_string()),
        ..Default::default()
    };

    let res = run_with_fetcher(args, &workspace_root, &fetcher);
    assert!(
        res.is_ok(),
        "pull of an existing verified local release must succeed offline, got: {:?}",
        res.err()
    );

    let (active_date, _) = ods::workspace::get_active_release(&workspace_root).unwrap();
    assert_eq!(active_date, "2026-05-29", "current symlink must switch to 2026-05-29");
}

#[test]
fn test_pull_latest_offline_fails_and_lists_local_options() {
    let tmp = TempDir::new().unwrap();
    let workspace_root = tmp.path().join("ods_data");

    // Pre-populate a local release
    let rel_dir = workspace_root.join("releases").join("2026-05-29");
    fs::create_dir_all(&rel_dir).unwrap();

    let fetcher = FailingMockReleaseFetcher;
    let args = Args {
        release_date: None,
        ..Default::default()
    };

    let res = run_with_fetcher(args, &workspace_root, &fetcher);
    let err = res.expect_err("pull latest offline must fail");
    assert!(
        err.downcast_ref::<AlreadyReported>().is_some(),
        "must return AlreadyReported"
    );
}

#[test]
fn test_pull_all_states_total_size_and_caches_existing() {
    let (mut releases, contents) = create_mock_dataset();
    let mut release2 = releases[0].clone();
    release2.tag_name = "data/2026-06-26".to_string();
    releases.push(release2);

    let fetcher = MockReleaseFetcher {
        releases,
        file_contents: contents,
    };

    let tmp = TempDir::new().unwrap();
    let workspace_root = tmp.path().join("ods_data");

    let args = Args {
        all: true,
        ..Default::default()
    };

    run_with_fetcher(args, &workspace_root, &fetcher).expect("pull --all must succeed");

    assert!(workspace_root.join("releases").join("2026-07-31").join("orgs.parquet").exists());
    assert!(workspace_root.join("releases").join("2026-06-26").join("orgs.parquet").exists());

    let (active_date, _) = ods::workspace::get_active_release(&workspace_root).unwrap();
    assert_eq!(active_date, "2026-07-31", "current symlink must point to latest release");
}


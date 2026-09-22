//! Acceptance tests for `.agents/briefs/commands-write-where-told.md`.
//!
//! Protects `docs/tests.md` W4: creating a workspace is a side effect worth earning.
//! `ods pull --list` only reads, and `ods trud pull -o <dir>` was told where to write;
//! neither should leave an `ods_data/` behind. Commands that install a release keep
//! creating one, exactly as today.

mod common;

use common::create_mock_trud_zip;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use tempfile::TempDir;

fn ods_binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ods"))
}

fn empty_dir_listing(dir: &std::path::Path) -> Vec<String> {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect()
}

/// A one-shot local HTTP server serving `response_body` for every request, so a
/// test can exercise the "successful fetch" branch of `resolve_index_with_baked`
/// without reaching the real network. Stop it by sending on the returned channel.
fn run_mock_http_server(response_body: Vec<u8>) -> (String, mpsc::Sender<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        loop {
            if rx.try_recv().is_ok() {
                break;
            }
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
                    response_body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.write_all(&response_body);
                let _ = stream.flush();
            }
            thread::sleep(std::time::Duration::from_millis(10));
        }
    });
    (format!("http://127.0.0.1:{}/releases.json", port), tx)
}

// ---------------------------------------------------------------------------
// Task 1: `ods pull --list` reads what is there
// ---------------------------------------------------------------------------

#[test]
fn pull_list_in_empty_directory_writes_nothing() {
    let idx_dir = TempDir::new().unwrap();
    let idx_file = idx_dir.path().join("releases.json");
    fs::write(&idx_file, ods::index::BAKED_RELEASES_JSON_BYTES).unwrap();

    let cwd = TempDir::new().unwrap();
    let output = ods_binary()
        .current_dir(cwd.path())
        .args(["pull", "--list", "--index", idx_file.to_str().unwrap()])
        .output()
        .expect("run ods pull --list");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let entries = empty_dir_listing(cwd.path());
    assert!(
        entries.is_empty(),
        "an empty directory must stay empty after `ods pull --list`, found: {:?}",
        entries
    );
}

#[test]
fn pull_list_in_existing_workspace_still_marks_local_and_active() {
    let idx_dir = TempDir::new().unwrap();
    let idx_file = idx_dir.path().join("releases.json");
    let index = common::make_v1_index(&[(
        "2026-08-28",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37_000_000,
        &[("1.0.0", "sha256:0f2a000000000000000000000000000000000000000000000000000000000000")],
    )]);
    fs::write(&idx_file, serde_json::to_vec_pretty(&index).unwrap()).unwrap();

    let ws_root = TempDir::new().unwrap();
    let ws = ws_root.path().join("ods_data");
    fs::create_dir_all(ws.join("releases").join("2026-08-28")).unwrap();
    let workspace = ods::workspace::Workspace::open_or_create(Some(&ws)).unwrap();
    workspace.set_active("2026-08-28").unwrap();

    let output = ods_binary()
        .current_dir(&ws)
        .args(["pull", "--list", "--index", idx_file.to_str().unwrap()])
        .output()
        .expect("run ods pull --list");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("2026-08-28") && stdout.contains("active (local)"),
        "an existing local, active release is still marked active (local), got:\n{}",
        stdout
    );
}

#[test]
fn pull_date_with_index_flag_in_empty_directory_still_creates_workspace() {
    let idx_dir = TempDir::new().unwrap();
    let idx_file = idx_dir.path().join("releases.json");
    fs::write(&idx_file, ods::index::BAKED_RELEASES_JSON_BYTES).unwrap();

    let cwd = TempDir::new().unwrap();
    let output = ods_binary()
        .current_dir(cwd.path())
        .args(["pull", "--index", idx_file.to_str().unwrap()])
        .output()
        .expect("run ods pull --index");

    // No release is published in an empty baked index, so this is expected to fail
    // with "no releases available" rather than install one — but it must still have
    // earned a workspace by the time it gives up, since installing is what `ods pull`
    // without `--list` is for.
    let entries = empty_dir_listing(cwd.path());
    assert!(
        entries.iter().any(|e| e == "ods_data"),
        "`ods pull` (not `--list`) still creates a workspace, found: {:?}; stderr: {}",
        entries,
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn pull_list_with_a_stale_non_workspace_ods_data_writes_nothing() {
    // A directory that happens to be called ods_data but holds no _releases.json is
    // not a workspace: it's exactly what `Workspace::open` finds nothing at, so
    // `pull::run`'s no-workspace fallback path (relative "ods_data") lands here too.
    let index = common::make_v1_index(&[(
        "2026-08-28",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37_000_000,
        &[("1.0.0", "sha256:0f2a000000000000000000000000000000000000000000000000000000000000")],
    )]);
    let (url, stop_server) = run_mock_http_server(serde_json::to_vec_pretty(&index).unwrap());

    let cwd = TempDir::new().unwrap();
    fs::create_dir_all(cwd.path().join("ods_data")).unwrap();

    let output = ods_binary()
        .current_dir(cwd.path())
        .env("ODS_RELEASE_INDEX_URL", &url)
        .args(["pull", "--list"])
        .output()
        .expect("run ods pull --list");

    let _ = stop_server.send(());

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("2026-08-28") && stdout.contains("remote"),
        "the fetched index's rows still print, got:\n{}",
        stdout
    );
    assert_eq!(
        empty_dir_listing(&cwd.path().join("ods_data")),
        Vec::<String>::new(),
        "a stale, non-workspace ods_data/ must stay exactly as empty as it was"
    );
}

#[test]
fn pull_list_in_existing_workspace_refreshes_its_cached_index() {
    let index = common::make_v1_index(&[(
        "2026-08-28",
        "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
        37_000_000,
        &[("1.0.0", "sha256:0f2a000000000000000000000000000000000000000000000000000000000000")],
    )]);
    let served_bytes = serde_json::to_vec_pretty(&index).unwrap();
    let (url, stop_server) = run_mock_http_server(served_bytes.clone());

    let ws_root = TempDir::new().unwrap();
    let ws = ws_root.path().join("ods_data");
    ods::workspace::Workspace::open_or_create(Some(&ws)).unwrap();
    let marker = ws.join("_releases.json");
    let before_mtime = fs::metadata(&marker).unwrap().modified().unwrap();
    // A workspace's own clock resolution can be coarser than this test; make sure
    // the refreshed write has a chance to land in a strictly later tick.
    std::thread::sleep(std::time::Duration::from_millis(1100));

    let output = ods_binary()
        .current_dir(&ws)
        .env("ODS_RELEASE_INDEX_URL", &url)
        .args(["pull", "--list"])
        .output()
        .expect("run ods pull --list");

    let _ = stop_server.send(());

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let after_bytes = fs::read(&marker).unwrap();
    let after_mtime = fs::metadata(&marker).unwrap().modified().unwrap();
    assert_eq!(after_bytes, served_bytes, "the workspace's cached index is refreshed with the served bytes");
    assert!(
        after_mtime > before_mtime,
        "the cached index file is rewritten, not left untouched"
    );
}

// ---------------------------------------------------------------------------
// Task 2: `-o` means only there
// ---------------------------------------------------------------------------

#[test]
fn trud_pull_local_archive_with_output_creates_only_that_directory() {
    let src = TempDir::new().unwrap();
    let zip_path = create_mock_trud_zip(src.path(), "hscorgrefdataxml_data_7.0.0_20260731000001.zip");

    let cwd = TempDir::new().unwrap();
    let output = ods_binary()
        .current_dir(cwd.path())
        .args(["trud", "pull", "--local-archive"])
        .arg(&zip_path)
        .arg("-o")
        .arg("out")
        .output()
        .expect("run ods trud pull -o out");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let entries = empty_dir_listing(cwd.path());
    assert_eq!(
        entries,
        vec!["out".to_string()],
        "`-o` must be the only thing created, found: {:?}",
        entries
    );
    assert!(cwd.path().join("out").join("_provenance.json").exists());
}

#[test]
fn trud_pull_local_archive_without_output_still_creates_workspace() {
    let src = TempDir::new().unwrap();
    let zip_path = create_mock_trud_zip(src.path(), "hscorgrefdataxml_data_7.0.0_20260731000001.zip");

    let cwd = TempDir::new().unwrap();
    let output = ods_binary()
        .current_dir(cwd.path())
        .args(["trud", "pull", "--local-archive"])
        .arg(&zip_path)
        .output()
        .expect("run ods trud pull with no -o");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let ws = cwd.path().join("ods_data");
    assert!(ws.exists(), "no `-o` still creates ods_data/");
    let (active_date, _) = ods::workspace::Workspace::open(Some(&ws)).unwrap().active_release().unwrap();
    assert_eq!(active_date, "2026-07-31", "the pulled release is pinned current");
}

#[test]
fn trud_pull_local_archive_with_workspace_flag_creates_that_workspace() {
    let src = TempDir::new().unwrap();
    let zip_path = create_mock_trud_zip(src.path(), "hscorgrefdataxml_data_7.0.0_20260731000001.zip");

    let cwd = TempDir::new().unwrap();
    // `Workspace::open_or_create` only creates an explicit path that already exists
    // (empty) or is named `ods_data`; a brand-new, differently-named path is refused,
    // so a typo in `--workspace` can't silently create a directory (`workspace_test.rs`
    // `test_workspace_open_or_create_nonexistent_explicit_path_refuses`).
    let ws = cwd.path().join("named_workspace");
    fs::create_dir_all(&ws).unwrap();
    let output = ods_binary()
        .current_dir(cwd.path())
        .args(["trud", "pull", "--local-archive"])
        .arg(&zip_path)
        .arg("--workspace")
        .arg(&ws)
        .output()
        .expect("run ods trud pull --workspace");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(ws.exists(), "--workspace still creates that workspace");
    assert!(
        !cwd.path().join("ods_data").exists(),
        "the default ods_data/ must not also appear"
    );
}

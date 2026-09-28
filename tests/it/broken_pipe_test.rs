//! Acceptance test for `.agents/briefs/backlog-top4.md` Task 4.
//!
//! When whatever reads `ods`'s stdout stops early (`ods pull --list | head`), `ods`
//! must stop quietly and exit 0 — not panic with "failed printing to stdout:
//! Broken pipe (os error 32)" and exit 101. Protects `docs/tests.md` O1.
//!
//! `ods pull --list` writes its release table with raw `println!` (`list_releases_cmd`
//! in `src/commands/pull.rs`), unlike `find`/`role`, which already write through an
//! injected `Write` and so never panic on a closed pipe. A release index of a few
//! thousand synthetic releases makes that table comfortably exceed a pipe's 64 KB
//! kernel buffer, so `ods` is still writing when the reader below closes its end.

use crate::common;

use chrono::NaiveDate;
use ods::index::{Dataset, MirrorEntry, OdsReleaseIndex, Release, RELEASES_SCHEMA_V1_URL};
use std::io::Read;
use std::process::Stdio;

/// A valid release index with `count` synthetic releases, newest first, each a
/// short but valid `YYYY-MM-DD` calendar date so `OdsReleaseIndex::validate` accepts it.
fn make_large_index(count: usize) -> OdsReleaseIndex {
    let base = NaiveDate::from_ymd_opt(2000, 1, 1).expect("valid base date");

    let mut releases: Vec<Release> = (0..count)
        .map(|i| {
            let date = base + chrono::Duration::days(i as i64);
            Release {
                trud_release_date: date.format("%Y-%m-%d").to_string(),
                trud_release_sha256: "A".repeat(64),
                trud_release_filesize_bytes: 1,
                datasets: vec![Dataset {
                    dataset_version: "1.0.1".to_string(),
                    manifest_digest: format!("sha256:{}", "b".repeat(64)),
                    dataset_filesize_bytes: 1,
                    tool_version: "0.1.0".to_string(),
                    tool_git_sha: "0".repeat(40),
                    dataset_doi: None,
                    withdrawn: None,
                }],
            }
        })
        .collect();
    releases.reverse(); // index.validate() requires newest first

    OdsReleaseIndex {
        schema: RELEASES_SCHEMA_V1_URL.to_string(),
        trud_signing_key_fingerprints: vec!["71ED5964BAE53E83556320A42BE59DADEE84BEB0".to_string()],
        mirrors: vec![
            MirrorEntry {
                url: "https://ods.fyi/v2/ods-data".to_string(),
            },
            MirrorEntry {
                url: "https://ghcr.io/v2/olizilla/ods-data".to_string(),
            },
        ],
        releases,
    }
}

#[test]
fn closed_stdout_pipe_exits_quietly() {
    let index = make_large_index(3000);
    index.validate().expect("fixture index is valid");

    let tmp = tempfile::TempDir::new().expect("temp dir for index file");
    let index_path = tmp.path().join("releases.json");
    std::fs::write(&index_path, serde_json::to_string(&index).unwrap()).expect("write index");

    // `cmd` is a local (not a temporary in the `.spawn()` chain) so its cwd/HOME
    // temp dirs stay alive for the child's whole run, not just until `spawn()`
    // returns — otherwise they're deleted out from under the still-running child.
    let mut cmd = common::ods_binary();
    cmd.arg("pull")
        .arg("--list")
        .arg("--index")
        .arg(&index_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn ods pull --list");

    {
        let mut stdout = child.stdout.take().expect("stdout piped");
        let mut first_bytes = [0u8; 64];
        stdout
            .read_exact(&mut first_bytes)
            .expect("read first bytes of output");
        // Dropping `stdout` here closes our end of the pipe while `ods` is still
        // writing the rest of a >64 KB table — the `ods pull --list | head` case.
    }

    let output = child.wait_with_output().expect("wait for ods to exit");

    assert!(
        output.status.success(),
        "expected exit 0 on a closed stdout pipe, got {:?} (stderr: {})",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "expected empty stderr on a closed stdout pipe, got: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

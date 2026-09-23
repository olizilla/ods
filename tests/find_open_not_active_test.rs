//! Acceptance tests for `.agents/briefs/open-not-just-active.md` Task 1.
//!
//! `ods find` covers what is open — active, and not past its legal end as of the
//! release date — not NHS's `active`, which stays true while a legally dissolved
//! organisation's suppliers migrate. Protects `docs/tests.md` Q7.
//!
//! These need the real 2026-08-28 release (`ods_data/releases/2026-08-28`, gitignored,
//! not present on a fresh clone), so they're `#[ignore]`d; run with
//! `cargo test --test find_open_not_active_test -- --include-ignored`, or
//! `scripts/ci.sh --release-data`.

use std::path::PathBuf;
use std::process::Command;

fn ods_binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ods"))
}

/// The maintainer's real, already-built 2026-08-28 release, or `None` if this
/// checkout doesn't have one. `ods_data/` is gitignored and per-checkout, so a
/// worktree that isn't the maintainer's own main checkout won't have it unless
/// `ODS_REAL_RELEASE_DIR` names where it lives.
fn real_release_dir() -> Option<PathBuf> {
    let candidate = std::env::var("ODS_REAL_RELEASE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("ods_data")
                .join("releases")
                .join("2026-08-28")
        });
    candidate.join("orgs.parquet").exists().then_some(candidate)
}

fn run_find(args: &[&str], release_dir: &std::path::Path) -> (String, String, bool) {
    let output = ods_binary()
        .arg("find")
        .args(args)
        .arg("-i")
        .arg(release_dir)
        .output()
        .expect("run ods find");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.success(),
    )
}

#[test]
#[ignore]
fn find_gp_count_is_unchanged_by_the_open_filter() {
    let Some(release) = real_release_dir() else {
        panic!("needs the real 2026-08-28 release at ods_data/releases/2026-08-28");
    };
    let (stdout, stderr, ok) = run_find(&["--gp"], &release);
    assert!(ok, "stderr: {stderr}");
    assert!(
        stdout.contains("7941"),
        "GP Practice holds no legally-closed records, so its count is unchanged, got:\n{stdout}"
    );
}

#[test]
#[ignore]
fn find_role_ro197_drops_to_the_open_count() {
    let Some(release) = real_release_dir() else {
        panic!("needs the real 2026-08-28 release at ods_data/releases/2026-08-28");
    };
    let (stdout, stderr, ok) = run_find(&["--role", "RO197"], &release);
    assert!(ok, "stderr: {stderr}");
    assert!(
        stdout.contains("202"),
        "NHS Trust goes from 247 to 202 once its legally-closed holders drop out, got:\n{stdout}"
    );
    assert!(!stdout.contains("247"), "the old, active-only count must not still appear, got:\n{stdout}");
}

#[test]
#[ignore]
fn find_code_north_bristol_is_not_open_but_all_shows_it() {
    let Some(release) = real_release_dir() else {
        panic!("needs the real 2026-08-28 release at ods_data/releases/2026-08-28");
    };

    let (stdout, stderr, ok) = run_find(&["--code", "RVJ"], &release);
    assert!(ok, "stderr: {stderr}");
    assert!(
        !stdout.contains("NORTH BRISTOL"),
        "legally dissolved 2026-06-30, before the 2026-08-28 release: not open, got:\n{stdout}"
    );
    assert!(
        stdout.contains("0 open · 1 legally closed") && stdout.contains("Use --all to see it"),
        "the footer names the one thing it held back, singular, got:\n{stdout}"
    );

    let (stdout_all, stderr_all, ok_all) = run_find(&["--code", "RVJ", "--all"], &release);
    assert!(ok_all, "stderr: {stderr_all}");
    assert!(
        stdout_all.contains("NORTH BRISTOL"),
        "--all is the unfiltered table, so it still shows it, got:\n{stdout_all}"
    );
}

#[test]
#[ignore]
fn find_sql_matches_find_for_the_open_filter() {
    let Some(release) = real_release_dir() else {
        panic!("needs the real 2026-08-28 release at ods_data/releases/2026-08-28");
    };
    if Command::new("duckdb").arg("--version").output().is_err() {
        eprintln!("Skipping: duckdb CLI not found on PATH");
        return;
    }

    for args in [vec!["--gp"], vec!["--role", "RO197"]] {
        let mut sql_args: Vec<&str> = args.clone();
        sql_args.push("--sql");
        let (sql, sql_stderr, sql_ok) = run_find(&sql_args, &release);
        assert!(sql_ok, "stderr: {sql_stderr}");
        assert!(
            sql.contains("(legal_end IS NULL OR legal_end > trud_release_date)"),
            "the open clause must appear beside status = 'active', got:\n{sql}"
        );

        let mut csv_args = args.clone();
        csv_args.push("--format");
        csv_args.push("csv");
        let (csv, csv_stderr, csv_ok) = run_find(&csv_args, &release);
        assert!(csv_ok, "stderr: {csv_stderr}");
        let find_codes: std::collections::BTreeSet<&str> = csv
            .lines()
            .skip(1)
            .filter_map(|l| l.split(',').next())
            .collect();

        let duckdb_out = std::process::Command::new("duckdb")
            .arg("-csv")
            .arg("-noheader")
            .output_with_stdin(sql.as_bytes());
        let duck_codes: std::collections::BTreeSet<&str> = std::str::from_utf8(&duckdb_out)
            .expect("duckdb utf8")
            .lines()
            .map(|l| l.split(',').next().unwrap_or(""))
            .collect();

        assert_eq!(
            find_codes, duck_codes,
            "for {:?}, find and --sql | duckdb must return the same row set",
            args
        );
    }
}

/// A small extension trait so the duckdb-piping helper above reads in one expression.
trait OutputWithStdin {
    fn output_with_stdin(&mut self, stdin: &[u8]) -> Vec<u8>;
}

impl OutputWithStdin for Command {
    fn output_with_stdin(&mut self, stdin: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut child = self
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("spawn duckdb");
        child.stdin.take().unwrap().write_all(stdin).expect("write duckdb stdin");
        let out = child.wait_with_output().expect("wait duckdb");
        out.stdout
    }
}

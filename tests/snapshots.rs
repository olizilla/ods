mod common;

use common::setup_find_test_workspace;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::thread;

struct TestCase {
    cmd_str: &'static str,
    columns: Option<usize>,
    args: Vec<&'static str>,
    use_input: bool,
}

fn run_case_full(
    case: &TestCase,
    input_dir: Option<&Path>,
    cwd: Option<&Path>,
    envs: &[(&str, &str)],
) -> String {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ods"));
    if let Some(cols) = case.columns {
        cmd.env("COLUMNS", cols.to_string());
    }
    cmd.env_remove("PAGER");
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    for (k, v) in envs {
        cmd.env(k, v);
    }

    for (i, arg) in case.args.iter().enumerate() {
        cmd.arg(arg);
        if i == 0 && case.use_input {
            if let Some(dir) = input_dir {
                cmd.arg("--input").arg(dir);
            }
        }
    }

    let output = cmd.output().expect("execute ods command");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let exit_code = output.status.code().unwrap_or(0);

    let mut s = format!("$ {}\n", case.cmd_str);
    if !stdout.is_empty() {
        s.push_str(&stdout);
        if !stdout.ends_with('\n') {
            s.push('\n');
        }
    }
    if !stderr.is_empty() {
        s.push_str("--- stderr\n");
        s.push_str(&stderr);
        if !stderr.ends_with('\n') {
            s.push('\n');
        }
    }
    if exit_code != 0 {
        s.push_str(&format!("--- exit {}\n", exit_code));
    }
    s
}

fn run_case(case: &TestCase, input_dir: Option<&Path>) -> String {
    run_case_full(case, input_dir, None, &[])
}

fn run_mock_trud_server(response_body: Vec<u8>) -> (String, mpsc::Sender<()>) {
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
                let mut buf = [0u8; 2048];
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
    (format!("http://127.0.0.1:{}", port), tx)
}

fn parse_cases(content: &str) -> Vec<String> {
    let mut cases = Vec::new();
    let mut current_case = String::new();

    for line in content.lines() {
        if line.starts_with("$ ") && !current_case.is_empty() {
            cases.push(current_case);
            current_case = String::new();
        }
        current_case.push_str(line);
        current_case.push('\n');
    }
    if !current_case.is_empty() {
        cases.push(current_case);
    }
    cases
}

fn check_snapshot(file_name: &str, actual_cases: &[String], case_names: &[&str]) {
    let snapshot_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots");
    let file_path = snapshot_dir.join(file_name);

    if std::env::var("UPDATE_EXPECT").is_ok() {
        fs::create_dir_all(&snapshot_dir).expect("create snapshot dir");
        let mut full = String::new();
        for (i, case) in actual_cases.iter().enumerate() {
            if i > 0 {
                full.push('\n');
            }
            full.push_str(case);
        }
        fs::write(&file_path, full).expect("write snapshot file");
        return;
    }

    let expected_content = fs::read_to_string(&file_path).unwrap_or_else(|e| {
        panic!(
            "Failed to read snapshot file {}: {}. Run with UPDATE_EXPECT=1 to generate.",
            file_path.display(),
            e
        );
    });

    let expected_cases = parse_cases(&expected_content);
    assert_eq!(
        expected_cases.len(),
        actual_cases.len(),
        "Snapshot case count mismatch in {}: expected {} cases, got {}",
        file_path.display(),
        expected_cases.len(),
        actual_cases.len()
    );

    for (i, ((actual, expected), name)) in actual_cases
        .iter()
        .zip(&expected_cases)
        .zip(case_names)
        .enumerate()
    {
        if actual.trim_end() != expected.trim_end() {
            panic!(
                "Snapshot mismatch in {} for case '{}' (case #{}):\n=== Expected ===\n{}\n=== Actual ===\n{}",
                file_path.display(),
                name,
                i + 1,
                expected,
                actual
            );
        }
    }
}

#[test]
fn snapshot_help() {
    let cases = [
        TestCase {
            cmd_str: "ods --help",
            columns: Some(100),
            args: vec!["--help"],
            use_input: false,
        },
        TestCase {
            cmd_str: "ods make --help",
            columns: Some(100),
            args: vec!["make", "--help"],
            use_input: false,
        },
        TestCase {
            cmd_str: "ods trud --help",
            columns: Some(100),
            args: vec!["trud", "--help"],
            use_input: false,
        },
        TestCase {
            cmd_str: "ods trud pull --help",
            columns: Some(100),
            args: vec!["trud", "pull", "--help"],
            use_input: false,
        },
        TestCase {
            cmd_str: "ods trud list --help",
            columns: Some(100),
            args: vec!["trud", "list", "--help"],
            use_input: false,
        },
        TestCase {
            cmd_str: "ods find --help",
            columns: Some(100),
            args: vec!["find", "--help"],
            use_input: false,
        },
    ];

    let case_names: Vec<&str> = cases.iter().map(|c| c.cmd_str).collect();
    let actual_cases: Vec<String> = cases.iter().map(|c| run_case(c, None)).collect();

    check_snapshot("help.txt", &actual_cases, &case_names);
}

#[test]
fn snapshot_role() {
    let (_tmp, parquet_dir) = setup_find_test_workspace();

    let cases = [
        TestCase {
            cmd_str: "COLUMNS=100 ods role 'general practice'",
            columns: Some(100),
            args: vec!["role", "general practice"],
            use_input: true,
        },
        TestCase {
            cmd_str: "COLUMNS=100 ods role GP",
            columns: Some(100),
            args: vec!["role", "GP"],
            use_input: true,
        },
    ];

    let case_names: Vec<&str> = cases.iter().map(|c| c.cmd_str).collect();
    let actual_cases: Vec<String> = cases.iter().map(|c| run_case(c, Some(&parquet_dir))).collect();

    check_snapshot("role.txt", &actual_cases, &case_names);
}

#[test]
fn snapshot_find() {
    let (_tmp, parquet_dir) = setup_find_test_workspace();

    let cases = [
        TestCase {
            cmd_str: "COLUMNS=100 ods find --in sedbergh --sort code",
            columns: Some(100),
            args: vec!["find", "--in", "sedbergh", "--sort", "code"],
            use_input: true,
        },
        TestCase {
            cmd_str: "COLUMNS=100 ods find sedbergh --gp --in cumbria",
            columns: Some(100),
            args: vec!["find", "sedbergh", "--gp", "--in", "cumbria"],
            use_input: true,
        },
        TestCase {
            cmd_str: "COLUMNS=100 ods find FAH",
            columns: Some(100),
            args: vec!["find", "FAH"],
            use_input: true,
        },
        TestCase {
            cmd_str: "COLUMNS=100 ods find",
            columns: Some(100),
            args: vec!["find"],
            use_input: false,
        },
        TestCase {
            cmd_str: "COLUMNS=100 ods find --role 'General Practice'",
            columns: Some(100),
            args: vec!["find", "--role", "General Practice"],
            use_input: true,
        },
        TestCase {
            cmd_str: "COLUMNS=100 ods find 'SEDBERGH MEDICAL PRACTICE'",
            columns: Some(100),
            args: vec!["find", "SEDBERGH MEDICAL PRACTICE"],
            use_input: true,
        },
        TestCase {
            cmd_str: "COLUMNS=100 ods find 'SEDBERGH MEDICAL PRACTICE' --verbose",
            columns: Some(100),
            args: vec!["find", "SEDBERGH MEDICAL PRACTICE", "--verbose"],
            use_input: true,
        },
        TestCase {
            cmd_str: "COLUMNS=100 ods find --code RJZ",
            columns: Some(100),
            args: vec!["find", "--code", "RJZ"],
            use_input: true,
        },
        TestCase {
            cmd_str: "COLUMNS=100 ods find --code A85619",
            columns: Some(100),
            args: vec!["find", "--code", "A85619"],
            use_input: true,
        },
        TestCase {
            cmd_str: "COLUMNS=100 ods find CLWYD --all --sort code",
            columns: Some(100),
            args: vec!["find", "CLWYD", "--all", "--sort", "code"],
            use_input: true,
        },
        TestCase {
            cmd_str: "COLUMNS=100 ods find sedbergh",
            columns: Some(100),
            args: vec!["find", "sedbergh"],
            use_input: true,
        },
        TestCase {
            cmd_str: "COLUMNS=100 ods find sedbergh --all",
            columns: Some(100),
            args: vec!["find", "sedbergh", "--all"],
            use_input: true,
        },
        TestCase {
            cmd_str: "COLUMNS=100 ods find --code A82608",
            columns: Some(100),
            args: vec!["find", "--code", "A82608"],
            use_input: true,
        },
        TestCase {
            cmd_str: "COLUMNS=100 ods find nonexistentquery12345",
            columns: Some(100),
            args: vec!["find", "nonexistentquery12345"],
            use_input: true,
        },
        TestCase {
            cmd_str: "COLUMNS=40 ods find sedbergh",
            columns: Some(40),
            args: vec!["find", "sedbergh"],
            use_input: true,
        },
        TestCase {
            cmd_str: "COLUMNS=100 ods find --in SW1",
            columns: Some(100),
            args: vec!["find", "--in", "SW1"],
            use_input: true,
        },
        TestCase {
            cmd_str: "COLUMNS=100 ods find --in E1",
            columns: Some(100),
            args: vec!["find", "--in", "E1"],
            use_input: true,
        },
    ];

    let case_names: Vec<&str> = cases.iter().map(|c| c.cmd_str).collect();
    let actual_cases: Vec<String> = cases.iter().map(|c| run_case(c, Some(&parquet_dir))).collect();

    check_snapshot("find.txt", &actual_cases, &case_names);
}

#[test]
fn snapshot_trud_list() {
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/trud_releases_response.json");
    let fixture_bytes = fs::read(&fixture_path).expect("read trud_releases_response.json");
    let (api_url, stop_server) = run_mock_trud_server(fixture_bytes);

    let tmp = tempfile::TempDir::new().expect("create tempdir");
    let rel_dir = tmp.path().join("ods_data/releases/2026-07-31/trud");
    fs::create_dir_all(&rel_dir).expect("create release dir");
    fs::write(
        rel_dir.join("hscorgrefdataxml_data_7.0.0_20260731000001.zip"),
        b"dummy zip content",
    )
    .expect("write dummy zip");

    let cases = [
        TestCase {
            cmd_str: "COLUMNS=100 ods trud list",
            columns: Some(100),
            args: vec!["trud", "list"],
            use_input: false,
        },
        TestCase {
            cmd_str: "COLUMNS=100 ods trud list --all",
            columns: Some(100),
            args: vec!["trud", "list", "--all"],
            use_input: false,
        },
    ];

    let case_names: Vec<&str> = cases.iter().map(|c| c.cmd_str).collect();
    let actual_cases: Vec<String> = cases
        .iter()
        .map(|c| {
            run_case_full(
                c,
                None,
                Some(tmp.path()),
                &[("TRUD_API_KEY", "test"), ("ODS_TRUD_API_URL", &api_url)],
            )
        })
        .collect();

    let _ = stop_server.send(());
    check_snapshot("trud-list.txt", &actual_cases, &case_names);
}

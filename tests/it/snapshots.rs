use crate::common;

use common::{make_v1_index, setup_cite_case_workspace, setup_find_test_workspace, setup_test_release_for_cite};
use sha2::Digest;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
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
    let mut cmd = common::ods_cmd();
    if let Some(cols) = case.columns {
        cmd.env("COLUMNS", cols.to_string());
    }
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

fn extract_http_path(buf: &[u8]) -> Option<String> {
    let line_end = buf.iter().position(|&b| b == b'\r' || b == b'\n')?;
    let line_str = std::str::from_utf8(&buf[..line_end]).ok()?;
    let mut parts = line_str.split_whitespace();
    let _method = parts.next()?;
    let full_path = parts.next()?;
    let path = full_path.split('?').next().unwrap_or(full_path);
    Some(path.to_string())
}

fn run_mock_trud_server(
    mut routes: std::collections::HashMap<String, Vec<u8>>,
) -> (String, mpsc::Sender<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let port = listener.local_addr().unwrap().port();
    let base_url = format!("http://127.0.0.1:{}", port);

    for body in routes.values_mut() {
        if let Ok(s) = std::str::from_utf8(body) {
            if s.contains("<BASE_URL>") {
                let replaced = s.replace("<BASE_URL>", &base_url);
                *body = replaced.into_bytes();
            }
        }
    }

    let routes = std::sync::Arc::new(routes);
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        loop {
            if rx.try_recv().is_ok() {
                break;
            }
            if let Ok((mut stream, _)) = listener.accept() {
                let _ = stream.set_nonblocking(false);
                let routes = routes.clone();
                thread::spawn(move || {
                    let mut buf = [0u8; 4096];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    if n > 0 {
                        if let Some(path) = extract_http_path(&buf[..n]) {
                            if let Some(body) = routes.get(&path) {
                                let content_type =
                                    if path.ends_with(".json") || path.contains("/releases") {
                                        "application/json"
                                    } else {
                                        "application/octet-stream"
                                    };
                                let header = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: {}\r\nConnection: close\r\n\r\n",
                                    body.len(),
                                    content_type
                                );
                                let _ = stream.write_all(header.as_bytes());
                                let _ = stream.write_all(body);
                                let _ = stream.flush();
                            } else {
                                let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                                let _ = stream.write_all(resp.as_bytes());
                                let _ = stream.flush();
                            }
                        }
                    }
                });
            }
            thread::sleep(std::time::Duration::from_millis(5));
        }
    });
    (base_url, tx)
}

fn is_duration_str(s: &str) -> bool {
    if s.is_empty() || !s.ends_with('s') {
        return false;
    }
    if !s.chars().next().unwrap().is_ascii_digit() {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_digit() || c == '.' || c == 'm' || c == 'h' || c == 's')
}

fn normalise_durations(text: &str) -> String {
    let mut result = String::new();
    for line in text.lines() {
        let mut new_line = String::new();
        let mut rest = line;
        while let Some(pos) = rest.find(" in ") {
            new_line.push_str(&rest[..pos + 4]); // includes " in "
            let after = &rest[pos + 4..];
            let end = after.find(|c: char| c.is_whitespace()).unwrap_or(after.len());
            let token = &after[..end];
            if is_duration_str(token) {
                new_line.push_str("<duration>");
                rest = &after[end..];
            } else {
                new_line.push_str(token);
                rest = &after[end..];
            }
        }
        new_line.push_str(rest);
        result.push_str(&new_line);
        result.push('\n');
    }
    if !text.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }
    result
}

fn normalise_snapshot(text: &str) -> String {
    normalise_durations(text)
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

    let actual_cases: Vec<String> = actual_cases.iter().map(|s| normalise_snapshot(s)).collect();

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

    let expected_cases: Vec<String> = parse_cases(&expected_content)
        .into_iter()
        .map(|s| normalise_snapshot(&s))
        .collect();
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
    let mut routes = std::collections::HashMap::new();
    routes.insert("/keys/test/items/341/releases".to_string(), fixture_bytes);
    let (api_url, stop_server) = run_mock_trud_server(routes);

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

/// `ods trud list --all --format json` is a machine format (O2): each item carries TRUD's
/// `sha256` (Task 1 of ci-builds-datasets.md), so a run's own listing can be turned into a
/// release-index row without a second call.
#[test]
fn trud_list_json_carries_sha256() {
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/trud_releases_response.json");
    let fixture_bytes = fs::read(&fixture_path).expect("read trud_releases_response.json");
    let mut routes = std::collections::HashMap::new();
    routes.insert("/keys/test/items/341/releases".to_string(), fixture_bytes);
    let (api_url, stop_server) = run_mock_trud_server(routes);

    let tmp = tempfile::TempDir::new().expect("create tempdir");

    let mut cmd = common::ods_cmd();
    cmd.current_dir(tmp.path());
    cmd.env("TRUD_API_KEY", "test");
    cmd.env("ODS_TRUD_API_URL", &api_url);
    cmd.args(["trud", "list", "--all", "--format", "json"]);
    let output = cmd.output().expect("execute ods trud list --all --format json");
    let _ = stop_server.send(());

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let first = &items[0];
    assert_eq!(first["date"], "2026-07-31");
    assert_eq!(first["sha256"], "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933");
    assert_eq!(first["size_bytes"], 37983173);
    assert_eq!(first["status"], "remote");
    // Field order: date, sha256, size_bytes, status
    let keys: Vec<&str> = first.as_object().unwrap().keys().map(|s| s.as_str()).collect();
    assert_eq!(keys, vec!["date", "sha256", "size_bytes", "status"]);
}

fn create_padded_zip(date_str: &str, target_size: usize) -> Vec<u8> {
    let xml_content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<un:OrganisationManifest xmlns:un="http://refdata.hscic.gov.uk/org/v2-0-0">
  <un:ManifestHeader>
    <un:PublicationType value="Full" />
    <un:PublicationDate value="{date_str}" />
    <un:PublicationSeqNum value="4700" />
    <un:PublicationSource value="HSCIC" />
    <un:RecordCount value="1" />
  </un:ManifestHeader>
</un:OrganisationManifest>"#
    );

    let build_zip = |pad_bytes: &[u8]| -> Vec<u8> {
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);

            zip.start_file("HSCOrgRefData_Full.xml", options).unwrap();
            std::io::Write::write_all(&mut zip, xml_content.as_bytes()).unwrap();

            zip.start_file("padding.bin", options).unwrap();
            std::io::Write::write_all(&mut zip, pad_bytes).unwrap();

            zip.finish().unwrap();
        }
        cursor.into_inner()
    };

    let base_zip = build_zip(&[]);
    let base_len = base_zip.len();
    assert!(target_size >= base_len);
    let pad_len = target_size - base_len;
    let pad: Vec<u8> = (0..pad_len).map(|i| (i % 251) as u8).collect();
    let final_zip = build_zip(&pad);
    assert_eq!(final_zip.len(), target_size);
    final_zip
}

#[test]
fn snapshot_trud_pull() {
    let tmp = tempfile::TempDir::new().expect("create tempdir");

    // Two releases: 2026-07-31 (3MB) and 2026-06-26 (2MB)
    let archive_3mb = create_padded_zip("2026-07-31", 3_145_728);
    let sha_3mb = format!("{:x}", sha2::Sha256::digest(&archive_3mb));

    let archive_2mb = create_padded_zip("2026-06-26", 2_097_152);
    let sha_2mb = format!("{:x}", sha2::Sha256::digest(&archive_2mb));

    let checksum_content = b"<FCIV><FILE_ENTRY><name>archive.zip</name></FILE_ENTRY></FCIV>".to_vec();
    let sig_content = b"-----BEGIN PGP SIGNATURE-----\nmock\n-----END PGP SIGNATURE-----".to_vec();
    let pubkey_content = b"-----BEGIN PGP PUBLIC KEY BLOCK-----\nmock\n-----END PGP PUBLIC KEY BLOCK-----".to_vec();

    let releases_json = serde_json::json!({
        "apiVersion": "1",
        "releases": [
            {
                "id": "hscorgrefdataxml_data_7.0.0_20260731000001.zip",
                "name": "Release 7.0.0",
                "releaseDate": "2026-07-31",
                "archiveFileUrl": "<BASE_URL>/files/2026-07-31/archive.zip",
                "archiveFileName": "hscorgrefdataxml_data_7.0.0_20260731000001.zip",
                "archiveFileSizeBytes": 3_145_728,
                "archiveFileSha256": sha_3mb,
                "checksumFileUrl": "<BASE_URL>/files/2026-07-31/checksum.xml",
                "checksumFileName": "trud_hscorgrefdataxml_data_7.0.0_20260731000001.xml",
                "checksumFileSizeBytes": checksum_content.len(),
                "signatureFileUrl": "<BASE_URL>/files/2026-07-31/signature.asc",
                "signatureFileName": "trud_hscorgrefdataxml_data_7.0.0_20260731000001.sig",
                "signatureFileSizeBytes": sig_content.len(),
                "publicKeyFileUrl": "<BASE_URL>/files/2026-07-31/public_key.pgp",
                "publicKeyFileName": "trud-public-key-2013-04-01.pgp",
                "publicKeyFileSizeBytes": pubkey_content.len()
            },
            {
                "id": "hscorgrefdataxml_data_6.0.0_20260626000001.zip",
                "name": "Release 6.0.0",
                "releaseDate": "2026-06-26",
                "archiveFileUrl": "<BASE_URL>/files/2026-06-26/archive.zip",
                "archiveFileName": "hscorgrefdataxml_data_6.0.0_20260626000001.zip",
                "archiveFileSizeBytes": 2_097_152,
                "archiveFileSha256": sha_2mb,
                "checksumFileUrl": "<BASE_URL>/files/2026-06-26/checksum.xml",
                "checksumFileName": "trud_hscorgrefdataxml_data_6.0.0_20260626000001.xml",
                "checksumFileSizeBytes": checksum_content.len(),
                "signatureFileUrl": "<BASE_URL>/files/2026-06-26/signature.asc",
                "signatureFileName": "trud_hscorgrefdataxml_data_6.0.0_20260626000001.sig",
                "signatureFileSizeBytes": sig_content.len(),
                "publicKeyFileUrl": "<BASE_URL>/files/2026-06-26/public_key.pgp",
                "publicKeyFileName": "trud-public-key-2013-04-01.pgp",
                "publicKeyFileSizeBytes": pubkey_content.len()
            }
        ]
    });

    let mut routes = std::collections::HashMap::new();
    routes.insert(
        "/keys/test/items/341/releases".to_string(),
        serde_json::to_vec(&releases_json).unwrap(),
    );
    routes.insert("/files/2026-07-31/archive.zip".to_string(), archive_3mb);
    routes.insert("/files/2026-07-31/checksum.xml".to_string(), checksum_content.clone());
    routes.insert("/files/2026-07-31/signature.asc".to_string(), sig_content.clone());
    routes.insert("/files/2026-07-31/public_key.pgp".to_string(), pubkey_content.clone());

    routes.insert("/files/2026-06-26/archive.zip".to_string(), archive_2mb);
    routes.insert("/files/2026-06-26/checksum.xml".to_string(), checksum_content);
    routes.insert("/files/2026-06-26/signature.asc".to_string(), sig_content);
    routes.insert("/files/2026-06-26/public_key.pgp".to_string(), pubkey_content);

    let (api_url, stop_server) = run_mock_trud_server(routes);

    let cases = [
        TestCase {
            cmd_str: "COLUMNS=100 ods trud pull 2026-06-26",
            columns: Some(100),
            args: vec!["trud", "pull", "2026-06-26"],
            use_input: false,
        },
        TestCase {
            cmd_str: "COLUMNS=100 ods trud pull",
            columns: Some(100),
            args: vec!["trud", "pull"],
            use_input: false,
        },
        TestCase {
            cmd_str: "COLUMNS=100 ods trud pull",
            columns: Some(100),
            args: vec!["trud", "pull"],
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
    check_snapshot("trud-pull.txt", &actual_cases, &case_names);
}

/// Manifest digest, manifest bytes, `(title, digest, bytes)` per layer, and the layer sizes'
/// sum — what `build_padded_oci_release` returns.
type PaddedOciRelease = (String, Vec<u8>, Vec<(String, String, Vec<u8>)>, u64);

/// `len` characters of hex that zstd can't squeeze much: a SHA-256 chain from `seed`. What a
/// stub table needs to hold for its file to be about as big as a real one.
fn incompressible(seed: &str, len: usize) -> String {
    let mut out = String::with_capacity(len + 64);
    let mut block = sha2::Sha256::digest(seed.as_bytes());
    while out.len() < len {
        out.push_str(&format!("{:x}", block));
        block = sha2::Sha256::digest(block);
    }
    out.truncate(len);
    out
}

/// Builds a real release directory — the four Parquet files, carrying the release's
/// provenance, `orgs.parquet` padded so the whole is about `target_total` bytes and the bar
/// has something to move over — and its OCI manifest, the way `ods make oci` would.
fn build_padded_oci_release(
    dir: &Path,
    date: &str,
    version: &str,
    target_total: usize,
) -> PaddedOciRelease {
    let write = |name: &str, content: &str| {
        common::write_fixture_parquet(
            &dir.join(name),
            date,
            "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
            version,
            content,
        )
    };
    write("roles.parquet", &format!("roles parquet for {}", date));
    write("relationships.parquet", &format!("relationships parquet for {}", date));
    write("successions.parquet", &format!("successions parquet for {}", date));
    // Hex packs two characters a byte once compressed.
    write("orgs.parquet", &incompressible(&format!("orgs parquet for {}", date), target_total * 2));

    let (manifest, manifest_bytes) = ods::commands::make_oci::build_manifest_from_dir(dir).unwrap();
    let manifest_digest = manifest.digest().unwrap();
    let total_size: u64 = manifest.layers.iter().map(|l| l.size).sum();

    let mut files = Vec::new();
    for layer in &manifest.layers {
        let title = layer
            .annotations
            .as_ref()
            .and_then(|a| a.get("org.opencontainers.image.title"))
            .cloned()
            .unwrap();
        let bytes = fs::read(dir.join(&title)).unwrap();
        files.push((title, layer.digest.clone(), bytes));
    }

    (manifest_digest, manifest_bytes, files, total_size)
}

#[test]
fn snapshot_pull() {
    let tmp = tempfile::TempDir::new().expect("create tempdir");
    let fresh_tmp = tempfile::TempDir::new().expect("create tempdir");

    let fix_dir_1 = tmp.path().join("fixture_2026-06-26");
    fs::create_dir_all(&fix_dir_1).unwrap();
    let (digest1, manifest_bytes1, files1, total1) = build_padded_oci_release(&fix_dir_1, "2026-06-26", "0.1.0", 2_097_152);

    let fix_dir_2 = tmp.path().join("fixture_2026-07-31");
    fs::create_dir_all(&fix_dir_2).unwrap();
    let (digest2, manifest_bytes2, files2, total2) = build_padded_oci_release(&fix_dir_2, "2026-07-31", "0.1.0", 3_145_728);

    let mut routes = std::collections::HashMap::new();
    routes.insert(format!("/v2/ods-data/manifests/{}", digest1), manifest_bytes1);
    for (_, d, b) in &files1 {
        routes.insert(format!("/v2/ods-data/blobs/{}", d), b.clone());
    }
    routes.insert(format!("/v2/ods-data/manifests/{}", digest2), manifest_bytes2);
    for (_, d, b) in &files2 {
        routes.insert(format!("/v2/ods-data/blobs/{}", d), b.clone());
    }

    let (api_url, stop_server) = run_mock_trud_server(routes);
    // `MirrorEntry::host` drops the port, so serving from `localhost` (not `127.0.0.1`) is
    // what makes `from localhost` in the settled block stable across machines.
    let port = api_url.rsplit(':').next().unwrap();
    let mirror_url = format!("http://localhost:{}/v2/ods-data", port);

    let mut index = make_v1_index(&[
        (
            "2026-07-31",
            "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
            total2,
            &[("0.1.0", &digest2)],
        ),
        (
            "2026-06-26",
            "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
            total1,
            &[("0.1.0", &digest1)],
        ),
    ]);
    index.mirrors = vec![ods::index::MirrorEntry { url: mirror_url }];
    index.releases[0].datasets[0].dataset_filesize_bytes = total2;
    index.releases[1].datasets[0].dataset_filesize_bytes = total1;

    let index_bytes = serde_json::to_vec_pretty(&index).unwrap();
    fs::write(tmp.path().join("rehearsal.json"), &index_bytes).unwrap();
    fs::write(fresh_tmp.path().join("rehearsal.json"), &index_bytes).unwrap();

    let cases = [
        TestCase {
            cmd_str: "COLUMNS=100 ods pull 2026-06-26 --index rehearsal.json",
            columns: Some(100),
            args: vec!["pull", "2026-06-26", "--index", "rehearsal.json"],
            use_input: false,
        },
        TestCase {
            cmd_str: "COLUMNS=100 ods pull --index rehearsal.json",
            columns: Some(100),
            args: vec!["pull", "--index", "rehearsal.json"],
            use_input: false,
        },
        TestCase {
            cmd_str: "COLUMNS=100 ods pull --index rehearsal.json",
            columns: Some(100),
            args: vec!["pull", "--index", "rehearsal.json"],
            use_input: false,
        },
    ];

    let mut case_names: Vec<&str> = cases.iter().map(|c| c.cmd_str).collect();
    let mut actual_cases: Vec<String> = cases.iter().map(|c| run_case_full(c, None, Some(tmp.path()), &[])).collect();

    let all_case = TestCase {
        cmd_str: "COLUMNS=100 ods pull --all --index rehearsal.json",
        columns: Some(100),
        args: vec!["pull", "--all", "--index", "rehearsal.json"],
        use_input: false,
    };
    actual_cases.push(run_case_full(&all_case, None, Some(fresh_tmp.path()), &[]));
    case_names.push(all_case.cmd_str);

    let _ = stop_server.send(());
    check_snapshot("pull.txt", &actual_cases, &case_names);
}

#[test]
fn snapshot_make() {
    let tmp = tempfile::TempDir::new().expect("create tempdir");
    let zip = "hscorgrefdataxml_data_7.0.0_20260731000001.zip";
    common::create_mock_trud_zip(tmp.path(), zip);

    // Piped, so the report prints once, settled, and the warnings follow it. This zip matches no
    // release index row, so building it at all takes --force.
    let cases = [
        TestCase {
            cmd_str: "COLUMNS=100 ods make --input hscorgrefdataxml_data_7.0.0_20260731000001.zip --output out",
            columns: Some(100),
            args: vec!["make", "--input", zip, "--output", "out"],
            use_input: false,
        },
        TestCase {
            cmd_str: "COLUMNS=100 ods make --input hscorgrefdataxml_data_7.0.0_20260731000001.zip --force",
            columns: Some(100),
            args: vec!["make", "--input", zip, "--force"],
            use_input: false,
        },
        TestCase {
            cmd_str: "COLUMNS=100 ods make --input hscorgrefdataxml_data_7.0.0_20260731000001.zip --output out --force",
            columns: Some(100),
            args: vec!["make", "--input", zip, "--output", "out", "--force"],
            use_input: false,
        },
        TestCase {
            cmd_str: "COLUMNS=100 ods make --input hscorgrefdataxml_data_7.0.0_20260731000001.zip --output out --force --verbose",
            columns: Some(100),
            args: vec!["make", "--input", zip, "--output", "out", "--force", "--verbose"],
            use_input: false,
        },
    ];

    let case_names: Vec<&str> = cases.iter().map(|c| c.cmd_str).collect();
    let actual_cases: Vec<String> = cases
        .iter()
        .map(|c| run_case_full(c, None, Some(tmp.path()), &[]))
        .collect();

    check_snapshot("make.txt", &actual_cases, &case_names);
}
#[test]
fn snapshot_cite() {
    let (_tmp, rel_dir) = setup_test_release_for_cite(None);

    let cases = [
        TestCase {
            cmd_str: "ods cite",
            columns: None,
            args: vec!["cite"],
            use_input: true,
        },
        TestCase {
            cmd_str: "ods cite -f apa",
            columns: None,
            args: vec!["cite", "-f", "apa"],
            use_input: true,
        },
        TestCase {
            cmd_str: "ods cite -f bibtex",
            columns: None,
            args: vec!["cite", "-f", "bibtex"],
            use_input: true,
        },
        TestCase {
            cmd_str: "ods cite -f csljson",
            columns: None,
            args: vec!["cite", "-f", "csljson"],
            use_input: true,
        },
        // Case A: Date unknown
        TestCase {
            cmd_str: "ods cite",
            columns: None,
            args: vec!["cite"],
            use_input: true,
        },
        // Case B: Different archive
        TestCase {
            cmd_str: "ods cite",
            columns: None,
            args: vec!["cite"],
            use_input: true,
        },
        // Case C: Version unpublished
        TestCase {
            cmd_str: "ods cite",
            columns: None,
            args: vec!["cite"],
            use_input: true,
        },
        // Case D: The files disagree about what release they are
        TestCase {
            cmd_str: "ods cite",
            columns: None,
            args: vec!["cite"],
            use_input: true,
        },
        // Case F: Different bytes
        TestCase {
            cmd_str: "ods cite",
            columns: None,
            args: vec!["cite"],
            use_input: true,
        },
    ];

    let case_names: Vec<&str> = cases.iter().map(|c| c.cmd_str).collect();
    let mut actual_cases: Vec<String> = cases[..4]
        .iter()
        .map(|c| run_case(c, Some(&rel_dir)))
        .collect();

    let (_tmp_a, dir_a) = setup_cite_case_workspace("A");
    actual_cases.push(run_case_full(&cases[4], Some(&dir_a), Some(_tmp_a.path()), &[]));

    let (_tmp_b, dir_b) = setup_cite_case_workspace("B");
    actual_cases.push(run_case_full(&cases[5], Some(&dir_b), Some(_tmp_b.path()), &[]));

    let (_tmp_c, dir_c) = setup_cite_case_workspace("C");
    actual_cases.push(run_case_full(&cases[6], Some(&dir_c), Some(_tmp_c.path()), &[]));

    let (_tmp_d, dir_d) = setup_cite_case_workspace("D");
    actual_cases.push(run_case_full(&cases[7], Some(&dir_d), Some(_tmp_d.path()), &[]));

    let (_tmp_f, dir_f) = setup_cite_case_workspace("F");
    actual_cases.push(run_case_full(&cases[8], Some(&dir_f), Some(_tmp_f.path()), &[]));

    check_snapshot("cite.txt", &actual_cases, &case_names);
}

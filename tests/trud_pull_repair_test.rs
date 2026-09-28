//! R4: a release directory heals from the zip it already holds, `--force` refreshes NHS's files
//! without downloading the zip again, and `--format ndjson` says which happened: `repaired`,
//! `refreshed`, `cached` or `failed`, and a failed repair exits 1 after the rest is done.
//! B2: `test_a_pull_with_a_key_leaves_it_out_of_every_file_and_line`, nothing `ods` writes or
//! prints holds the API key.

mod common;

use ods::commands::fetch::{run_with_fetcher, Args, TrudApiResponse, TrudFetcher, TrudReleaseItem};
use ods::progress::{Progress, ProgressCaps};
use ods::provenance::{compute_file_sha256, PullRecord, RecordLoad, LEGACY_PROVENANCE_FILENAME};
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

struct BufferWriter(Arc<Mutex<Vec<u8>>>);

impl Write for BufferWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A small zip for `date`, byte-identical every time it's built.
fn zip_bytes(date: &str, salt: &str) -> Vec<u8> {
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .last_modified_time(zip::DateTime::from_date_and_time(2026, 1, 1, 0, 0, 0).unwrap());
        zip.start_file(
            format!("trud_hscorgrefdataxml_data_7.0.0_{}000001.xml", date.replace('-', "")),
            options,
        )
        .unwrap();
        zip.write_all(format!("<hsctradingpartnerdesc date=\"{}\" salt=\"{}\"/>", date, salt).as_bytes())
            .unwrap();
        zip.finish().unwrap();
    }
    cursor.into_inner()
}

/// Stands in for TRUD, and counts what it was asked for.
struct Recorder {
    releases: Vec<TrudReleaseItem>,
    zips: HashMap<String, Vec<u8>>,
    archive_downloads: AtomicUsize,
    file_downloads: Mutex<Vec<String>>,
}

impl Recorder {
    fn archives(&self) -> usize {
        self.archive_downloads.load(Ordering::SeqCst)
    }
    fn files(&self) -> usize {
        self.file_downloads.lock().unwrap().len()
    }
}

impl TrudFetcher for Recorder {
    fn fetch_releases(&self) -> anyhow::Result<Vec<TrudReleaseItem>> {
        Ok(self.releases.clone())
    }

    fn download_archive(
        &self,
        url: &str,
        dest_path: &Path,
        on_bytes: &(dyn Fn(u64) + Send + Sync),
    ) -> anyhow::Result<()> {
        self.archive_downloads.fetch_add(1, Ordering::SeqCst);
        let bytes = &self.zips[url];
        fs::write(dest_path, bytes)?;
        on_bytes(bytes.len() as u64);
        Ok(())
    }

    fn download_file(&self, url: &str, dest_path: &Path) -> anyhow::Result<()> {
        self.file_downloads.lock().unwrap().push(url.to_string());
        fs::write(dest_path, format!("NHS's bytes for {url}"))?;
        Ok(())
    }
}

/// The first `count` releases of the fixture TRUD listing, each with a zip of our own so its
/// hash and size are ours to match or break.
fn recorder(count: usize) -> Recorder {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/trud_releases_response.json");
    let resp: TrudApiResponse = serde_json::from_str(&fs::read_to_string(fixture).unwrap()).unwrap();
    let mut releases = Vec::new();
    let mut zips = HashMap::new();
    for mut release in resp.releases.into_iter().take(count) {
        let bytes = zip_bytes(&release.release_date, "trud");
        release.archive_file_sha256 = sha256_of(&bytes);
        release.archive_file_size = bytes.len() as u64;
        release.download_url = format!("https://example.com/{}.zip", release.release_date);
        zips.insert(release.download_url.clone(), bytes);
        releases.push(release);
    }
    Recorder {
        releases,
        zips,
        archive_downloads: AtomicUsize::new(0),
        file_downloads: Mutex::new(Vec::new()),
    }
}

fn sha256_of(bytes: &[u8]) -> String {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("x");
    fs::write(&path, bytes).unwrap();
    compute_file_sha256(&path).unwrap()
}

fn trud_dir(ws: &Path, date: &str) -> PathBuf {
    ws.join("releases").join(date).join("trud")
}

/// Puts a release's zip where a pull would, with the `trud/_provenance.json` an older `ods`
/// wrote instead of `trud/datapackage.json`.
fn hold_zip(ws: &Path, release: &TrudReleaseItem, bytes: &[u8]) -> PathBuf {
    let dir = trud_dir(ws, &release.release_date);
    fs::create_dir_all(&dir).unwrap();
    let zip = dir.join(&release.archive_file_name);
    fs::write(&zip, bytes).unwrap();
    // An old mtime, so a rewritten zip can't pass for an untouched one
    let file = fs::OpenOptions::new().write(true).open(&zip).unwrap();
    file.set_modified(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_500_000_000))
        .unwrap();
    fs::write(
        dir.join(LEGACY_PROVENANCE_FILENAME),
        r#"{"$schema":"https://ods.fyi/schema/provenance.v1.json","trud_release_date":"old"}"#,
    )
    .unwrap();
    zip
}

struct Pulled {
    result: anyhow::Result<()>,
    output: String,
}

fn pull(ws: &Path, fetcher: &Recorder, date: Option<&str>, all: bool, force: bool) -> Pulled {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let caps = ProgressCaps { is_tty: false, no_color: true, quiet: false, verbose: false, width: 100 };
    let progress = Progress::new(caps, Box::new(BufferWriter(buffer.clone())));
    let args = Args {
        release_date: date.map(|d| d.to_string()),
        all,
        force,
        workspace: Some(ws.to_path_buf()),
        ..Default::default()
    };
    let result = run_with_fetcher(args, ws, fetcher, &progress);
    let output = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
    Pulled { result, output }
}

fn mtime(path: &Path) -> std::time::SystemTime {
    fs::metadata(path).unwrap().modified().unwrap()
}

fn readable_provenance(ws: &Path, date: &str) -> bool {
    let path = ods::provenance::pull_record_path(&ws.join("releases").join(date));
    matches!(PullRecord::load_from_file(&path), RecordLoad::Read(..))
}

// A directory with only a zip and an old-format provenance is made whole from the zip.
#[test]
fn test_pull_repairs_a_directory_from_the_zip_it_holds() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    let trud = recorder(1);
    let release = &trud.releases[0];
    let zip = hold_zip(&ws, release, &trud.zips[&release.download_url]);
    let bytes_before = fs::read(&zip).unwrap();
    let mtime_before = mtime(&zip);
    // What an older pull left: TRUD's listing
    let listing = trud_dir(&ws, &release.release_date).join(format!("trud-releases{}{}.json", "-", release.release_date));
    fs::write(&listing, "{}").unwrap();
    assert!(!readable_provenance(&ws, &release.release_date), "the old provenance must be unreadable to start");

    let first = pull(&ws, &trud, Some(&release.release_date), false, false);

    assert!(first.result.is_ok(), "{:?}\n{}", first.result.err(), first.output);
    assert_eq!(fs::read(&zip).unwrap(), bytes_before, "the zip's bytes changed");
    assert_eq!(mtime(&zip), mtime_before, "the zip was rewritten");
    assert_eq!(trud.archives(), 0, "the zip was downloaded");
    assert_eq!(trud.files(), 3, "NHS's checksum, signature and key are fetched");
    assert!(readable_provenance(&ws, &release.release_date), "the pull record is written");
    assert!(
        !trud_dir(&ws, &release.release_date).join(LEGACY_PROVENANCE_FILENAME).exists(),
        "the older _provenance.json it replaces is removed"
    );
    assert!(!listing.exists(), "the leftover TRUD listing is removed");
    assert!(
        first.output.contains("repaired: checksum, signature, key, provenance"),
        "the block names what it did:\n{}",
        first.output
    );

    let second = pull(&ws, &trud, Some(&release.release_date), false, false);
    assert!(second.result.is_ok());
    assert_eq!(trud.files(), 3, "a second pull fetches nothing");
    assert_eq!(trud.archives(), 0);
    assert!(second.output.contains("cached"), "{}", second.output);
    assert!(!second.output.contains("repaired"), "{}", second.output);
}

// `--force` fetches NHS's files again, and leaves the zip alone.
#[test]
fn test_force_refreshes_nhs_files_and_not_the_zip() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    let trud = recorder(1);
    let release = &trud.releases[0];
    let zip = hold_zip(&ws, release, &trud.zips[&release.download_url]);
    assert!(pull(&ws, &trud, Some(&release.release_date), false, false).result.is_ok());
    assert_eq!(trud.files(), 3);
    let mtime_before = mtime(&zip);

    let forced = pull(&ws, &trud, Some(&release.release_date), false, true);

    assert!(forced.result.is_ok(), "{:?}\n{}", forced.result.err(), forced.output);
    assert_eq!(trud.files(), 6, "all three of NHS's files are fetched again");
    assert_eq!(trud.archives(), 0, "--force must not download a zip that matches TRUD's hash");
    assert_eq!(mtime(&zip), mtime_before);
    assert!(forced.output.contains("refreshed: checksum, signature, key, provenance"), "{}", forced.output);
}

// A zip that isn't the one TRUD lists is refused without `--force`, and downloaded again with it.
#[test]
fn test_a_zip_that_is_not_trud_s_is_refused_then_downloaded_with_force() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    let trud = recorder(1);
    let release = &trud.releases[0];
    let other = zip_bytes(&release.release_date, "not trud's");
    let zip = hold_zip(&ws, release, &other);
    let held_hash = compute_file_sha256(&zip).unwrap();

    let refused = pull(&ws, &trud, Some(&release.release_date), false, false);

    assert!(refused.result.is_err(), "a mismatched zip must be refused");
    assert!(refused.output.contains(&held_hash), "names the hash held:\n{}", refused.output);
    assert!(refused.output.contains(&release.archive_file_sha256), "names TRUD's hash:\n{}", refused.output);
    assert!(refused.output.contains("--force"), "suggests --force:\n{}", refused.output);
    assert_eq!(fs::read(&zip).unwrap(), other, "the zip must not be touched");
    assert_eq!(trud.archives(), 0);
    assert_eq!(trud.files(), 0, "nothing else is fetched for a release that is refused");

    let forced = pull(&ws, &trud, Some(&release.release_date), false, true);

    assert!(forced.result.is_ok(), "{:?}\n{}", forced.result.err(), forced.output);
    assert_eq!(trud.archives(), 1, "the zip is downloaded again");
    assert_eq!(compute_file_sha256(&zip).unwrap(), release.archive_file_sha256);
    assert!(readable_provenance(&ws, &release.release_date));
    assert_eq!(trud.files(), 3);
}

// `--all` heals every release whose zip it holds, names a zip that isn't TRUD's and carries on.
#[test]
fn test_pull_all_repairs_every_held_release_and_names_a_bad_zip() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    let trud = recorder(3);
    let (a, b, c) = (&trud.releases[0], &trud.releases[1], &trud.releases[2]);
    let zip_a = hold_zip(&ws, a, &trud.zips[&a.download_url]);
    let zip_b = hold_zip(&ws, b, &trud.zips[&b.download_url]);
    let bad = zip_bytes(&c.release_date, "not trud's");
    let zip_c = hold_zip(&ws, c, &bad);
    let (mtime_a, mtime_b) = (mtime(&zip_a), mtime(&zip_b));

    let first = pull(&ws, &trud, None, true, false);

    assert!(first.result.is_err(), "a batch with a refused zip exits 1");
    assert_eq!(trud.archives(), 0, "no zip is downloaded");
    assert_eq!(trud.files(), 6, "the two good releases are repaired, three files each");
    assert!(readable_provenance(&ws, &a.release_date) && readable_provenance(&ws, &b.release_date));
    assert_eq!((mtime(&zip_a), mtime(&zip_b)), (mtime_a, mtime_b));
    assert_eq!(fs::read(&zip_c).unwrap(), bad, "the refused zip is untouched");
    assert!(
        first.output.contains(&format!("ods trud pull {} --force", c.release_date)),
        "the refusal names its retry:\n{}",
        first.output
    );

    let second = pull(&ws, &trud, None, true, false);
    assert!(second.result.is_err(), "the refused zip is still refused");
    assert_eq!(trud.files(), 6, "a second batch fetches nothing new");

    let forced = pull(&ws, &trud, None, true, true);
    assert!(forced.result.is_ok(), "{:?}\n{}", forced.result.err(), forced.output);
    assert_eq!(trud.archives(), 1, "only the mismatched zip is downloaded");
    assert_eq!(trud.files(), 6 + 9, "NHS's files are fetched again for all three");
    assert_eq!(compute_file_sha256(&zip_c).unwrap(), c.archive_file_sha256);
}

/// TRUD's name for the zip it lists under `date`.
fn served_zip_name(date: &str) -> String {
    format!("hscorgrefdataxml_data_7.0.0_{}000001.zip", date.replace('-', ""))
}

/// A stand-in TRUD on a local port, for `ODS_TRUD_API_URL`: a listing of `dates` (newest
/// first), each release's zip, and NHS's three files. Returns the base URL and each zip by date.
fn serve_trud(key: &str, dates: &[&str]) -> (String, HashMap<String, Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let content = format!("/keys/{key}/content/items/341");
    let url = |name: &str| format!("{base}{content}/{name}");
    let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
    let mut zips = HashMap::new();
    let mut releases = Vec::new();
    for date in dates {
        let zip = zip_bytes(date, "served");
        let zip_name = served_zip_name(date);
        releases.push(serde_json::json!({
            "id": zip_name, "name": "Release 7.0.0", "releaseDate": date,
            "archiveFileUrl": url(&zip_name), "archiveFileName": zip_name,
            "archiveFileSizeBytes": zip.len(), "archiveFileSha256": sha256_of(&zip),
            "checksumFileUrl": url("trud_x.xml"), "checksumFileName": "trud_x.xml",
            "signatureFileUrl": url("trud_x.xml.asc"), "signatureFileName": "trud_x.xml.sig",
            "publicKeyFileUrl": url("trud-public-key.pgp"), "publicKeyFileName": "trud-public-key.pgp"
        }));
        routes.insert(format!("{content}/{zip_name}"), zip.clone());
        zips.insert(date.to_string(), zip);
    }
    let listing = serde_json::json!({ "apiVersion": "1", "releases": releases });
    routes.insert(format!("/keys/{key}/items/341/releases"), listing.to_string().into_bytes());
    for name in ["trud_x.xml", "trud_x.xml.asc", "trud-public-key.pgp"] {
        routes.insert(format!("{content}/{name}"), b"NHS's bytes".to_vec());
    }
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            let path = request.split_whitespace().nth(1).unwrap_or("").to_string();
            match routes.get(&path) {
                Some(body) => {
                    let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                    let _ = stream.write_all(head.as_bytes());
                    let _ = stream.write_all(body);
                }
                None => {
                    let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                }
            }
        }
    });
    (base, zips)
}

// Nothing `ods` writes or prints holds a credential, even under `--verbose`.
#[test]
fn test_a_pull_with_a_key_leaves_it_out_of_every_file_and_line() {
    const KEY: &str = "TESTKEY123";
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    fs::create_dir_all(&ws).unwrap();

    let date = "2026-07-31";
    let (base, _) = serve_trud(KEY, &[date]);

    let output = common::ods_binary()
        .args(["trud", "pull", date, "--verbose", "--no-progress", "-w"])
        .arg(&ws)
        .env("TRUD_API_KEY", KEY)
        .env("ODS_TRUD_API_URL", &base)
        .output()
        .expect("run ods trud pull");
    let all_output = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "the pull must succeed:\n{all_output}");
    assert!(all_output.contains("[VERBOSE]"), "the run must be verbose to prove anything:\n{all_output}");

    assert!(!all_output.contains(KEY), "a line of output holds the key:\n{all_output}");
    let mut files = 0;
    for entry in walkdir::WalkDir::new(&ws).into_iter().flatten().filter(|e| e.file_type().is_file()) {
        files += 1;
        let bytes = fs::read(entry.path()).unwrap();
        assert!(
            !String::from_utf8_lossy(&bytes).contains(KEY),
            "{} holds the key",
            entry.path().display()
        );
    }
    assert!(files >= 5, "the pull must have written its files, found {files}");
}

// `--format ndjson` says what a pull did to each release whose zip it already held: `repaired`
// when it fetched what was missing, `refreshed` under `--force`, `cached` when the directory was
// already whole. Each is checked for one release and for `--all`.
const KEY: &str = "NDJSONKEY";

/// Puts a release's zip, and only the zip, where a pull would.
fn hold_served_zip(ws: &Path, date: &str, zips: &HashMap<String, Vec<u8>>) {
    let dir = trud_dir(ws, date);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(served_zip_name(date)), &zips[date]).unwrap();
}

/// Runs `ods trud pull <args> --format ndjson`, prints its stdout, and parses each line.
fn pull_ndjson(ws: &Path, base: &str, args: &[&str]) -> Vec<serde_json::Value> {
    let (status, lines, _) = pull_ndjson_exit(ws, base, args);
    assert!(status.success(), "the pull must succeed");
    lines
}

/// As `pull_ndjson`, and returns the exit status and stderr instead of requiring success.
fn pull_ndjson_exit(ws: &Path, base: &str, args: &[&str]) -> (std::process::ExitStatus, Vec<serde_json::Value>, String) {
    let output = common::ods_binary()
        .args(["trud", "pull"])
        .args(args)
        .args(["--format", "ndjson", "--no-progress", "-w"])
        .arg(ws)
        .env("TRUD_API_KEY", KEY)
        .env("ODS_TRUD_API_URL", base)
        .output()
        .expect("run ods trud pull");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    println!("$ ods trud pull {} --format ndjson  (exit {:?})\n{stdout}{stderr}", args.join(" "), output.status.code());
    let lines = stdout.lines().map(|l| serde_json::from_str(l).expect("each line is JSON")).collect();
    (output.status, lines, stderr)
}

fn status_of<'a>(lines: &'a [serde_json::Value], date: &str) -> &'a str {
    let line = lines
        .iter()
        .find(|l| l["trud_release_date"] == date)
        .unwrap_or_else(|| panic!("no ndjson line for {date}: {lines:?}"));
    line["status"].as_str().unwrap()
}

/// A held zip pulled by date, then again by `--all`: `run` does the pulls and returns the last
/// one's lines, whose status must be `expected` both times.
fn for_each_way_to_pull(expected: &str, run: impl Fn(&Path, &str, &[&str]) -> Vec<serde_json::Value>) {
    for args in [vec!["2026-07-31"], vec!["--all"]] {
        let tmp = TempDir::new().unwrap();
        let ws = tmp.path().join("ods_data");
        let (base, zips) = serve_trud(KEY, &["2026-07-31"]);
        hold_served_zip(&ws, "2026-07-31", &zips);
        let lines = run(&ws, &base, &args);
        assert_eq!(status_of(&lines, "2026-07-31"), expected, "pulling {args:?}");
    }
}

#[test]
fn test_ndjson_says_repaired_for_a_release_whose_missing_files_were_fetched() {
    for_each_way_to_pull("repaired", pull_ndjson);
}

#[test]
fn test_ndjson_says_refreshed_for_a_release_pulled_with_force() {
    for_each_way_to_pull("refreshed", |ws, base, args| {
        pull_ndjson(ws, base, args); // makes the directory whole
        let mut forced = args.to_vec();
        forced.push("--force");
        pull_ndjson(ws, base, &forced)
    });
}

#[test]
fn test_ndjson_says_cached_for_a_release_whose_directory_was_whole() {
    for_each_way_to_pull("cached", |ws, base, args| {
        pull_ndjson(ws, base, args); // makes the directory whole
        pull_ndjson(ws, base, args)
    });
}

// `--all` with one release to download and one to repair reports each by its own status.
#[test]
fn test_ndjson_of_a_batch_with_a_download_names_the_repair_and_the_download() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    let (base, zips) = serve_trud(KEY, &["2026-07-31", "2026-06-26"]);
    hold_served_zip(&ws, "2026-06-26", &zips);

    let lines = pull_ndjson(&ws, &base, &["--all"]);

    assert_eq!(status_of(&lines, "2026-07-31"), "downloaded");
    assert_eq!(status_of(&lines, "2026-06-26"), "repaired");
}

// A repair that fails is `failed` with its error, as a failed download is; the pull carries on with the rest, then
// exits 1.

/// Makes a release's repair fail: `trud/datapackage.json` is a directory, so it can't be
/// rewritten.
fn break_repair(ws: &Path, date: &str) {
    let path = ods::provenance::pull_record_path(&ws.join("releases").join(date));
    let _ = fs::remove_file(&path);
    fs::create_dir_all(&path).unwrap();
}

fn line_for<'a>(lines: &'a [serde_json::Value], date: &str) -> &'a serde_json::Value {
    lines.iter().find(|l| l["trud_release_date"] == date).unwrap_or_else(|| panic!("no line for {date}: {lines:?}"))
}

#[test]
fn test_ndjson_says_failed_when_a_repair_fails_and_the_pull_exits_1() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    let (base, zips) = serve_trud(KEY, &["2026-07-31"]);
    hold_served_zip(&ws, "2026-07-31", &zips);
    break_repair(&ws, "2026-07-31");

    let (status, lines, _) = pull_ndjson_exit(&ws, &base, &["2026-07-31"]);

    assert_eq!(status.code(), Some(1));
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0]["status"], "failed");
    assert!(lines[0]["error"].as_str().is_some_and(|e| !e.is_empty()), "names the error: {lines:?}");
}

// Every release is held: the ones that repair are reported, the one that can't is `failed`.
#[test]
fn test_a_batch_carries_on_past_a_failed_repair_and_exits_1() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    let dates = ["2026-07-31", "2026-06-26", "2026-05-29"];
    let (base, zips) = serve_trud(KEY, &dates);
    for date in dates {
        hold_served_zip(&ws, date, &zips);
    }
    break_repair(&ws, "2026-06-26");

    let (status, lines, stderr) = pull_ndjson_exit(&ws, &base, &["--all"]);

    assert_eq!(status.code(), Some(1), "a failed repair fails the batch");
    assert!(stderr.contains("2 cached,"), "the failed release isn't counted as cached:\n{stderr}");
    assert_eq!(status_of(&lines, "2026-07-31"), "repaired");
    assert_eq!(status_of(&lines, "2026-05-29"), "repaired", "the release after the failure is still repaired");
    assert_eq!(status_of(&lines, "2026-06-26"), "failed");
    assert!(line_for(&lines, "2026-06-26")["error"].as_str().is_some_and(|e| !e.is_empty()), "{lines:?}");
    assert!(line_for(&lines, "2026-07-31").get("error").is_none());
}

// The same with a release to download in the batch.
#[test]
fn test_a_batch_with_a_download_reports_a_failed_repair_and_exits_1() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ods_data");
    let (base, zips) = serve_trud(KEY, &["2026-07-31", "2026-06-26", "2026-05-29"]);
    hold_served_zip(&ws, "2026-06-26", &zips);
    hold_served_zip(&ws, "2026-05-29", &zips);
    break_repair(&ws, "2026-06-26");

    let (status, lines, stderr) = pull_ndjson_exit(&ws, &base, &["--all"]);

    assert_eq!(status.code(), Some(1));
    assert!(stderr.contains("1 downloaded · 1 cached · 1 failed"), "{stderr}");
    assert_eq!(status_of(&lines, "2026-07-31"), "downloaded");
    assert_eq!(status_of(&lines, "2026-06-26"), "failed");
    assert_eq!(status_of(&lines, "2026-05-29"), "repaired");
}

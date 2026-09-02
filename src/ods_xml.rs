use anyhow::{Context, Result};
use quick_xml::events::{Event, BytesStart};
use quick_xml::reader::Reader;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read, Seek};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct OdsDate {
    #[serde(rename = "type")]
    pub date_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Location {
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub address_lines: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub town: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub county: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub postcode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uprn: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct OdsRole {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub unique_role_id: String,
    pub primary_role: bool,
    pub status: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub dates: Vec<OdsDate>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct OdsRelationshipTarget {
    pub ods_code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assigning_authority_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_role_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_role_display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_role_unique_role_id: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct OdsRelationship {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub unique_rel_id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub dates: Vec<OdsDate>,
    pub target: OdsRelationshipTarget,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct OdsSuccessor {
    pub unique_succ_id: String,
    #[serde(rename = "type")]
    pub succ_type: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub dates: Vec<OdsDate>,
    pub target: OdsRelationshipTarget,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct OdsContact {
    #[serde(rename = "type")]
    pub contact_type: String,
    pub value: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ParentOrganisation {
    pub ods_code: String,
    pub name: String,
}

/// Raw organisation data as parsed directly from the TRUD XML.
#[derive(Debug, Clone)]
pub struct ParsedOrg {
    pub ods_code: String,
    pub name: String,
    pub status: String,
    pub role: String,
    pub parent_organisation: Option<ParentOrganisation>,
    pub region_code: Option<String>,
    pub root: Option<String>,
    pub assigning_authority_name: Option<String>,
    pub org_record_class: Option<String>,
    pub last_change_date: Option<String>,
    pub dates: Vec<OdsDate>,
    pub geo_loc: Option<Location>,
    pub contacts: Vec<OdsContact>,
    pub roles: Vec<OdsRole>,
    pub relationships: Vec<OdsRelationship>,
    pub successors: Vec<OdsSuccessor>,
}

/// Fully-resolved organisation record.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct OdsRecord {
    pub ods_code: String,
    pub name: String,
    pub status: String,
    pub role: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_organisation: Option<ParentOrganisation>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub region_code: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub assigning_authority_name: Option<String>,

    pub record_class: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_change_date: Option<String>,

    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub dates: Vec<OdsDate>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub geo_loc: Option<Location>,

    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub contacts: Vec<OdsContact>,

    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub roles: Vec<OdsRole>,

    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub relationships: Vec<OdsRelationship>,

    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub successors: Vec<OdsSuccessor>,
}

/// Best-effort removal of extraction directories left behind by earlier runs.
///
/// Each extraction unpacks a ~660 MB XML, and the returned path has to outlive
/// this call, so the directory cannot be scoped to a `TempDir` guard here.
/// Sweeping stale ones on entry keeps the leak bounded rather than unbounded —
/// without this, every `ods make` and `ods trud audit` permanently consumed
/// another 660 MB of temp space.
fn sweep_stale_extractions() {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else { return };
    let now = std::time::SystemTime::now();
    let ours = format!("ods_zip_{}_", std::process::id());

    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with("ods_zip_") || name.starts_with(&ours) {
            continue;
        }
        // Leave anything recent enough that a concurrent `ods` run might still
        // be reading from it.
        let stale = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age.as_secs() > 3600);
        if stale {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Opt-in cache location for extracted release XML, or `None` to extract to
/// scratch space that is deleted when the command finishes.
///
/// **Caching is off by default, deliberately.** Unpacking a release is a
/// ~660 MB write, and a normal `ods make` or `ods trud audit` run needs the XML
/// only for the duration of that command. Persisting it would mean a user who
/// ran one command silently acquired 660 MB of cache they never asked for.
///
/// It also matters for correctness in `ods trud audit`, whose job is to verify
/// the published artifacts against ground-truth XML. Re-extracting from the
/// archive on every run means the audit always derives that ground truth from
/// the SHA-256-verified archive, rather than trusting an unpack it performed
/// earlier and never verified.
///
/// Set `ODS_CACHE_DIR` to trade that away for speed when iterating locally
/// against the same release repeatedly — a developer workflow, not a user one.
pub fn xml_cache_root() -> Option<PathBuf> {
    match std::env::var("ODS_CACHE_DIR") {
        Ok(dir) if !dir.trim().is_empty() => Some(PathBuf::from(dir)),
        _ => None,
    }
}

static EXTRACTION_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_suffix() -> String {
    let count = EXTRACTION_COUNTER.fetch_add(1, Ordering::Relaxed);
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}_{}_{}", std::process::id(), time, count)
}

/// Scratch directory for this process, created on first use.
///
/// One per process rather than one per extraction: the returned XML path has to
/// outlive the extraction call, so it cannot be scoped to a `TempDir` guard
/// there. `cleanup_scratch()` removes it when the command exits.
fn process_scratch() -> Result<PathBuf> {
    static SCRATCH: OnceLock<PathBuf> = OnceLock::new();
    let dir = SCRATCH.get_or_init(|| {
        std::env::temp_dir().join(format!("ods_zip_{}_{}", std::process::id(), unique_suffix()))
    });
    std::fs::create_dir_all(dir)
        .with_context(|| format!("creating scratch directory {}", dir.display()))?;
    Ok(dir.clone())
}

/// Removes this process's scratch directory. Call once, as the command exits.
pub fn cleanup_scratch() {
    if let Some(dir) = std::env::temp_dir()
        .join(format!("ods_zip_{}", std::process::id()))
        .parent()
    {
        // Only our own pid-prefixed directories.
        let prefix = format!("ods_zip_{}_", std::process::id());
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with(&prefix) {
                    let _ = std::fs::remove_dir_all(entry.path());
                }
            }
        }
    }
}

/// Cache key for a release archive.
///
/// TRUD filenames already encode version, date and sequence and never change
/// for a given release; length is included so a truncated or replaced download
/// misses the cache rather than silently reusing a stale unpack.
fn cache_key(zip_path: &Path) -> Result<String> {
    let stem = zip_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "release".to_string());
    let len = std::fs::metadata(zip_path)
        .with_context(|| format!("reading metadata for {}", zip_path.display()))?
        .len();
    Ok(format!("{stem}_{len}"))
}

/// Returns a non-empty `.xml` in `dir`, if one is already cached there.
fn cached_xml(dir: &Path) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "xml")
            && entry.metadata().ok().is_some_and(|m| m.len() > 0)
        {
            return Some(path);
        }
    }
    None
}

/// Extracts the release XML, reusing a previous unpack when one exists.
///
/// Unpacking is ~660 MB and previously ran on every invocation, leaving the
/// result behind in the temp directory each time. Now it happens once per
/// release and is shared.
pub fn extract_xml_from_zip(zip_path: &Path) -> Result<PathBuf> {
    sweep_stale_extractions();

    // Default path: extract to process scratch, cleaned up when the command
    // exits. No persistent state, and `audit` re-derives ground truth from the
    // verified archive every time.
    let Some(root) = xml_cache_root() else {
        // A unique subdirectory per extraction, not the bare process scratch.
        // Release archives all carry the same inner filename, so two concurrent
        // extractions sharing one directory overwrite each other's XML — which
        // showed up as an intermittent failure when several tests extracted at
        // once. The parent is still removed by `cleanup_scratch()`, so this
        // costs nothing in leaked space.
        let staging = process_scratch()?.join(format!("x_{}", unique_suffix()));
        std::fs::create_dir_all(&staging)
            .with_context(|| format!("creating extraction directory {}", staging.display()))?;
        return extract_into(zip_path, &staging);
    };

    let key = cache_key(zip_path)?;
    let cache_dir = root.join(&key);

    if let Some(xml) = cached_xml(&cache_dir) {
        return Ok(xml);
    }

    std::fs::create_dir_all(&root)
        .with_context(|| format!("creating XML cache directory {}", root.display()))?;

    // Stage inside the cache root so the publish step is a same-filesystem
    // rename, and a crashed run leaves a `.staging-` directory the sweep
    // collects rather than a half-written cache entry that looks valid.
    let staging = root.join(format!(
        ".staging-{}",
        unique_suffix()
    ));
    std::fs::create_dir_all(&staging)?;

    let extracted = match extract_into(zip_path, &staging) {
        Ok(p) => p,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e);
        }
    };

    // Keep only the XML; the intermediate inner zip is another ~30 MB.
    if let Ok(entries) = std::fs::read_dir(&staging) {
        for entry in entries.flatten() {
            if entry.path() != extracted {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    match std::fs::rename(&staging, &cache_dir) {
        Ok(()) => Ok(cache_dir.join(extracted.file_name().unwrap())),
        Err(_) => {
            // Another process published this release first: prefer theirs and
            // discard our copy.
            if let Some(xml) = cached_xml(&cache_dir) {
                let _ = std::fs::remove_dir_all(&staging);
                return Ok(xml);
            }
            Ok(extracted)
        }
    }
}

fn extract_into(zip_path: &Path, dest: &Path) -> Result<PathBuf> {
    let file = File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;

    let mut inner_zip_names = Vec::new();
    let mut direct_xml_names = Vec::new();

    for i in 0..archive.len() {
        let file = archive.by_index(i)?;
        let name = file.name().to_string();
        if name.ends_with(".zip") {
            inner_zip_names.push(name);
        } else if name.ends_with(".xml") {
            direct_xml_names.push(name);
        }
    }

    if !inner_zip_names.is_empty() {
        let selected_inner = inner_zip_names
            .iter()
            .find(|n| n.to_lowercase().contains("full"))
            .or_else(|| inner_zip_names.iter().find(|n| !n.to_lowercase().contains("archive")));

        let selected_inner = match selected_inner {
            Some(name) => name.clone(),
            None => {
                anyhow::bail!(
                    "No full dataset ZIP found inside archive {}. Package contains only historical 'archive.zip'.",
                    zip_path.display()
                );
            }
        };

        let mut inner_file = archive.by_name(&selected_inner)?;
        let inner_zip_path = dest.join(Path::new(&selected_inner).file_name().unwrap());
        let mut out = File::create(&inner_zip_path)?;
        std::io::copy(&mut inner_file, &mut out)?;

        // Recurse without re-entering the cache: the cache is keyed on the
        // outer release archive, not on intermediate inner zips.
        return extract_into(&inner_zip_path, dest);
    }

    if !direct_xml_names.is_empty() {
        let selected_xml = direct_xml_names
            .iter()
            .find(|n| n.to_lowercase().contains("full"))
            .or_else(|| direct_xml_names.iter().find(|n| !n.to_lowercase().contains("archive")));

        let selected_xml = match selected_xml {
            Some(name) => name.clone(),
            None => {
                anyhow::bail!(
                    "No full dataset XML found inside archive {}. Package contains only historical archive XML.",
                    zip_path.display()
                );
            }
        };

        let mut xml_file = archive.by_name(&selected_xml)?;
        let file_name = Path::new(&selected_xml).file_name().unwrap();
        let extracted_xml_path = dest.join(file_name);
        let mut out = File::create(&extracted_xml_path)?;
        std::io::copy(&mut xml_file, &mut out)?;

        return Ok(extracted_xml_path);
    }

    anyhow::bail!("No XML or ZIP files found inside archive {}", zip_path.display())
}

pub fn find_xml_file(input_path: &Path) -> Result<PathBuf> {
    if input_path.is_file() {
        if input_path.extension().is_some_and(|ext| ext == "zip") {
            return extract_xml_from_zip(input_path);
        }
        if input_path.extension().is_some_and(|ext| ext == "xml") {
            return Ok(input_path.to_path_buf());
        }
        if let Some(parent) = input_path.parent() {
            return find_xml_file(parent);
        }
        return Ok(input_path.to_path_buf());
    }

    let mut candidates = Vec::new();
    for entry in walkdir::WalkDir::new(input_path) {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            if path.extension().is_some_and(|ext| ext == "zip") || path.extension().is_some_and(|ext| ext == "xml") {
                candidates.push(path.to_path_buf());
            }
        }
    }

    // Exclude candidates that are strictly archive.zip
    candidates.retain(|p| {
        let name = p.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
        name != "archive.zip" && !name.contains("archive")
    });

    // Prioritize top-level TRUD zip, then fullfile.zip
    candidates.sort_by_key(|p| {
        let name = p.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
        if name.starts_with("hscorgrefdataxml_data") {
            0
        } else if name == "fullfile.zip" || name.contains("full") {
            1
        } else {
            2
        }
    });

    // Extraction failures must not be swallowed. A zip that is present but
    // cannot be unpacked — a full disk, a permissions problem, a truncated
    // download — is a completely different situation from "there is no zip
    // here", and reporting both as "No XML file found" makes it undiagnosable.
    let mut failures: Vec<String> = Vec::new();
    for path in &candidates {
        if path.extension().is_some_and(|ext| ext == "zip") {
            match extract_xml_from_zip(path) {
                Ok(xml) => return Ok(xml),
                Err(e) => failures.push(format!("  {}: {e:#}", path.display())),
            }
        } else if path.extension().is_some_and(|ext| ext == "xml") {
            return Ok(path.clone());
        }
    }

    if failures.is_empty() {
        anyhow::bail!(
            "No XML or ZIP file found in {}",
            input_path.display()
        );
    }

    anyhow::bail!(
        "Found {} archive(s) in {} but none could be extracted:\n{}",
        failures.len(),
        input_path.display(),
        failures.join("\n")
    )
}

pub fn convert_parsed_orgs(parsed: HashMap<String, ParsedOrg>) -> std::collections::BTreeMap<String, OdsRecord> {
    let name_map: HashMap<String, String> =
        parsed.iter().map(|(k, v)| (k.clone(), v.name.clone())).collect();

    let mut records: std::collections::BTreeMap<String, OdsRecord> = std::collections::BTreeMap::new();

    for (code, mut org) in parsed {
        if let Some(ref mut parent) = org.parent_organisation {
            if let Some(name) = name_map.get(&parent.ods_code) {
                parent.name = name.clone();
            }
        }
        for rel in &mut org.relationships {
            rel.target.name = name_map.get(&rel.target.ods_code).cloned();
        }
        for succ in &mut org.successors {
            succ.target.name = name_map.get(&succ.target.ods_code).cloned();
        }

        let record_class = match org.org_record_class.as_deref() {
            Some("RC1") => "org".to_string(),
            Some("RC2") => "site".to_string(),
            _ => "org".to_string(),
        };

        for r in &mut org.roles {
            r.status = r.status.to_lowercase();
            if let Some(ref mut d) = r.display_name {
                *d = d.to_lowercase();
            }
        }
        for rel in &mut org.relationships {
            rel.status = rel.status.to_lowercase();
            if let Some(ref mut prd) = rel.target.primary_role_display_name {
                *prd = prd.to_lowercase();
            }
        }
        for succ in &mut org.successors {
            succ.succ_type = succ.succ_type.to_lowercase();
            if let Some(ref mut prd) = succ.target.primary_role_display_name {
                *prd = prd.to_lowercase();
            }
        }

        records.insert(code, OdsRecord {
            ods_code: org.ods_code,
            name: org.name,
            status: org.status.to_lowercase(),
            role: org.role.to_lowercase(),
            parent_organisation: org.parent_organisation,
            region_code: org.region_code,
            root: org.root,
            assigning_authority_name: org.assigning_authority_name,
            record_class,
            last_change_date: org.last_change_date,
            dates: org.dates,
            geo_loc: org.geo_loc,
            contacts: org.contacts,
            roles: org.roles,
            relationships: org.relationships,
            successors: org.successors,
        });
    }

    records
}

fn parse_concept_attrs<B: std::io::BufRead>(e: &BytesStart, reader: &Reader<B>, concept_map: &mut HashMap<String, String>) -> Result<()> {
    let mut attr_id = None;
    let mut attr_code = None;
    let mut display_name = None;
    for attr in e.attributes() {
        let attr = attr?;
        let key = attr.key.as_ref();
        if key == b"id" {
            attr_id = Some(attr.decode_and_unescape_value(reader)?.into_owned());
        } else if key == b"code" {
            attr_code = Some(attr.decode_and_unescape_value(reader)?.into_owned());
        } else if key == b"displayName" {
            display_name = Some(attr.decode_and_unescape_value(reader)?.into_owned());
        }
    }
    if let Some(v) = display_name {
        if let Some(id) = attr_id {
            concept_map.insert(id, v.clone());
        }
        if let Some(code) = attr_code {
            concept_map.insert(code, v);
        }
    }
    Ok(())
}

fn get_manifest_attr<B: std::io::BufRead>(e: &BytesStart, reader: &Reader<B>) -> Option<String> {
    for attr in e.attributes().flatten() {
        if attr.key.as_ref() == b"value" {
            return attr.decode_and_unescape_value(reader).ok().map(|s| s.into_owned());
        }
    }
    None
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ManifestHeader {
    pub publication_date: Option<String>,
    pub publication_seq_num: Option<String>,
    pub publication_type: Option<String>,
    pub publication_source: Option<String>,
    pub publication_schema_version: Option<String>,
    pub publication_record_count: Option<usize>,
    pub primary_role_scope: Option<Vec<String>>,
}

pub fn parse_manifest_header<R: std::io::BufRead>(mut reader: Reader<R>) -> Result<ManifestHeader> {
    reader.trim_text(true);
    let mut header = ManifestHeader::default();
    let mut buf = Vec::new();
    let mut primary_role_scope = Vec::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let name = e.local_name();
                let name_ref = name.as_ref();
                if name_ref.eq_ignore_ascii_case(b"Organisation") || name_ref.eq_ignore_ascii_case(b"Organisations") {
                    break;
                }
                if name_ref == b"PublicationDate" {
                    header.publication_date = get_manifest_attr(e, &reader);
                } else if name_ref == b"PublicationSeqNum" {
                    header.publication_seq_num = get_manifest_attr(e, &reader);
                } else if name_ref == b"PublicationType" {
                    header.publication_type = get_manifest_attr(e, &reader);
                } else if name_ref == b"PublicationSource" {
                    header.publication_source = get_manifest_attr(e, &reader);
                } else if name_ref == b"Version" {
                    header.publication_schema_version = get_manifest_attr(e, &reader);
                } else if name_ref == b"RecordCount" {
                    header.publication_record_count = get_manifest_attr(e, &reader).and_then(|s| s.parse::<usize>().ok());
                } else if name_ref.eq_ignore_ascii_case(b"PrimaryRole") {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref().eq_ignore_ascii_case(b"id") {
                            if let Ok(val) = attr.decode_and_unescape_value(&reader) {
                                primary_role_scope.push(val.into_owned());
                            }
                        }
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    if !primary_role_scope.is_empty() {
        header.primary_role_scope = Some(primary_role_scope);
    }

    Ok(header)
}

fn extract_manifest_header_from_archive<R: Read + Seek>(archive: &mut zip::ZipArchive<R>) -> Result<ManifestHeader> {
    let mut inner_zip_names = Vec::new();
    let mut direct_xml_names = Vec::new();

    for i in 0..archive.len() {
        if let Ok(file) = archive.by_index(i) {
            let name = file.name().to_string();
            let lower = name.to_lowercase();
            if lower.ends_with(".zip") {
                inner_zip_names.push(name);
            } else if lower.ends_with(".xml") {
                direct_xml_names.push(name);
            }
        }
    }

    if !inner_zip_names.is_empty() {
        let selected_inner = inner_zip_names
            .iter()
            .find(|n| n.to_lowercase().contains("full"))
            .or_else(|| inner_zip_names.iter().find(|n| !n.to_lowercase().contains("archive")));

        let selected_inner = match selected_inner {
            Some(name) => name.clone(),
            None => {
                anyhow::bail!("No full dataset ZIP found inside archive. Package contains only historical 'archive.zip'.");
            }
        };

        let mut inner_file = archive.by_name(&selected_inner)?;
        let mut inner_bytes = Vec::new();
        inner_file.read_to_end(&mut inner_bytes)?;
        let cursor = std::io::Cursor::new(inner_bytes);
        let mut inner_archive = zip::ZipArchive::new(cursor)?;
        return extract_manifest_header_from_archive(&mut inner_archive);
    }

    if !direct_xml_names.is_empty() {
        let selected_xml = direct_xml_names
            .iter()
            .find(|n| n.to_lowercase().contains("full"))
            .or_else(|| direct_xml_names.iter().find(|n| !n.to_lowercase().contains("archive")));

        let selected_xml = match selected_xml {
            Some(name) => name.clone(),
            None => {
                anyhow::bail!("No full dataset XML found inside archive.");
            }
        };

        let xml_file = archive.by_name(&selected_xml)?;
        return parse_manifest_header(Reader::from_reader(BufReader::new(xml_file)));
    }

    anyhow::bail!("No XML or ZIP files found inside archive")
}

pub fn extract_manifest_header(path: &Path) -> Result<ManifestHeader> {
    if path.is_file() && path.extension().is_some_and(|e| e == "xml") {
        let file = File::open(path)?;
        return parse_manifest_header(Reader::from_reader(BufReader::new(file)));
    }

    let mut zip_candidates = Vec::new();
    let mut xml_candidates = Vec::new();

    if path.is_file() && path.extension().is_some_and(|e| e == "zip") {
        zip_candidates.push(path.to_path_buf());
    } else if path.is_dir() {
        for entry in walkdir::WalkDir::new(path).into_iter().flatten() {
            let p = entry.path();
            if p.is_file() {
                if p.extension().is_some_and(|e| e == "zip") {
                    let name = p.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
                    if !name.contains("archive") {
                        zip_candidates.push(p.to_path_buf());
                    }
                } else if p.extension().is_some_and(|e| e == "xml") {
                    xml_candidates.push(p.to_path_buf());
                }
            }
        }
    }

    if let Some(xml_path) = xml_candidates.first() {
        let file = File::open(xml_path)?;
        return parse_manifest_header(Reader::from_reader(BufReader::new(file)));
    }

    if let Some(zip_path) = zip_candidates.first() {
        let file = File::open(zip_path)?;
        let mut archive = zip::ZipArchive::new(file)?;
        return extract_manifest_header_from_archive(&mut archive);
    }

    anyhow::bail!("No XML or ZIP files found in {}", path.display())
}

pub fn parse_single_pass(
    xml_path: &Path,
) -> Result<(crate::provenance::OdsProvenance, HashMap<String, String>, HashMap<String, ParsedOrg>)> {
    let file = File::open(xml_path)?;
    let buf_reader = BufReader::with_capacity(128 * 1024, file);
    let mut reader = Reader::from_reader(buf_reader);
    reader.trim_text(true);
 
    let mut concept_map = HashMap::new();
    let mut parsed: HashMap<String, ParsedOrg> = HashMap::new();
    let mut parser_state = ParserState::new();
    let mut buf = Vec::new();

    let mut pub_date = None;
    let mut pub_seq = None;
    let mut pub_type = None;
    let mut pub_source = None;
    let mut xml_version = None;
    let mut manifest_record_count = None;
 
    let mut primary_role_scope: Vec<String> = Vec::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(ref e) => {
                let name = e.local_name();
                let name_ref = name.as_ref();
                if name_ref == b"concept" || name_ref == b"Concept" {
                    parse_concept_attrs(e, &reader, &mut concept_map)?;
                } else if name_ref == b"PublicationDate" {
                    pub_date = get_manifest_attr(e, &reader);
                } else if name_ref == b"PublicationSeqNum" {
                    pub_seq = get_manifest_attr(e, &reader);
                } else if name_ref == b"PublicationType" {
                    pub_type = get_manifest_attr(e, &reader);
                } else if name_ref == b"PublicationSource" {
                    pub_source = get_manifest_attr(e, &reader);
                } else if name_ref == b"Version" {
                    xml_version = get_manifest_attr(e, &reader);
                } else if name_ref == b"RecordCount" {
                    manifest_record_count = get_manifest_attr(e, &reader).and_then(|s| s.parse::<usize>().ok());
                } else if name_ref.eq_ignore_ascii_case(b"PrimaryRole") && !parser_state.in_organisation {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref().eq_ignore_ascii_case(b"id") {
                            if let Ok(val) = attr.decode_and_unescape_value(&reader) {
                                primary_role_scope.push(val.into_owned());
                            }
                        }
                    }
                } else {
                    parser_state.handle_start_or_empty(name_ref, e, false, &reader, &concept_map)?;
                }
            }
            Event::Empty(ref e) => {
                let name = e.local_name();
                let name_ref = name.as_ref();
                if name_ref == b"concept" || name_ref == b"Concept" {
                    parse_concept_attrs(e, &reader, &mut concept_map)?;
                } else if name_ref == b"PublicationDate" {
                    pub_date = get_manifest_attr(e, &reader);
                } else if name_ref == b"PublicationSeqNum" {
                    pub_seq = get_manifest_attr(e, &reader);
                } else if name_ref == b"PublicationType" {
                    pub_type = get_manifest_attr(e, &reader);
                } else if name_ref == b"PublicationSource" {
                    pub_source = get_manifest_attr(e, &reader);
                } else if name_ref == b"Version" {
                    xml_version = get_manifest_attr(e, &reader);
                } else if name_ref == b"RecordCount" {
                    manifest_record_count = get_manifest_attr(e, &reader).and_then(|s| s.parse::<usize>().ok());
                } else if name_ref.eq_ignore_ascii_case(b"PrimaryRole") && !parser_state.in_organisation {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref().eq_ignore_ascii_case(b"id") {
                            if let Ok(val) = attr.decode_and_unescape_value(&reader) {
                                primary_role_scope.push(val.into_owned());
                            }
                        }
                    }
                } else {
                    parser_state.handle_start_or_empty(name_ref, e, true, &reader, &concept_map)?;
                }
            }
            Event::End(ref e) => {
                let name = e.local_name();
                parser_state.handle_end(name.as_ref(), &mut parsed)?;
            }
            Event::Text(ref e) => {
                if parser_state.current_text_target != TextTarget::None {
                    let text = e.unescape()?.into_owned();
                    parser_state.handle_text(text)?;
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    let mut provenance = crate::provenance::OdsProvenance::new(
        pub_date.clone(),
        Some(xml_path),
    );
    provenance.publication_date = pub_date;
    provenance.publication_seq_num = pub_seq;
    provenance.publication_type = pub_type;
    provenance.publication_source = pub_source;
    provenance.publication_schema_version = xml_version;
    provenance.publication_record_count = manifest_record_count;

    Ok((provenance, concept_map, parsed))
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum TextTarget {
    None,
    OrgName,
    AddrLine,
    Town,
    County,
    PostCode,
    Country,
    Uprn,
    SuccType,
}

#[derive(Default)]
struct OrgState {
    code: Option<String>,
    name: Option<String>,
    status: Option<String>,
    root: Option<String>,
    assigning_authority_name: Option<String>,
    record_class: Option<String>,
    last_change_date: Option<String>,
    dates: Vec<OdsDate>,
    location: Option<Location>,
    contacts: Vec<OdsContact>,
    roles: Vec<OdsRole>,
    relationships: Vec<OdsRelationship>,
    successors: Vec<OdsSuccessor>,
    role: Option<OdsRole>,
    rel: Option<OdsRelationship>,
    succ: Option<OdsSuccessor>,
    date: Option<OdsDate>,
    target: Option<OdsRelationshipTarget>,
}

struct ParserState {
    in_organisation: bool,
    in_role: bool,
    in_rel: bool,
    in_succ: bool,
    in_geoloc: bool,
    in_location: bool,
    current_text_target: TextTarget,
    org: OrgState,
}

impl ParserState {
    fn new() -> Self {
        Self {
            in_organisation: false,
            in_role: false,
            in_rel: false,
            in_succ: false,
            in_geoloc: false,
            in_location: false,
            current_text_target: TextTarget::None,
            org: OrgState::default(),
        }
    }

    fn reset_org(&mut self) {
        self.in_organisation = true;
        self.in_role = false;
        self.in_rel = false;
        self.in_succ = false;
        self.in_geoloc = false;
        self.in_location = false;
        self.current_text_target = TextTarget::None;
        self.org = OrgState::default();
    }

    fn handle_start_or_empty<B: std::io::BufRead>(
        &mut self,
        name_ref: &[u8],
        e: &BytesStart,
        is_empty: bool,
        reader: &Reader<B>,
        concept_map: &HashMap<String, String>,
    ) -> Result<()> {
        match name_ref {
            b"Organisation" => {
                self.reset_org();
                for attr in e.attributes() {
                    let attr = attr?;
                    let key = attr.key.as_ref();
                    if key == b"orgRecordClass" {
                        self.org.record_class = Some(attr.decode_and_unescape_value(reader)?.into_owned());
                    }
                }
            }
            b"Name" if self.in_organisation => {
                self.current_text_target = TextTarget::OrgName;
            }
            // ODS expresses dates as child elements, never attributes:
            //   <Date><Type value="Legal"/><Start value="..."/><End value="..."/></Date>
            // The enclosing <Date> therefore carries no data of its own; it just
            // opens a slot that Type/Start/End fill and </Date> routes to the
            // organisation, role, relationship or successor currently in scope.
            b"Date" if self.in_organisation && !is_empty => {
                self.org.date = Some(OdsDate {
                    date_type: String::new(),
                    start: None,
                    end: None,
                });
            }
            // Must precede the `in_succ` arm below: a <Succ> contains BOTH a
            // <Date><Type value="Legal"/></Date> and a sibling <Type>Predecessor</Type>,
            // which mean different things.
            b"Type" if self.org.date.is_some() => {
                if let Some(ref mut date) = self.org.date {
                    for attr in e.attributes() {
                        let attr = attr?;
                        if attr.key.as_ref() == b"value" {
                            date.date_type = attr.decode_and_unescape_value(reader)?.into_owned();
                        }
                    }
                }
            }
            b"Start" if self.org.date.is_some() => {
                if let Some(ref mut date) = self.org.date {
                    for attr in e.attributes() {
                        let attr = attr?;
                        if attr.key.as_ref() == b"value" {
                            date.start = Some(attr.decode_and_unescape_value(reader)?.into_owned());
                        }
                    }
                }
            }
            b"End" if self.org.date.is_some() => {
                if let Some(ref mut date) = self.org.date {
                    for attr in e.attributes() {
                        let attr = attr?;
                        if attr.key.as_ref() == b"value" {
                            date.end = Some(attr.decode_and_unescape_value(reader)?.into_owned());
                        }
                    }
                }
            }
            b"GeoLoc" if self.in_organisation => {
                self.in_geoloc = true;
            }
            b"Location" if self.in_geoloc => {
                self.in_location = true;
                if self.org.location.is_none() {
                    self.org.location = Some(Location::default());
                }
            }
            b"AddrLn1" | b"AddrLn2" | b"AddrLn3" if self.in_location => {
                self.current_text_target = TextTarget::AddrLine;
            }
            b"Town" if self.in_location => {
                self.current_text_target = TextTarget::Town;
            }
            b"County" if self.in_location => {
                self.current_text_target = TextTarget::County;
            }
            b"PostCode" if self.in_location => {
                self.current_text_target = TextTarget::PostCode;
            }
            b"Country" if self.in_location => {
                self.current_text_target = TextTarget::Country;
            }
            b"UPRN" if self.in_location => {
                self.current_text_target = TextTarget::Uprn;
            }
            b"Contact" if self.in_organisation => {
                let mut c_type = None;
                let mut c_val = None;
                for attr in e.attributes() {
                    let attr = attr?;
                    let key = attr.key.as_ref();
                    if key == b"type" {
                        c_type = Some(attr.decode_and_unescape_value(reader)?.into_owned());
                    } else if key == b"value" {
                        c_val = Some(attr.decode_and_unescape_value(reader)?.into_owned());
                    }
                }
                if let (Some(t), Some(mut v)) = (c_type, c_val) {
                    if t.eq_ignore_ascii_case("http") {
                        v = v.to_lowercase();
                    }
                    self.org.contacts.push(OdsContact {
                        contact_type: t,
                        value: v,
                    });
                }
            }
            b"Role" if self.in_organisation => {
                self.in_role = !is_empty;
                let mut role_id = None;
                let mut unique_role_id = None;
                let mut primary_role = false;
                // Real ODS data always supplies <Status> as a child element,
                // which overwrites this. The default only covers records that
                // omit it entirely.
                let status = "active".to_string();

                for attr in e.attributes() {
                    let attr = attr?;
                    let key = attr.key.as_ref();
                    if key == b"id" {
                        role_id = Some(attr.decode_and_unescape_value(reader)?.into_owned());
                    } else if key.eq_ignore_ascii_case(b"uniqueroleid") {
                        unique_role_id = Some(attr.decode_and_unescape_value(reader)?.into_owned());
                    } else if key == b"primaryRole" {
                        let val = attr.decode_and_unescape_value(reader)?;
                        primary_role = val.eq_ignore_ascii_case("true");
                    }
                }

                if let Some(rid) = role_id {
                    let urid = unique_role_id.unwrap_or_default();
                    let display_name = concept_map.get(&rid).cloned();
                    let role_code = rid.clone();
                    let new_role = OdsRole {
                        id: rid,
                        code: Some(role_code),
                        display_name,
                        unique_role_id: urid,
                        primary_role,
                        status,
                        dates: Vec::new(),
                    };
                    self.org.role = Some(new_role);
                }
            }
            b"Relationship" | b"Rel" if self.in_organisation => {
                self.in_rel = !is_empty;
                let mut rel_id = None;
                let mut unique_rel_id = None;
                // As with <Role>, a <Status> child element overwrites this.
                let status = "active".to_string();

                for attr in e.attributes() {
                    let attr = attr?;
                    let key = attr.key.as_ref();
                    if key == b"id" {
                        rel_id = Some(attr.decode_and_unescape_value(reader)?.into_owned());
                    } else if key.eq_ignore_ascii_case(b"uniquerelid") {
                        unique_rel_id = Some(attr.decode_and_unescape_value(reader)?.into_owned());
                    }
                }

                if let Some(rid) = rel_id {
                    let urid = unique_rel_id.unwrap_or_default();
                    let display_name = concept_map.get(&rid).cloned();
                    self.org.rel = Some(OdsRelationship {
                        id: rid,
                        display_name,
                        unique_rel_id: urid,
                        status,
                        dates: Vec::new(),
                        target: OdsRelationshipTarget::default(),
                    });
                }
            }
            b"Successor" | b"Succ" if self.in_organisation => {
                self.in_succ = true;
                let mut unique_succ_id = None;
                for attr in e.attributes() {
                    let attr = attr?;
                    let key = attr.key.as_ref();
                    if key.eq_ignore_ascii_case(b"uniquesuccid") {
                        unique_succ_id = Some(attr.decode_and_unescape_value(reader)?.into_owned());
                    }
                }
                let usid = unique_succ_id.unwrap_or_default();
                self.org.succ = Some(OdsSuccessor {
                    unique_succ_id: usid,
                    succ_type: String::new(),
                    dates: Vec::new(),
                    target: OdsRelationshipTarget::default(),
                });
            }
            b"Type" if self.in_succ => {
                self.current_text_target = TextTarget::SuccType;
            }
            b"Target" if self.in_rel || self.in_succ => {
                let mut target = OdsRelationshipTarget::default();
                for attr in e.attributes() {
                    let attr = attr?;
                    let key = attr.key.as_ref();
                    if key == b"root" {
                        target.root = Some(attr.decode_and_unescape_value(reader)?.into_owned());
                    } else if key == b"assigningAuthorityName" {
                        target.assigning_authority_name = Some(attr.decode_and_unescape_value(reader)?.into_owned());
                    }
                }
                self.org.target = Some(target);
            }
            b"OrgId" if self.org.target.is_some() => {
                for attr in e.attributes() {
                    let attr = attr?;
                    let key = attr.key.as_ref();
                    if key == b"extension" {
                        if let Some(ref mut t) = self.org.target {
                            t.ods_code = attr.decode_and_unescape_value(reader)?.into_owned();
                        }
                    } else if key == b"root" {
                        if let Some(ref mut t) = self.org.target {
                            t.root = Some(attr.decode_and_unescape_value(reader)?.into_owned());
                        }
                    } else if key == b"assigningAuthorityName" {
                        if let Some(ref mut t) = self.org.target {
                            t.assigning_authority_name = Some(attr.decode_and_unescape_value(reader)?.into_owned());
                        }
                    }
                }
            }
            b"PrimaryRole" if self.org.target.is_some() => {
                for attr in e.attributes() {
                    let attr = attr?;
                    let key = attr.key.as_ref();
                    if key == b"id" {
                        let id = attr.decode_and_unescape_value(reader)?.into_owned();
                        if let Some(ref mut t) = self.org.target {
                            t.primary_role_display_name = concept_map.get(&id).cloned();
                            t.primary_role_id = Some(id);
                        }
                    } else if key == b"uniqueRoleID" {
                        if let Some(ref mut t) = self.org.target {
                            t.primary_role_unique_role_id = Some(attr.decode_and_unescape_value(reader)?.into_owned());
                        }
                    }
                }
            }
            b"OrgId" if self.in_organisation && !self.in_rel && !self.in_succ => {
                for attr in e.attributes() {
                    let attr = attr?;
                    let key = attr.key.as_ref();
                    if key == b"extension" {
                        self.org.code = Some(attr.decode_and_unescape_value(reader)?.into_owned());
                    } else if key == b"root" {
                        self.org.root = Some(attr.decode_and_unescape_value(reader)?.into_owned());
                    } else if key == b"assigningAuthorityName" {
                        self.org.assigning_authority_name = Some(attr.decode_and_unescape_value(reader)?.into_owned());
                    }
                }
            }
            // <Status> appears at organisation, role and relationship level.
            // The narrower scopes must be matched first.
            b"Status" if self.in_role => {
                if let Some(ref mut role) = self.org.role {
                    for attr in e.attributes() {
                        let attr = attr?;
                        if attr.key.as_ref() == b"value" {
                            role.status = attr.decode_and_unescape_value(reader)?.into_owned();
                        }
                    }
                }
            }
            b"Status" if self.in_rel => {
                if let Some(ref mut rel) = self.org.rel {
                    for attr in e.attributes() {
                        let attr = attr?;
                        if attr.key.as_ref() == b"value" {
                            rel.status = attr.decode_and_unescape_value(reader)?.into_owned();
                        }
                    }
                }
            }
            b"Status" if self.in_organisation && !self.in_role && !self.in_rel && !self.in_succ => {
                for attr in e.attributes() {
                    let attr = attr?;
                    if attr.key.as_ref() == b"value" {
                        self.org.status = Some(attr.decode_and_unescape_value(reader)?.into_owned());
                    }
                }
            }
            b"LastChangeDate" if self.in_organisation => {
                for attr in e.attributes() {
                    let attr = attr?;
                    if attr.key.as_ref() == b"value" {
                        self.org.last_change_date = Some(attr.decode_and_unescape_value(reader)?.into_owned());
                    }
                }
            }
            _ => {}
        }

        if is_empty {
            self.handle_end(name_ref, &mut HashMap::new())?;
        }
        Ok(())
    }

    fn handle_text(&mut self, text: String) -> Result<()> {
        match self.current_text_target {
            TextTarget::OrgName => self.org.name = Some(text),
            TextTarget::AddrLine => {
                if let Some(ref mut loc) = self.org.location {
                    loc.address_lines.push(text);
                }
            }
            TextTarget::Town => {
                if let Some(ref mut loc) = self.org.location {
                    loc.town = Some(text);
                }
            }
            TextTarget::County => {
                if let Some(ref mut loc) = self.org.location {
                    loc.county = Some(text);
                }
            }
            TextTarget::PostCode => {
                if let Some(ref mut loc) = self.org.location {
                    loc.postcode = Some(text);
                }
            }
            TextTarget::Country => {
                if let Some(ref mut loc) = self.org.location {
                    loc.country = Some(text);
                }
            }
            TextTarget::Uprn => {
                if let Some(ref mut loc) = self.org.location {
                    loc.uprn = Some(text);
                }
            }
            TextTarget::SuccType => {
                if let Some(ref mut succ) = self.org.succ {
                    succ.succ_type = text;
                }
            }
            TextTarget::None => {}
        }
        self.current_text_target = TextTarget::None;
        Ok(())
    }

    fn handle_end(&mut self, name_ref: &[u8], parsed: &mut HashMap<String, ParsedOrg>) -> Result<()> {
        match name_ref {
            b"Organisation" => {
                if let (Some(code), Some(name), Some(status)) = (
                    self.org.code.take(),
                    self.org.name.take(),
                    self.org.status.take(),
                ) {
                    let primary_role_display = self
                        .org
                        .roles
                        .iter()
                        .find(|r| r.primary_role && r.status.eq_ignore_ascii_case("active"))
                        .and_then(|r| r.display_name.clone())
                        .or_else(|| {
                            self.org
                                .roles
                                .iter()
                                .find(|r| r.primary_role)
                                .and_then(|r| r.display_name.clone())
                        })
                        .unwrap_or_else(|| "unknown".to_string());

                    let parent_org = self.org.relationships.iter().find_map(|rel| {
                        if rel.status.eq_ignore_ascii_case("active") && rel.id == crate::ods_codes::REL_COMMISSIONED_BY {
                            Some(ParentOrganisation {
                                ods_code: rel.target.ods_code.clone(),
                                name: rel.target.name.clone().unwrap_or_default(),
                            })
                        } else {
                            None
                        }
                    });

                    let region_code = self.org.relationships.iter().find_map(|rel| {
                        if rel.status.eq_ignore_ascii_case("active") && rel.id == crate::ods_codes::REL_REGION {
                            Some(rel.target.ods_code.clone())
                        } else {
                            None
                        }
                    });

                    let parsed_org = ParsedOrg {
                        ods_code: code.clone(),
                        name,
                        status,
                        role: primary_role_display,
                        parent_organisation: parent_org,
                        region_code,
                        root: self.org.root.take(),
                        assigning_authority_name: self.org.assigning_authority_name.take(),
                        org_record_class: self.org.record_class.take(),
                        last_change_date: self.org.last_change_date.take(),
                        dates: std::mem::take(&mut self.org.dates),
                        geo_loc: self.org.location.take(),
                        contacts: std::mem::take(&mut self.org.contacts),
                        roles: std::mem::take(&mut self.org.roles),
                        relationships: std::mem::take(&mut self.org.relationships),
                        successors: std::mem::take(&mut self.org.successors),
                    };

                    parsed.insert(code, parsed_org);
                }
                self.in_organisation = false;
            }
            b"GeoLoc" => self.in_geoloc = false,
            b"Location" => self.in_location = false,
            b"Date" => {
                if let Some(date) = self.org.date.take() {
                    if let Some(ref mut role) = self.org.role {
                        role.dates.push(date.clone());
                    } else if let Some(ref mut rel) = self.org.rel {
                        rel.dates.push(date.clone());
                    } else if let Some(ref mut succ) = self.org.succ {
                        succ.dates.push(date.clone());
                    } else {
                        self.org.dates.push(date);
                    }
                }
            }
            b"Role" => {
                if let Some(role) = self.org.role.take() {
                    self.org.roles.push(role);
                }
                self.in_role = false;
            }
            b"Relationship" | b"Rel" => {
                if let Some(mut rel) = self.org.rel.take() {
                    if let Some(target) = self.org.target.take() {
                        rel.target = target;
                    }
                    self.org.relationships.push(rel);
                }
                self.in_rel = false;
            }
            b"Successor" | b"Succ" => {
                if let Some(mut succ) = self.org.succ.take() {
                    if let Some(target) = self.org.target.take() {
                        succ.target = target;
                    }
                    self.org.successors.push(succ);
                }
                self.in_succ = false;
            }
            b"Target" => {
                if let Some(target) = self.org.target.take() {
                    if let Some(ref mut rel) = self.org.rel {
                        rel.target = target;
                    } else if let Some(ref mut succ) = self.org.succ {
                        succ.target = target;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}

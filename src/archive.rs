use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

/// Extracted TRUD release archive metadata from verified filename pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrudArchiveInfo {
    pub archive_path: PathBuf,
    pub filename: String,
    pub version: String,
    pub release_name: String,
    pub release_date: String,
}

/// Parses and validates a TRUD release archive filename.
/// Pattern: `hscorgrefdataxml_data_<version>_<YYYYMMDD><seq>.zip`
/// Returns `(version, release_name, release_date_yyyy_mm_dd)`
pub fn parse_trud_archive_filename(filename: &str) -> Option<(String, String, String)> {
    let stem = filename.strip_suffix(".zip")?;
    let prefix = "hscorgrefdataxml_data_";
    if !stem.to_ascii_lowercase().starts_with(prefix) {
        return None;
    }
    let rest = &stem[prefix.len()..];
    let parts: Vec<&str> = rest.split('_').collect();
    if parts.len() < 2 {
        return None;
    }
    let version = parts[0];
    if version.is_empty() {
        return None;
    }
    let date_seq = parts[1];
    let digits: String = date_seq.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() < 8 {
        return None;
    }
    let yyyy: i32 = digits[0..4].parse().ok()?;
    let mm: u32 = digits[4..6].parse().ok()?;
    let dd: u32 = digits[6..8].parse().ok()?;

    let naive_date = chrono::NaiveDate::from_ymd_opt(yyyy, mm, dd)?;
    let date_str = naive_date.format("%Y-%m-%d").to_string();
    let release_name = format!("Release {}", version);

    Some((version.to_string(), release_name, date_str))
}

/// Formats the official error when input is not a recognized TRUD release archive.
pub fn format_not_a_trud_archive_error(filename: &str) -> String {
    format!(
        "✖ Not a TRUD release archive: {}\n  Expected a file named like hscorgrefdataxml_data_<version>_<YYYYMMDD><seq>.zip\n  The release date is read from the filename, so `ods` can record which release\n  the data came from.",
        filename
    )
}

/// Resolves and validates a TRUD release archive from a file or directory path.
pub fn resolve_trud_archive(input_path: &Path) -> Result<TrudArchiveInfo> {
    if !input_path.exists() {
        bail!("Input path does not exist: {}", input_path.display());
    }

    if input_path.is_file() {
        let filename = input_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        match parse_trud_archive_filename(filename) {
            Some((version, release_name, release_date)) => Ok(TrudArchiveInfo {
                archive_path: input_path.to_path_buf(),
                filename: filename.to_string(),
                version,
                release_name,
                release_date,
            }),
            None => bail!("{}", format_not_a_trud_archive_error(filename)),
        }
    } else if input_path.is_dir() {
        let mut candidates = Vec::new();
        let trud_sub = input_path.join("trud");
        let search_dirs = if trud_sub.is_dir() {
            vec![trud_sub, input_path.to_path_buf()]
        } else {
            vec![input_path.to_path_buf()]
        };

        for dir in search_dirs {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() && path.extension().is_some_and(|ext| ext == "zip") {
                        candidates.push(path);
                    }
                }
            }
        }

        // Check matching archive
        for path in &candidates {
            let filename = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if let Some((version, release_name, release_date)) = parse_trud_archive_filename(filename) {
                return Ok(TrudArchiveInfo {
                    archive_path: path.clone(),
                    filename: filename.to_string(),
                    version,
                    release_name,
                    release_date,
                });
            }
        }

        if let Some(first) = candidates.first() {
            let filename = first.file_name().and_then(|s| s.to_str()).unwrap_or("");
            bail!("{}", format_not_a_trud_archive_error(filename));
        }

        bail!(
            "No TRUD release archive (*.zip) found in directory '{}'",
            input_path.display()
        );
    } else {
        bail!("Invalid input path: {}", input_path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_trud_filenames() {
        let (ver, name, date) = parse_trud_archive_filename("hscorgrefdataxml_data_7.0.0_20260731000001.zip").unwrap();
        assert_eq!(ver, "7.0.0");
        assert_eq!(name, "Release 7.0.0");
        assert_eq!(date, "2026-07-31");

        let (ver, name, date) = parse_trud_archive_filename("hscorgrefdataxml_data_5.0.0_20260529000001.zip").unwrap();
        assert_eq!(ver, "5.0.0");
        assert_eq!(name, "Release 5.0.0");
        assert_eq!(date, "2026-05-29");

        let (ver, name, date) = parse_trud_archive_filename("hscorgrefdataxml_data_6.0.0_20260626000001.zip").unwrap();
        assert_eq!(ver, "6.0.0");
        assert_eq!(name, "Release 6.0.0");
        assert_eq!(date, "2026-06-26");
    }

    #[test]
    fn test_reject_invalid_filenames() {
        assert!(parse_trud_archive_filename("HSCOrgRefData_Full_20260728.xml").is_none());
        assert!(parse_trud_archive_filename("renamed.zip").is_none());
        assert!(parse_trud_archive_filename("mock_trud_release.zip").is_none());
        assert!(parse_trud_archive_filename("hscorgrefdataxml_data_invalid.zip").is_none());
    }
}

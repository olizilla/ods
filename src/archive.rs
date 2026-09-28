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

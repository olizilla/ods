//! The ODS role vocabulary and its curated display names.
//!
//! ODS publishes role display names in upper case, occasionally abbreviated
//! (`INDEPENDENT SECTOR H/C PROVIDER SITE`), truncated (`... PRESCRIBING COST CE`)
//! or misspelled (`MANAGMENT`). We publish a curated name instead.
//!
//! `role_code` is the fact; both the source string and ours are cosmetic
//! renderings of it, so the source string is not carried in the dataset. The
//! curation lives here as data, with a recorded reason for every entry that
//! changes more than case.

use anyhow::{bail, Result};
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
use std::sync::OnceLock;

const ROLE_NAMES_JSON: &str = include_str!("../data/role_names.json");
const ROLE_DISPLAY_JSON: &str = include_str!("../data/role_display.json");

#[derive(Debug, Deserialize)]
pub struct LowSignalRoleEntry {
    pub name: String,
    pub why: String,
}

#[derive(Debug, Deserialize)]
pub struct RoleDisplayConfig {
    pub version: u32,
    pub low_signal: BTreeMap<String, LowSignalRoleEntry>,
}

#[derive(Debug, Deserialize)]
pub struct RoleNames {
    pub version: u32,
    #[serde(default)]
    pub rewrites: BTreeMap<String, String>,
    pub names: BTreeMap<String, String>,
}

impl RoleNames {
    /// The curated display name for a role code, if it is in the vocabulary.
    pub fn name(&self, role_code: &str) -> Option<&str> {
        self.names.get(role_code).map(|s| s.as_str())
    }

    /// Strict lookup for the curated display name of a role code.
    ///
    /// Fails immediately with an error if the role code is not in the vocabulary (no fallbacks).
    pub fn role_name(&self, role_code: &str) -> Result<&str> {
        self.names
            .get(role_code)
            .map(|s| s.as_str())
            .ok_or_else(|| anyhow::anyhow!("Role code '{}' has no curated name in data/role_names.json", role_code))
    }

    /// Role codes present in a release but absent from the curation file.
    ///
    /// A new ODS role code must fail the build rather than ship nameless, so
    /// callers should treat a non-empty result as fatal.
    pub fn missing<'a, I: IntoIterator<Item = &'a str>>(&self, observed: I) -> Vec<String> {
        let mut missing: Vec<String> = observed
            .into_iter()
            .filter(|c| !self.names.contains_key(*c))
            .map(|c| c.to_string())
            .collect();
        missing.sort();
        missing.dedup();
        missing
    }
}

/// Parsed once and reused; the file is compiled into the binary so `ods make`
/// is hermetic and the vocabulary version is tied to the tool version.
pub fn role_names() -> &'static RoleNames {
    static CACHE: OnceLock<RoleNames> = OnceLock::new();
    CACHE.get_or_init(|| {
        serde_json::from_str(ROLE_NAMES_JSON)
            .expect("data/role_names.json is compiled in and must always parse")
    })
}

/// Parsed once and reused; configuration for CLI display sorting and de-emphasis.
pub fn role_display_config() -> &'static RoleDisplayConfig {
    static CACHE: OnceLock<RoleDisplayConfig> = OnceLock::new();
    CACHE.get_or_init(|| {
        serde_json::from_str(ROLE_DISPLAY_JSON)
            .expect("data/role_display.json is compiled in and must always parse")
    })
}

/// Returns true if a role code is defined as a low-signal container or subtype modifier.
pub fn is_low_signal_role(code: &str) -> bool {
    role_display_config().low_signal.contains_key(code)
}

/// Formats a list of role codes and their corresponding role names for CLI display.
///
/// If `verbose` is true, renders all role names joined by `", "` in stored lexical order.
/// If `verbose` is false, prioritises descriptive roles and places low-signal roles
/// behind a `+N` count (e.g. `"GP Practice +1"`, `"NHS Trust, Hospice +1"`).
///
/// In the edge case where every role is low-signal, displays the first low-signal role
/// so the display is never empty.
pub fn format_roles_for_display(role_codes: &[String], role_names: &[String], verbose: bool) -> String {
    if role_codes.is_empty() && role_names.is_empty() {
        return String::new();
    }
    if verbose {
        return role_names.join(", ");
    }

    let mut descriptive: Vec<&str> = Vec::new();
    let mut low_signal_count = 0;

    for (code, name) in role_codes.iter().zip(role_names.iter()) {
        if is_low_signal_role(code) {
            low_signal_count += 1;
        } else {
            descriptive.push(name.as_str());
        }
    }

    if !descriptive.is_empty() {
        let desc_str = descriptive.join(", ");
        if low_signal_count > 0 {
            format!("{} +{}", desc_str, low_signal_count)
        } else {
            desc_str
        }
    } else {
        // Fallback: all roles in the set were low-signal
        let first_name = role_names.first().map(|s| s.as_str()).unwrap_or("");
        if role_names.len() > 1 {
            format!("{} +{}", first_name, role_names.len() - 1)
        } else {
            first_name.to_string()
        }
    }
}

/// Fails with an actionable message listing any uncurated role codes.
pub fn ensure_vocabulary_covers(observed: &HashSet<String>) -> Result<()> {
    let missing = role_names().missing(observed.iter().map(|s| s.as_str()));
    if missing.is_empty() {
        return Ok(());
    }
    bail!(
        "✖ {} role code(s) in this release have no curated name: {}\n  \
         Add them to data/role_names.json. If the change is more than case \
         normalisation, record why under `rewrites`.",
        missing.len(),
        missing.join(", ")
    );
}

/// Finds top role suggestions for a misspelled or approximate role name.
pub fn find_role_suggestions(input: &str) -> Vec<(&'static str, &'static str)> {
    let input_norm = input.to_lowercase();
    let input_words: Vec<&str> = input_norm.split_whitespace().collect();
    let vocab = role_names();

    let mut scored: Vec<(&'static str, &'static str, usize, usize)> = Vec::new();

    for (code, name) in &vocab.names {
        let name_lower = name.to_lowercase();
        let name_words: Vec<&str> = name_lower.split_whitespace().collect();

        let mut score = 0;
        if name_lower.contains(&input_norm) || input_norm.contains(&name_lower) {
            score += 50;
        }
        for iw in &input_words {
            for nw in &name_words {
                if nw == iw {
                    score += 30;
                } else if nw.starts_with(iw) || iw.starts_with(nw) {
                    score += 15;
                }
            }
        }

        if score > 0 {
            // (name, code, score, name_length)
            scored.push((name.as_str(), code.as_str(), score, name.len()));
        }
    }

    // Sort by score descending, then shorter name first, then name alphabetically
    scored.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.3.cmp(&b.3)).then_with(|| a.0.cmp(b.0)));
    scored.into_iter().take(3).map(|(n, c, _, _)| (n, c)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curation_file_parses_and_is_populated() {
        let rn = role_names();
        assert_eq!(rn.version, 1);
        assert!(rn.names.len() >= 200, "expected the full ODS vocabulary");
    }

    #[test]
    fn role_display_config_parses_and_has_reasons() {
        let cfg = role_display_config();
        assert_eq!(cfg.version, 1);
        assert!(!cfg.low_signal.is_empty());
        for (code, entry) in &cfg.low_signal {
            assert!(!entry.name.is_empty(), "role {code} missing name");
            assert!(!entry.why.is_empty(), "role {code} missing why explanation");
        }
    }

    #[test]
    fn test_format_roles_for_display_ordering_and_deemphasis() {
        // 1. Single descriptive role
        let codes = vec!["RO76".to_string()];
        let names = vec!["GP Practice".to_string()];
        assert_eq!(format_roles_for_display(&codes, &names, false), "GP Practice");

        // 2. Descriptive + low-signal (RO177 is low-signal)
        let codes = vec!["RO177".to_string(), "RO76".to_string()];
        let names = vec!["Prescribing Cost Centre".to_string(), "GP Practice".to_string()];
        assert_eq!(format_roles_for_display(&codes, &names, false), "GP Practice +1");

        // 3. RO177 + RO72 (RO72 is kept descriptive, RO177 is low-signal)
        let codes = vec!["RO177".to_string(), "RO72".to_string()];
        let names = vec!["Prescribing Cost Centre".to_string(), "Other Prescribing Cost Centre".to_string()];
        assert_eq!(format_roles_for_display(&codes, &names, false), "Other Prescribing Cost Centre +1");

        // 4. Multiple descriptive + low-signal (RO197 NHS Trust, RO57 Foundation Trust, RO7 Hospice)
        let codes = vec!["RO197".to_string(), "RO57".to_string(), "RO7".to_string()];
        let names = vec!["NHS Trust".to_string(), "Foundation Trust".to_string(), "Hospice".to_string()];
        assert_eq!(format_roles_for_display(&codes, &names, false), "NHS Trust, Hospice +1");

        // 5. Multiple descriptive + no low-signal
        let codes = vec!["RO221".to_string(), "RO287".to_string()];
        let names = vec!["School".to_string(), "Non-Maintained Special School".to_string()];
        assert_eq!(format_roles_for_display(&codes, &names, false), "School, Non-Maintained Special School");

        // 6. Verbose mode shows full set in stored order
        let codes = vec!["RO177".to_string(), "RO76".to_string()];
        let names = vec!["Prescribing Cost Centre".to_string(), "GP Practice".to_string()];
        assert_eq!(format_roles_for_display(&codes, &names, true), "Prescribing Cost Centre, GP Practice");

        // 7. Edge fallback: single low-signal role
        let codes = vec!["RO177".to_string()];
        let names = vec!["Prescribing Cost Centre".to_string()];
        assert_eq!(format_roles_for_display(&codes, &names, false), "Prescribing Cost Centre");

        // 8. Edge fallback: multiple low-signal roles
        let codes = vec!["RO177".to_string(), "RO101".to_string()];
        let names = vec!["Prescribing Cost Centre".to_string(), "Social Care Site".to_string()];
        assert_eq!(format_roles_for_display(&codes, &names, false), "Prescribing Cost Centre +1");
    }

    #[test]
    fn known_codes_resolve_to_curated_names() {
        let rn = role_names();
        assert_eq!(rn.name("RO76"), Some("GP Practice"));
        assert_eq!(rn.name("RO318"), Some("ICB"));
        assert_eq!(rn.name("RO177"), Some("Prescribing Cost Centre"));

        assert_eq!(rn.role_name("RO76").unwrap(), "GP Practice");
        assert_eq!(rn.role_name("RO318").unwrap(), "ICB");
        assert_eq!(rn.role_name("RO177").unwrap(), "Prescribing Cost Centre");
        assert!(rn.role_name("RO99999").is_err(), "unknown code must fail strictly without fallback");
    }

    /// The source misspells MANAGEMENT and truncates RO258 at 50 characters.
    /// Both corrections must be recorded, not silent.
    #[test]
    fn substantive_rewrites_are_justified() {
        let rn = role_names();
        assert_eq!(
            rn.name("RO215"),
            Some("Hosts Data Management Integration Centre (DMIC)")
        );
        assert_eq!(
            rn.name("RO258"),
            Some("Sexual Assault Referral Centre Prescribing Cost Centre")
        );
        for code in ["RO215", "RO258", "RO176", "RO15", "RO31"] {
            assert!(
                rn.rewrites.contains_key(code),
                "{code} changes more than case and must record a reason"
            );
        }
    }

    /// Every rewrite must name a code that actually exists in the vocabulary.
    #[test]
    fn rewrites_reference_real_codes() {
        let rn = role_names();
        for code in rn.rewrites.keys() {
            assert!(
                rn.names.contains_key(code),
                "rewrite documents {code}, which is not in the vocabulary"
            );
        }
    }

    #[test]
    fn missing_codes_are_reported() {
        let rn = role_names();
        assert!(rn.missing(["RO76", "RO318"]).is_empty());
        assert_eq!(rn.missing(["RO76", "RO9999"]), vec!["RO9999".to_string()]);
    }
}

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

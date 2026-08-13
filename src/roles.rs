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

// ==========================================
// category — the one opinionated column.
// ==========================================

pub const CATEGORY_RULES_JSON: &str = include_str!("../data/category_rules.json");

#[derive(Debug, Deserialize)]
pub struct CategoryRule {
    pub when_role: String,
    pub category: String,
    #[allow(dead_code)]
    pub why: String,
}

#[derive(Debug, Deserialize)]
pub struct CategoryRules {
    pub version: u32,
    pub rules: Vec<CategoryRule>,
}

impl CategoryRules {
    /// Resolves an entity's category.
    ///
    /// Array order is precedence and the first matching rule wins. Codes with
    /// no rule can never win, which is why generic codes such as RO72 "Other
    /// Prescribing Cost Centre" are simply absent from the rules file.
    ///
    /// Falls through to the curated name of the primary role, which is where
    /// the overwhelming majority of entities land.
    pub fn categorise(&self, primary_role: &str, roles: &[String]) -> String {
        for rule in &self.rules {
            if roles.iter().any(|r| r == &rule.when_role) {
                return rule.category.clone();
            }
        }
        role_names()
            .name(primary_role)
            .unwrap_or(primary_role)
            .to_string()
    }

    /// Index of the rule that fires for these roles, for hit counting.
    pub fn matched_rule(&self, roles: &[String]) -> Option<usize> {
        self.rules
            .iter()
            .position(|rule| roles.iter().any(|r| r == &rule.when_role))
    }
}

pub fn category_rules() -> &'static CategoryRules {
    static CACHE: OnceLock<CategoryRules> = OnceLock::new();
    CACHE.get_or_init(|| {
        serde_json::from_str(CATEGORY_RULES_JSON)
            .expect("data/category_rules.json is compiled in and must always parse")
    })
}

/// Every `when_role` must name a code in the ODS vocabulary.
///
/// Deliberately checked against the curated vocabulary rather than the role
/// codes present in a given release: a partial dataset (a test fixture, a
/// single-trust extract) legitimately contains only a handful of codes, and
/// failing the build for those would make the guardrail unusable.
///
/// Drift in the other direction — ODS retiring a code so a rule quietly stops
/// firing — is caught by the per-rule hit counts printed on every build.
pub fn ensure_rules_reference_known_roles() -> Result<()> {
    let vocab = role_names();
    let unknown: Vec<&str> = category_rules()
        .rules
        .iter()
        .map(|r| r.when_role.as_str())
        .filter(|c| !vocab.names.contains_key(*c))
        .collect();

    if unknown.is_empty() {
        return Ok(());
    }
    bail!(
        "✖ category rule(s) name role code(s) outside the ODS vocabulary: {}\n  \
         Check data/category_rules.json against data/role_names.json.",
        unknown.join(", ")
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

    /// The flagship case: a GP practice registered as a prescribing cost centre.
    #[test]
    fn gp_practice_beats_its_register_label() {
        let rules = category_rules();
        let roles = vec!["RO177".to_string(), "RO76".to_string()];
        assert_eq!(rules.categorise("RO177", &roles), "GP Practice");
    }

    /// A Scottish practice must give the same answer, so "GP Practice" is one
    /// UK-wide category rather than two.
    #[test]
    fn scottish_gp_practice_categorises_the_same() {
        let rules = category_rules();
        let roles = vec!["RO227".to_string(), "RO76".to_string()];
        assert_eq!(rules.categorise("RO227", &roles), "GP Practice");
    }

    #[test]
    fn icb_beats_strategic_partnership() {
        let rules = category_rules();
        let roles = vec!["RO261".to_string(), "RO262".to_string(), "RO318".to_string()];
        assert_eq!(rules.categorise("RO261", &roles), "ICB");
    }

    /// 307 entities hold both; the physical establishment wins.
    #[test]
    fn care_home_outranks_domiciliary_care() {
        let rules = category_rules();
        let both = vec!["RO101".to_string(), "RO269".to_string(), "RO270".to_string()];
        assert_eq!(rules.categorise("RO101", &both), "Care Home");

        let dom_only = vec!["RO101".to_string(), "RO270".to_string()];
        assert_eq!(rules.categorise("RO101", &dom_only), "Domiciliary Care");
    }

    /// Most entities have no rule and fall through to the curated primary name.
    #[test]
    fn default_is_the_curated_primary_role_name() {
        let rules = category_rules();
        assert_eq!(rules.categorise("RO182", &["RO182".to_string()]), "Pharmacy");
        assert_eq!(rules.categorise("RO197", &["RO197".to_string()]), "NHS Trust");
    }

    /// Generic codes are absent from the rules file, so they can never win.
    #[test]
    fn generic_prescribing_cost_centre_never_wins() {
        let rules = category_rules();
        let roles = vec!["RO177".to_string(), "RO72".to_string()];
        assert_eq!(rules.categorise("RO177", &roles), "Prescribing Cost Centre");
    }

    /// Prescribing cost centre subtypes are taxonomy, not correction: the
    /// entity genuinely IS a prescribing cost centre and the subtype only says
    /// which service holds the budget. Same treatment as school subtypes.
    ///
    /// They must not be renamed to the bare service either — 96 entities hold
    /// RO175 "Prison" and 130 hold RO82 "Prison Prescribing Cost Centre" with
    /// zero overlap, so collapsing them would merge two distinct record types.
    #[test]
    fn prescribing_cost_centre_subtypes_fall_through_to_the_default() {
        let rules = category_rules();
        for subtype in ["RO250", "RO247", "RO321", "RO82", "RO248", "RO249"] {
            let roles = vec!["RO177".to_string(), subtype.to_string()];
            assert_eq!(
                rules.categorise("RO177", &roles),
                "Prescribing Cost Centre",
                "{subtype} should not get a category of its own"
            );
        }
    }

    /// The opinion stays small on purpose: five rules, each one a case where the
    /// ODS primary role is wrong about the kind of thing.
    #[test]
    fn the_rule_set_stays_small() {
        let rules = category_rules();
        assert_eq!(rules.rules.len(), 5, "rules: {:?}",
            rules.rules.iter().map(|r| &r.when_role).collect::<Vec<_>>());
    }

    /// Taxonomic detail is reachable via `roles` and deliberately has no rule.
    #[test]
    fn school_subtypes_do_not_get_their_own_category() {
        let rules = category_rules();
        let roles = vec!["RO221".to_string(), "RO302".to_string()];
        assert_eq!(rules.categorise("RO221", &roles), "School");
    }

    #[test]
    fn every_rule_names_a_real_role_code() {
        let vocab = role_names();
        for rule in &category_rules().rules {
            assert!(
                vocab.names.contains_key(&rule.when_role),
                "rule for {} names a code outside the vocabulary",
                rule.when_role
            );
            assert!(!rule.why.trim().is_empty(), "{} must record why", rule.when_role);
        }
    }

    #[test]
    fn missing_codes_are_reported() {
        let rn = role_names();
        assert!(rn.missing(["RO76", "RO318"]).is_empty());
        assert_eq!(rn.missing(["RO76", "RO9999"]), vec!["RO9999".to_string()]);
    }
}

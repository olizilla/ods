//! NHS England's terms for the ODS data. Each release freezes them, so they are written once,
//! here, and every output that states them reads from this file.

/// The SPDX identifier of the licence the data is published under.
pub const LICENSE: &str = "OGL-UK-3.0";

/// The attribution TRUD specifies for the data.
pub const ATTRIBUTION: &str =
    "Contains information from NHS England, licensed under the current version of the Open Government Licence.";

/// The human-readable title of the licence.
pub const LICENSE_TITLE: &str = "Open Government Licence v3.0";

/// The canonical URL of the licence text.
pub const LICENSE_URL: &str = "https://www.nationalarchives.gov.uk/doc/open-government-licence/version/3/";

/// NHS TRUD's landing page for this data category (item 341, ODS XML): the front door to
/// relocate the source from, not a hash-stable URL. The pull record states it as the TRUD
/// release's `homepage`, and the embedded object's source `path` is derived from that.
pub const LANDING_PAGE: &str =
    "https://isd.digital.nhs.uk/trud/users/guest/filters/0/categories/5/items/341/releases";

/// The licence as the Data Package vocabulary states it: the SPDX id, the licence text's URL,
/// its title, and the attribution NHS England asks for.
pub fn license() -> crate::provenance::License {
    crate::provenance::License {
        name: LICENSE.to_string(),
        path: LICENSE_URL.to_string(),
        title: LICENSE_TITLE.to_string(),
        attribution: ATTRIBUTION.to_string(),
    }
}

/// Who holds the rights and who distributes, as DataCite contributor roles.
pub fn contributors() -> Vec<crate::provenance::Contributor> {
    vec![
        crate::provenance::Contributor { title: "NHS England".to_string(), roles: vec!["rightsHolder".to_string()] },
        crate::provenance::Contributor { title: "NHS TRUD".to_string(), roles: vec!["distributor".to_string()] },
    ]
}

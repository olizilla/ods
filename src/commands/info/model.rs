use crate::commands::find::OrgRow;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InfoRole {
    pub role_code: String,
    pub role_name: String,
    #[serde(default)]
    pub primary: bool,
    pub status: String,
    pub operational_start: Option<String>,
    pub operational_end: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InfoRelationship {
    pub rel_code: String,
    #[serde(default = "default_direction")]
    pub direction: String,
    pub code: String,
    pub name: String,
    pub status: String,
    pub operational_start: Option<String>,
    pub operational_end: Option<String>,
}

fn default_direction() -> String {
    "outbound".to_string()
}

impl From<&RelationshipItemJson> for InfoRelationship {
    fn from(r: &RelationshipItemJson) -> Self {
        Self {
            rel_code: r.rel_code.clone(),
            direction: r.direction.clone(),
            code: r.code.clone(),
            name: r.name.clone(),
            status: r.status.clone(),
            operational_start: r.operational_start.clone(),
            operational_end: r.operational_end.clone(),
        }
    }
}

impl From<RelationshipItemJson> for InfoRelationship {
    fn from(r: RelationshipItemJson) -> Self {
        Self {
            rel_code: r.rel_code,
            direction: r.direction,
            code: r.code,
            name: r.name,
            status: r.status,
            operational_start: r.operational_start,
            operational_end: r.operational_end,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LiveSuccessor {
    pub code: String,
    pub name: String,
}

/// Enriched organisation record for the markdown card view (`ods info <ODS_CODE>`).
///
/// Embeds the canonical `OrgRow` from Parquet and implements `Deref<Target = OrgRow>`
/// so that all schema fields (`ods_code`, `name`, `address`, `status`, etc.) can be
/// accessed directly without copying.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InfoRecord {
    #[serde(flatten)]
    pub org: OrgRow,
    #[serde(default)]
    pub roles: Vec<InfoRole>,
    #[serde(default)]
    pub relationships: Vec<InfoRelationship>,
    #[serde(default)]
    pub successors_live: Vec<LiveSuccessor>,
    #[serde(default)]
    pub successor_hops: usize,
}

impl std::ops::Deref for InfoRecord {
    type Target = OrgRow;

    fn deref(&self) -> &Self::Target {
        &self.org
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SuccessionHopJson {
    pub depth: usize,
    pub date: Option<String>,
    pub code: String,
    pub name: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HierarchyEntityJson {
    pub code: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RelationshipItemJson {
    pub rel_code: String,
    pub rel_name: String,
    pub direction: String,
    pub code: String,
    pub name: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operational_start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operational_end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legal_start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legal_end: Option<String>,
}

/// Detailed JSON view for `ods info <ODS_CODE> --format json`.
///
/// Flattening `org: OrgRow` embeds the 23 canonical Parquet schema columns in exact
/// schema order, followed by the info-specific enriched fields below.
/// The struct field order directly defines the key order in the serialized JSON.
#[derive(Debug, Clone, Serialize)]
pub struct InfoRecordJson {
    #[serde(flatten)]
    pub org: OrgRow,
    pub primary_role_name: String,
    pub other_roles: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub relationships: Vec<RelationshipItemJson>,
    pub succession: Vec<SuccessionHopJson>,
    pub predecessors: Vec<SuccessionHopJson>,
}

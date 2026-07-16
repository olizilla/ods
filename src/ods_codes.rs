//! Centralized ODS Role and Relationship Code Constants.
//!
//! If NHS alters the standard code designations, updating them here
//! will propagate throughout the parsing, resolution, and export modules.

/// Primary Role: Primary Care Network (PCN)
pub const ROLE_PCN: &str = "RO272";

/// Primary Role: NHS Trust (Hospital / Provider Trust)
pub const ROLE_NHS_TRUST: &str = "RO197";

/// Primary Role: Integrated Care Board (ICB)
pub const ROLE_ICB: &str = "RO318";

/// Relationship: "is commissioned by" (used for hierarchical resolution)
pub const REL_COMMISSIONED_BY: &str = "RE4";

/// Relationship: "is managed by" (alternative hierarchy link)
pub const REL_MANAGED_BY: &str = "RE6";

/// Relationship: Regional link (used to identify root-level regions)
pub const REL_REGION: &str = "RE5";

use serde::{Deserialize, Serialize};
use std::fmt;

/// Strongly-typed organisation status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OrgStatus {
    Active,
    Inactive,
}

impl OrgStatus {
    pub fn parse(s: &str) -> Self {
        if s.eq_ignore_ascii_case("active") {
            Self::Active
        } else {
            Self::Inactive
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self, Self::Active)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "Active",
            Self::Inactive => "Inactive",
        }
    }
}

impl fmt::Display for OrgStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl Default for OrgStatus {
    fn default() -> Self {
        Self::Active
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_org_status() {
        assert_eq!(OrgStatus::parse("active"), OrgStatus::Active);
        assert_eq!(OrgStatus::parse("Active"), OrgStatus::Active);
        assert_eq!(OrgStatus::parse("inactive"), OrgStatus::Inactive);
        assert_eq!(OrgStatus::parse("Closed"), OrgStatus::Inactive);

        assert!(OrgStatus::Active.is_active());
        assert!(!OrgStatus::Inactive.is_active());

        assert_eq!(OrgStatus::Active.to_string(), "Active");
        assert_eq!(OrgStatus::Inactive.to_string(), "Inactive");
    }
}

use std::fmt;
use std::net::IpAddr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Lowercased canonical username for case-insensitive lookups.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Username(String);

impl Username {
    pub fn new(raw: &str) -> Self {
        Self(raw.to_lowercase())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Username {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DisplayName(String);

impl DisplayName {
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DisplayName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PasswordHash(String);

impl PasswordHash {
    pub fn new(hash: impl Into<String>) -> Self {
        Self(hash.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PremiumInfo {
    pub mojang_uuid: Uuid,
    pub force_cracked: bool,
    pub first_premium_login: DateTime<Utc>,
    pub last_premium_login: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthAccount {
    pub username: Username,
    pub display_name: DisplayName,
    pub password_hash: Option<PasswordHash>,
    pub registered_at: DateTime<Utc>,
    pub last_login: Option<DateTime<Utc>>,
    pub last_ip: Option<IpAddr>,
    pub login_count: u64,
    #[serde(default)]
    pub premium_info: Option<PremiumInfo>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn username_lowercases_input() {
        let u = Username::new("TestPlayer");
        assert_eq!(u.as_str(), "testplayer");
    }

    #[test]
    fn username_eq_is_case_insensitive() {
        assert_eq!(Username::new("Alice"), Username::new("ALICE"));
    }

    #[test]
    fn display_name_preserves_case() {
        let d = DisplayName::new("TestPlayer");
        assert_eq!(d.as_str(), "TestPlayer");
    }
}

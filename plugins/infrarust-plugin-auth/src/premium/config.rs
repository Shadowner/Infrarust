//! Premium auto-login configuration.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PremiumConfig {
    pub enabled: bool,
    pub cache_ttl_seconds: u64,
    pub rate_limit_per_second: u32,
    pub rate_limit_action: RateLimitAction,
    /// Applied when a Mojang lookup fails for any non-rate-limit reason
    /// (network error, unexpected status, malformed response).
    pub lookup_error_action: RateLimitAction,
    pub premium_name_conflict_action: NameConflictAction,
    pub allow_cracked_command: bool,
    pub failed_auth_remember_seconds: u64,
    pub messages: PremiumMessages,
}

impl Default for PremiumConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cache_ttl_seconds: 600,
            rate_limit_per_second: 1,
            rate_limit_action: RateLimitAction::default(),
            lookup_error_action: RateLimitAction::default(),
            premium_name_conflict_action: NameConflictAction::default(),
            allow_cracked_command: true,
            failed_auth_remember_seconds: 600,
            messages: PremiumMessages::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitAction {
    /// Let the player through in offline mode (fail-open).
    #[default]
    AllowOffline,
    /// Deny the connection entirely.
    Deny,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NameConflictAction {
    #[default]
    Kick,
    AllowCracked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PremiumMessages {
    pub premium_login: String,
    pub premium_name_conflict: String,
    pub cracked_enabled: String,
    pub cracked_disabled: String,
    pub rate_limited: String,
    pub lookup_failed: String,
}

impl Default for PremiumMessages {
    fn default() -> Self {
        Self {
            premium_login: "&aWelcome back, {username}! (Premium auto-login)".to_string(),
            premium_name_conflict:
                "&cThis username belongs to a premium account. Use the official Minecraft launcher."
                    .to_string(),
            cracked_enabled: "&aYou will now login as a cracked player. Reconnect to apply."
                .to_string(),
            cracked_disabled: "&aYou will now login as a premium player. Reconnect to apply."
                .to_string(),
            rate_limited: "&cThe server is busy. Please try again in a moment.".to_string(),
            lookup_failed: "&cCould not verify your account status. Please try again later."
                .to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_error_action_defaults_to_fail_open() {
        assert!(matches!(
            PremiumConfig::default().lookup_error_action,
            RateLimitAction::AllowOffline
        ));

        // Existing configs without the field keep the historical fail-open behavior.
        let parsed: PremiumConfig = toml::from_str("enabled = true").unwrap();
        assert!(matches!(
            parsed.lookup_error_action,
            RateLimitAction::AllowOffline
        ));

        let parsed: PremiumConfig = toml::from_str("lookup_error_action = \"deny\"").unwrap();
        assert!(matches!(parsed.lookup_error_action, RateLimitAction::Deny));
    }
}

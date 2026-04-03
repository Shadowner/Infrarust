use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::AuthError;
use crate::ip_mask::IpMaskingMode;
use crate::premium::config::PremiumConfig;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AuthConfig {
    pub storage: StorageConfig,
    pub hashing: HashingConfig,
    pub password_policy: PasswordPolicyConfig,
    pub security: SecurityConfig,
    pub privacy: PrivacyConfig,
    pub admin: AdminConfig,
    pub messages: AuthMessages,
    pub premium: PremiumConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    pub backend: String,
    pub path: String,
    pub auto_save_interval_seconds: u64,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            backend: "json".to_string(),
            path: "accounts.json".to_string(),
            auto_save_interval_seconds: 300,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HashingConfig {
    pub argon2_memory_cost: u32,
    pub argon2_time_cost: u32,
    pub argon2_parallelism: u32,
    pub migrate_legacy_hashes: bool,
}

impl Default for HashingConfig {
    fn default() -> Self {
        Self {
            argon2_memory_cost: 19456,
            argon2_time_cost: 2,
            argon2_parallelism: 1,
            migrate_legacy_hashes: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PasswordPolicyConfig {
    pub min_length: usize,
    pub max_length: usize,
    pub blocked_passwords_file: String,
    pub check_username: bool,
}

impl Default for PasswordPolicyConfig {
    fn default() -> Self {
        Self {
            min_length: 8,
            max_length: 128,
            blocked_passwords_file: "blocked_passwords.txt".to_string(),
            check_username: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SecurityConfig {
    pub max_login_attempts: u32,
    pub login_timeout_seconds: u64,
    pub title_reminder_interval_seconds: u64,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            max_login_attempts: 5,
            login_timeout_seconds: 60,
            title_reminder_interval_seconds: 5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PrivacyConfig {
    pub log_ip_masking: IpMaskingMode,
}

impl Default for PrivacyConfig {
    fn default() -> Self {
        Self {
            log_ip_masking: IpMaskingMode::LastTwoOctets,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AdminConfig {
    pub admin_usernames: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AuthMessages {
    pub login_title: String,
    pub login_subtitle: String,
    pub login_success: String,
    pub login_fail: String,
    pub login_max_attempts: String,
    pub login_timeout: String,
    pub login_usage: String,

    pub register_title: String,
    pub register_subtitle: String,
    pub register_success: String,
    pub register_password_mismatch: String,
    pub register_password_too_short: String,
    pub register_password_too_long: String,
    pub register_password_is_username: String,
    pub register_password_blocked: String,
    pub register_account_exists: String,
    pub register_usage: String,

    pub changepassword_success: String,
    pub changepassword_wrong_old: String,
    pub changepassword_usage: String,
    pub unregister_success: String,
    pub unregister_wrong_password: String,
    pub unregister_usage: String,

    pub forcelogin_success: String,
    pub forcelogin_not_in_limbo: String,
    pub forcelogin_not_found: String,
    pub forceunregister_success: String,
    pub forceunregister_not_found: String,
    pub forcechangepassword_success: String,
    pub forcechangepassword_not_found: String,
    pub forcechangepassword_usage: String,
    pub admin_no_permission: String,
    pub authreload_success: String,

    pub reminder_title: String,
    pub reminder_subtitle: String,
    pub unknown_command: String,
}

impl Default for AuthMessages {
    fn default() -> Self {
        Self {
            login_title: "&6Authentication Required".to_string(),
            login_subtitle: "&7/login <password>".to_string(),
            login_success: "&aLogin successful!".to_string(),
            login_fail: "&cWrong password! &7({attempts_left}/{max_attempts} attempts left)"
                .to_string(),
            login_max_attempts: "&cToo many failed login attempts.".to_string(),
            login_timeout: "&cAuthentication timed out.".to_string(),
            login_usage: "&cUsage: /login <password>".to_string(),

            register_title: "&6Welcome, {username}!".to_string(),
            register_subtitle: "&7/register <password> <confirm>".to_string(),
            register_success: "&aAccount created successfully!".to_string(),
            register_password_mismatch: "&cPasswords do not match.".to_string(),
            register_password_too_short: "&cPassword must be at least {min_length} characters."
                .to_string(),
            register_password_too_long: "&cPassword must be at most {max_length} characters."
                .to_string(),
            register_password_is_username: "&cPassword cannot be the same as your username."
                .to_string(),
            register_password_blocked:
                "&cThat password is too common. Please choose a different one.".to_string(),
            register_account_exists: "&cAn account already exists for this username.".to_string(),
            register_usage: "&cUsage: /register <password> <confirm>".to_string(),

            changepassword_success: "&aPassword changed successfully.".to_string(),
            changepassword_wrong_old: "&cCurrent password is incorrect.".to_string(),
            changepassword_usage: "&cUsage: /changepassword <old> <new>".to_string(),
            unregister_success: "&aYour account has been deleted.".to_string(),
            unregister_wrong_password: "&cIncorrect password.".to_string(),
            unregister_usage: "&cUsage: /unregister <password>".to_string(),

            forcelogin_success: "&a{username} has been force-authenticated.".to_string(),
            forcelogin_not_in_limbo: "&c{username} is not in auth limbo.".to_string(),
            forcelogin_not_found: "&cPlayer not found: {username}".to_string(),
            forceunregister_success: "&aAccount deleted for {username}.".to_string(),
            forceunregister_not_found: "&cNo account found for {username}.".to_string(),
            forcechangepassword_success: "&aPassword changed for {username}.".to_string(),
            forcechangepassword_not_found: "&cNo account found for {username}.".to_string(),
            forcechangepassword_usage: "&cUsage: /forcechangepassword <username> <password>"
                .to_string(),
            admin_no_permission: "&cYou do not have permission to use this command.".to_string(),
            authreload_success: "&aAuth configuration reloaded.".to_string(),

            reminder_title: "&6Please authenticate".to_string(),
            reminder_subtitle: "&7Use /login or /register".to_string(),
            unknown_command: "&7Available commands: &f/login&7, &f/register".to_string(),
        }
    }
}

impl AuthConfig {
    pub fn admin_set(&self) -> HashSet<String> {
        self.admin
            .admin_usernames
            .iter()
            .map(|u| u.to_lowercase())
            .collect()
    }
}

impl AuthMessages {
    pub fn format_message(&self, template: &str, replacements: &[(&str, &str)]) -> String {
        let mut result = template.to_string();
        for (key, value) in replacements {
            result = result.replace(key, value);
        }
        result
    }
}

pub async fn load_or_create_config(path: &Path) -> Result<AuthConfig, AuthError> {
    if path.exists() {
        let content = tokio::fs::read_to_string(path)
            .await
            .map_err(AuthError::Io)?;
        toml::from_str(&content).map_err(|e| AuthError::Config(e.to_string()))
    } else {
        let config = AuthConfig::default();
        let content =
            toml::to_string_pretty(&config).map_err(|e| AuthError::Config(e.to_string()))?;

        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(AuthError::Io)?;
        }

        tokio::fs::write(path, &content)
            .await
            .map_err(AuthError::Io)?;

        tracing::info!("Created default auth config at {}", path.display());
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_serializes_to_valid_toml() {
        let config = AuthConfig::default();
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let _: AuthConfig = toml::from_str(&toml_str).unwrap();
    }

    #[test]
    fn format_message_replaces_placeholders() {
        let messages = AuthMessages::default();
        let result = messages.format_message(
            "&cWrong password! ({attempts_left}/{max_attempts})",
            &[("{attempts_left}", "3"), ("{max_attempts}", "5")],
        );
        assert_eq!(result, "&cWrong password! (3/5)");
    }
}

//! Validation helpers for configuration structs.

use std::collections::HashSet;

use crate::error::ConfigError;
use crate::proxy::ProxyConfig;
use crate::server::ServerConfig;

/// Validates a single server configuration.
///
/// Checks:
/// - At least one domain is defined
/// - At least one address is defined
/// - No empty domain strings
/// - `name` (if set) matches `[a-z0-9_-]+`
/// - `network` (if set) matches `[a-z0-9_-]+`
///
/// # Errors
///
/// Returns [`ConfigError::NoDomains`] if no domains are defined,
/// [`ConfigError::NoAddresses`] if no addresses are defined, or
/// [`ConfigError::Validation`] if any domain string is empty or name/network are invalid.
pub fn validate_server_config(config: &ServerConfig) -> Result<(), ConfigError> {
    let id = config.effective_id();

    if config.domains.is_empty() {
        return Err(ConfigError::NoDomains { id });
    }

    if config.addresses.is_empty() {
        return Err(ConfigError::NoAddresses { id });
    }

    for domain in &config.domains {
        if domain.trim().is_empty() {
            return Err(ConfigError::Validation(format!(
                "server config {id} has an empty domain"
            )));
        }
    }

    if let Some(name) = &config.name {
        validate_identifier(name, "name", &id)?;
    }

    if let Some(network) = &config.network {
        validate_identifier(network, "network", &id)?;
    }

    #[cfg(not(target_os = "linux"))]
    if config.proxy_mode == crate::types::ProxyMode::ZeroCopy {
        tracing::warn!(
            server = %id,
            "proxy_mode = zero_copy is only supported on Linux"
        );
    }

    Ok(())
}

fn validate_identifier(value: &str, field: &str, server_id: &str) -> Result<(), ConfigError> {
    if value.is_empty() {
        return Err(ConfigError::Validation(format!(
            "server '{server_id}': {field} must not be empty"
        )));
    }
    if value.len() > 64 {
        return Err(ConfigError::Validation(format!(
            "server '{server_id}': {field} must be at most 64 characters"
        )));
    }
    if !value.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-') {
        return Err(ConfigError::Validation(format!(
            "server '{server_id}': {field} '{value}' contains invalid characters (allowed: a-z, 0-9, _, -)"
        )));
    }
    Ok(())
}

/// Validates a batch of server configurations for duplicate IDs.
///
/// # Errors
///
/// Returns [`ConfigError::DuplicateId`] if two or more configs share the
/// same `effective_id()`.
pub fn validate_server_configs(configs: &[ServerConfig]) -> Result<(), ConfigError> {
    let mut seen = HashSet::with_capacity(configs.len());
    for config in configs {
        let id = config.effective_id();
        if !seen.insert(id.clone()) {
            return Err(ConfigError::DuplicateId(id));
        }
    }
    Ok(())
}

/// Validates the global proxy configuration.
///
/// Checks:
/// - `servers_dir` exists on disk
///
/// # Errors
///
/// Returns [`ConfigError::DirectoryNotFound`] if `servers_dir` does not
/// exist or is not a directory.
pub fn validate_proxy_config(config: &ProxyConfig) -> Result<(), ConfigError> {
    if !config.servers_dir.is_dir() {
        return Err(ConfigError::DirectoryNotFound(config.servers_dir.clone()));
    }

    Ok(())
}

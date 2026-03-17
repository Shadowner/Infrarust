//! Validation helpers for configuration structs.

use crate::error::ConfigError;
use crate::proxy::ProxyConfig;
use crate::server::ServerConfig;

/// Validates a single server configuration.
///
/// Checks:
/// - At least one domain is defined
/// - At least one address is defined
/// - No empty domain strings
///
/// # Errors
///
/// Returns [`ConfigError::NoDomains`] if no domains are defined,
/// [`ConfigError::NoAddresses`] if no addresses are defined, or
/// [`ConfigError::Validation`] if any domain string is empty.
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

    #[cfg(not(target_os = "linux"))]
    if config.proxy_mode == crate::types::ProxyMode::ZeroCopy {
        tracing::warn!(
            server = %id,
            "proxy_mode = zero_copy is only supported on Linux"
        );
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

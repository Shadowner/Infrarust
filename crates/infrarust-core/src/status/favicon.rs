//! Favicon loading and caching.
//!
//! Favicons are loaded at startup (and on hot-reload) from file paths or
//! inline base64. They are stored in memory as data URIs ready for
//! `ServerPingResponse.favicon`.

use std::sync::Arc;

use base64::Engine;
use dashmap::DashMap;
use infrarust_config::{MotdConfig, ServerConfig};

use crate::error::CoreError;

const DATA_URI_PREFIX: &str = "data:image/png;base64,";

/// Loads a favicon value into a data URI string.
///
/// Accepts three formats:
/// - Data URI (`data:image/png;base64,...`) — returned as-is
/// - Raw base64 — prefixed with the data URI header
/// - File path — read, base64-encoded, and prefixed
pub async fn load_favicon(value: &str) -> Result<String, CoreError> {
    if value.starts_with(DATA_URI_PREFIX) {
        return Ok(value.to_string());
    }

    if is_file_path(value) {
        let bytes = tokio::fs::read(value).await.map_err(|e| {
            CoreError::Other(format!("failed to read favicon file '{}': {}", value, e))
        })?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
        return Ok(format!("{DATA_URI_PREFIX}{encoded}"));
    }

    // Treat as raw base64
    Ok(format!("{DATA_URI_PREFIX}{value}"))
}

/// Heuristic: if it contains a path separator or ends with an image extension,
/// treat it as a file path.
fn is_file_path(value: &str) -> bool {
    if value.contains('/') || value.contains('\\') {
        return true;
    }
    let lower = value.to_lowercase();
    lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".gif")
        || lower.ends_with(".ico")
}

/// In-memory cache of pre-loaded favicons.
///
/// Keyed by server id, with a global default fallback.
pub struct FaviconCache {
    by_server: DashMap<String, String>,
    default: Option<String>,
}

impl FaviconCache {
    /// Loads favicons from all server configs and the global default MOTD.
    pub async fn load_from_configs(
        server_configs: &[(String, Arc<ServerConfig>)],
        default_motd: Option<&MotdConfig>,
    ) -> Result<Self, CoreError> {
        let by_server = DashMap::new();

        for (id, cfg) in server_configs {
            if let Some(favicon_value) = cfg.motd.online.as_ref().and_then(|m| m.favicon.as_ref()) {
                match load_favicon(favicon_value).await {
                    Ok(data_uri) => {
                        by_server.insert(id.clone(), data_uri);
                    }
                    Err(e) => {
                        tracing::warn!(server = %id, error = %e, "failed to load favicon");
                    }
                }
            }
        }

        let default = match default_motd
            .and_then(|m| m.online.as_ref())
            .and_then(|e| e.favicon.as_ref())
        {
            Some(val) => match load_favicon(val).await {
                Ok(data_uri) => Some(data_uri),
                Err(e) => {
                    tracing::warn!(error = %e, "failed to load default favicon");
                    None
                }
            },
            None => None,
        };

        Ok(Self { by_server, default })
    }

    /// Reloads all favicons after a config hot-reload.
    pub async fn reload(
        &self,
        server_configs: &[(String, Arc<ServerConfig>)],
        _default_motd: Option<&MotdConfig>,
    ) -> Result<(), CoreError> {
        self.by_server.clear();

        for (id, cfg) in server_configs {
            if let Some(favicon_value) = cfg.motd.online.as_ref().and_then(|m| m.favicon.as_ref()) {
                match load_favicon(favicon_value).await {
                    Ok(data_uri) => {
                        self.by_server.insert(id.clone(), data_uri);
                    }
                    Err(e) => {
                        tracing::warn!(server = %id, error = %e, "failed to load favicon on reload");
                    }
                }
            }
        }

        // Note: default favicon is not reloaded here because ProxyConfig is
        // not hot-reloaded (only server configs are). The default stays
        // from startup.

        Ok(())
    }

    /// Returns the favicon for a server, falling back to the global default.
    pub fn get(&self, server_id: &str) -> Option<String> {
        if let Some(entry) = self.by_server.get(server_id) {
            return Some(entry.value().clone());
        }
        self.default.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_file_path() {
        assert!(is_file_path("icons/survival.png"));
        assert!(is_file_path("/abs/path/icon.png"));
        assert!(is_file_path("C:\\icons\\icon.png"));
        assert!(is_file_path("relative.png"));
        assert!(!is_file_path("iVBORw0KGgoAAAANSUhEUg"));
    }

    #[tokio::test]
    async fn test_load_from_base64_with_prefix() {
        let input = "data:image/png;base64,iVBORw0KGgo";
        let result = load_favicon(input).await.unwrap();
        assert_eq!(result, input);
    }

    #[tokio::test]
    async fn test_load_from_base64_raw() {
        let input = "iVBORw0KGgoAAAANSUhEUg";
        let result = load_favicon(input).await.unwrap();
        assert_eq!(result, format!("data:image/png;base64,{input}"));
    }

    #[tokio::test]
    async fn test_load_from_png_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("icon.png");
        // Write a minimal fake PNG (just bytes, not a real PNG — that's fine for this test)
        tokio::fs::write(&path, b"\x89PNG\r\n\x1a\nfake")
            .await
            .unwrap();

        let result = load_favicon(path.to_str().unwrap()).await.unwrap();
        assert!(result.starts_with("data:image/png;base64,"));
        // Verify it decodes back
        let b64 = result.strip_prefix("data:image/png;base64,").unwrap();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .unwrap();
        assert_eq!(decoded, b"\x89PNG\r\n\x1a\nfake");
    }

    #[tokio::test]
    async fn test_load_file_not_found() {
        let result = load_favicon("/nonexistent/path/icon.png").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_favicon_cache_by_server() {
        let cache = FaviconCache {
            by_server: DashMap::new(),
            default: Some("default_fav".to_string()),
        };
        cache
            .by_server
            .insert("survival".to_string(), "survival_fav".to_string());

        assert_eq!(cache.get("survival").as_deref(), Some("survival_fav"));
    }

    #[tokio::test]
    async fn test_favicon_cache_default_fallback() {
        let cache = FaviconCache {
            by_server: DashMap::new(),
            default: Some("default_fav".to_string()),
        };

        assert_eq!(cache.get("unknown").as_deref(), Some("default_fav"));
    }

    #[tokio::test]
    async fn test_favicon_cache_no_default() {
        let cache = FaviconCache {
            by_server: DashMap::new(),
            default: None,
        };

        assert!(cache.get("unknown").is_none());
    }
}

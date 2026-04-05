use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use infrarust_api::services::config_service::{ProxyMode, ServerConfig};
use infrarust_api::types::{ServerAddress, ServerId};
use serde::{Deserialize, Serialize};

/// Persistable representation of a server config.
#[derive(Serialize, Deserialize, Clone)]
struct StoredServer {
    id: String,
    domains: Vec<String>,
    addresses: Vec<String>,
    proxy_mode: String,
    #[serde(default)]
    limbo_handlers: Vec<String>,
}

impl StoredServer {
    fn from_config(config: &ServerConfig) -> Self {
        Self {
            id: config.id.as_str().to_string(),
            domains: config.domains.clone(),
            addresses: config
                .addresses
                .iter()
                .map(|a| format!("{}:{}", a.host, a.port))
                .collect(),
            proxy_mode: crate::util::proxy_mode_str(config.proxy_mode).to_string(),
            limbo_handlers: config.limbo_handlers.clone(),
        }
    }

    fn to_config(&self) -> ServerConfig {
        ServerConfig::new(
            ServerId::new(&self.id),
            None,
            self.addresses
                .iter()
                .filter_map(|a| {
                    let (host, port_str) = a.rsplit_once(':')?;
                    let port: u16 = port_str.parse().ok()?;
                    Some(ServerAddress {
                        host: host.to_string(),
                        port,
                    })
                })
                .collect(),
            self.domains.clone(),
            match self.proxy_mode.as_str() {
                "zero_copy" | "zerocopy" => ProxyMode::ZeroCopy,
                "client_only" => ProxyMode::ClientOnly,
                "offline" => ProxyMode::Offline,
                "server_only" => ProxyMode::ServerOnly,
                _ => ProxyMode::Passthrough,
            },
            self.limbo_handlers.clone(),
            0,
            None,
            false,
            false,
        )
    }
}

/// In-memory store for servers created via the REST API, backed by a JSON file.
pub struct ApiServerStore {
    servers: Mutex<HashMap<String, ServerConfig>>,
    persist_path: PathBuf,
}

impl ApiServerStore {
    /// Load existing servers from disk, or create an empty store.
    pub fn load(data_dir: &std::path::Path) -> Self {
        let persist_path = data_dir.join("servers.json");
        let mut servers = HashMap::new();

        if persist_path.exists() {
            match std::fs::read_to_string(&persist_path) {
                Ok(content) => match serde_json::from_str::<Vec<StoredServer>>(&content) {
                    Ok(stored) => {
                        for s in stored {
                            let id = s.id.clone();
                            servers.insert(id, s.to_config());
                        }
                        tracing::info!(
                            count = servers.len(),
                            path = %persist_path.display(),
                            "Loaded API-managed servers from disk"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            path = %persist_path.display(),
                            "Failed to parse servers.json, starting with empty store"
                        );
                    }
                },
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        path = %persist_path.display(),
                        "Failed to read servers.json"
                    );
                }
            }
        }

        Self {
            servers: Mutex::new(servers),
            persist_path,
        }
    }

    pub fn insert(&self, config: ServerConfig) {
        let id = config.id.as_str().to_string();
        self.servers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, config);
        self.persist();
    }

    pub fn get(&self, id: &str) -> Option<ServerConfig> {
        self.servers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(id)
            .cloned()
    }

    pub fn remove(&self, id: &str) -> Option<ServerConfig> {
        let removed = self
            .servers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(id);
        if removed.is_some() {
            self.persist();
        }
        removed
    }

    pub fn all(&self) -> Vec<ServerConfig> {
        self.servers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .cloned()
            .collect()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.servers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains_key(id)
    }

    fn persist(&self) {
        let stored: Vec<StoredServer> = self
            .servers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .map(StoredServer::from_config)
            .collect();

        match serde_json::to_string_pretty(&stored) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&self.persist_path, json) {
                    tracing::error!(
                        error = %e,
                        path = %self.persist_path.display(),
                        "Failed to persist servers.json"
                    );
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to serialize servers for persistence");
            }
        }
    }
}

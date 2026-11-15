use std::path::Path;

use serde::{Deserialize, Serialize};

use super::store::{DocumentId, parse_document};

pub(super) const LEGACY_STORE: &str = "servers.json";

/// Pre-2.0 persistence format: a JSON array in `<data_dir>/servers.json`.
#[derive(Deserialize)]
struct StoredServer {
    id: String,
    #[serde(default)]
    domains: Vec<String>,
    #[serde(default)]
    addresses: Vec<String>,
    #[serde(default)]
    proxy_mode: String,
    #[serde(default)]
    limbo_handlers: Vec<String>,
}

#[derive(Serialize)]
struct MigratedServer<'a> {
    id: &'a str,
    domains: &'a [String],
    addresses: &'a [String],
    proxy_mode: &'a str,
    limbo_handlers: &'a [String],
}

pub(super) fn migrate_legacy_store(data_dir: &Path, dir: &Path) {
    let legacy = data_dir.join(LEGACY_STORE);
    if !legacy.exists() {
        return;
    }

    let stored = match std::fs::read_to_string(&legacy)
        .map_err(|e| e.to_string())
        .and_then(|content| {
            serde_json::from_str::<Vec<StoredServer>>(&content).map_err(|e| e.to_string())
        }) {
        Ok(stored) => stored,
        Err(e) => {
            tracing::error!(path = %legacy.display(), error = %e, "Failed to read legacy servers.json");
            return;
        }
    };

    let mut migrated = 0;
    let mut failed = 0;
    for server in &stored {
        let Some(id) = DocumentId::new(&server.id) else {
            tracing::warn!(id = %server.id, "Skipping legacy server with an unusable id");
            failed += 1;
            continue;
        };
        let path = dir.join(format!("{id}.toml"));
        if path.exists() {
            continue;
        }

        let proxy_mode = server
            .proxy_mode
            .parse::<crate::vocab::ProxyModeName>()
            .unwrap_or(crate::vocab::ProxyModeName::Passthrough)
            .as_str();
        let document = match toml::to_string_pretty(&MigratedServer {
            id: &server.id,
            domains: &server.domains,
            addresses: &server.addresses,
            proxy_mode,
            limbo_handlers: &server.limbo_handlers,
        }) {
            Ok(document) => document,
            Err(e) => {
                tracing::warn!(id = %server.id, error = %e, "Skipping unconvertible legacy server");
                failed += 1;
                continue;
            }
        };

        if let Err(e) = parse_document(id.as_str(), &document) {
            tracing::warn!(id = %server.id, error = %e, "Skipping invalid legacy server");
            failed += 1;
            continue;
        }

        if let Err(e) = std::fs::write(&path, document.as_bytes()) {
            tracing::error!(path = %path.display(), error = %e, "Failed to migrate legacy server");
            failed += 1;
            continue;
        }
        migrated += 1;
    }

    if failed > 0 {
        tracing::error!(
            migrated,
            failed,
            path = %legacy.display(),
            "Legacy servers.json was migrated only in part and is kept as is; \
             fix or drop the servers that failed and restart"
        );
        return;
    }

    let done = legacy.with_extension("json.migrated");
    if let Err(e) = std::fs::rename(&legacy, &done) {
        tracing::error!(path = %legacy.display(), error = %e, "Failed to rename legacy servers.json");
        return;
    }

    tracing::info!(count = migrated, path = %done.display(), "Migrated legacy servers.json");
}

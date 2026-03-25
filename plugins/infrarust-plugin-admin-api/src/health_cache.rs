use std::collections::HashMap;
use std::sync::Mutex;

use crate::dto::server::HealthCheckResponse;

/// In-memory cache of the last health check result per server.
///
/// Updated only on demand (when the client requests a health check).
pub struct HealthCache {
    entries: Mutex<HashMap<String, HealthCheckResponse>>,
}

impl Default for HealthCache {
    fn default() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }
}

impl HealthCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, id: &str) -> Option<HealthCheckResponse> {
        self.entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(id)
            .cloned()
    }

    pub fn set(&self, id: &str, result: HealthCheckResponse) {
        self.entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id.to_string(), result);
    }
}

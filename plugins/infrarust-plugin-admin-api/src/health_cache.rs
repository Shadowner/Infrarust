use std::collections::HashMap;
use std::sync::Mutex;

use crate::dto::server::HealthCheckResponse;

#[derive(Default)]
pub struct HealthCache {
    entries: Mutex<HashMap<String, HealthCheckResponse>>,
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

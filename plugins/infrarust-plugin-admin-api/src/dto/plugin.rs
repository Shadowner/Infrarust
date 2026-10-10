use std::time::Duration;

use infrarust_api::plugin::{PluginFault, PluginRuntimeStatus};
use infrarust_api::services::plugin_registry::PluginInfo;
use serde::Serialize;

#[derive(Serialize)]
pub struct PluginResponse {
    pub id: String,
    pub name: String,
    pub version: String,
    pub authors: Vec<String>,
    pub description: Option<String>,
    pub state: String,
    pub dependencies: Vec<PluginDependencyResponse>,
    pub runtime: Option<PluginRuntimeResponse>,
}

impl PluginResponse {
    pub fn from_info(info: PluginInfo) -> Self {
        let state = info.state.as_str().to_string();
        let runtime = info.runtime.map(PluginRuntimeResponse::from_status);
        let meta = info.metadata;
        Self {
            id: meta.id,
            name: meta.name,
            version: meta.version,
            authors: meta.authors,
            description: meta.description,
            state,
            dependencies: meta
                .dependencies
                .into_iter()
                .map(|d| PluginDependencyResponse {
                    id: d.id,
                    optional: d.optional,
                })
                .collect(),
            runtime,
        }
    }
}

#[derive(Serialize)]
pub struct PluginDependencyResponse {
    pub id: String,
    pub optional: bool,
}

#[derive(Serialize)]
pub struct PluginRuntimeResponse {
    pub health: &'static str,
    pub retry_in_ms: Option<u64>,
    pub generation: u64,
    pub restarts_in_window: u32,
    pub max_restarts: u32,
    pub restart_window_secs: u64,
    pub last_fault: Option<PluginFaultResponse>,
    pub queue: PluginQueueResponse,
}

impl PluginRuntimeResponse {
    fn from_status(status: PluginRuntimeStatus) -> Self {
        let queue = status.queue;
        let recent = queue.recent;
        Self {
            health: status.health.as_str(),
            retry_in_ms: status.health.retry_in().map(millis),
            generation: status.generation,
            restarts_in_window: status.restarts.in_window,
            max_restarts: status.restarts.max,
            restart_window_secs: status.restarts.window.as_secs(),
            last_fault: status.last_fault.map(PluginFaultResponse::from_fault),
            queue: PluginQueueResponse {
                depth: queue.depth,
                capacity: queue.capacity,
                window_secs: recent.span.as_secs(),
                taken: recent.taken,
                peak_depth: recent.peak_depth,
                wait_p50_us: micros(recent.wait_p50),
                wait_p99_us: micros(recent.wait_p99),
                wait_max_us: micros(recent.wait_max),
            },
        }
    }
}

#[derive(Serialize)]
pub struct PluginFaultResponse {
    pub cause: String,
    pub secs_ago: u64,
    pub generation: u64,
}

impl PluginFaultResponse {
    fn from_fault(fault: PluginFault) -> Self {
        Self {
            cause: fault.cause,
            secs_ago: fault.ago.as_secs(),
            generation: fault.generation,
        }
    }
}

#[derive(Serialize)]
pub struct PluginQueueResponse {
    pub depth: usize,
    pub capacity: usize,
    pub window_secs: u64,
    pub taken: u64,
    pub peak_depth: usize,
    pub wait_p50_us: u64,
    pub wait_p99_us: u64,
    pub wait_max_us: u64,
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

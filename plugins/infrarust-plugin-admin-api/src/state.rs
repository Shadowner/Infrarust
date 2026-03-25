use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use infrarust_api::services::ban_service::BanService;
use infrarust_api::services::config_service::ConfigService;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::services::plugin_registry::PluginRegistry;
use infrarust_api::services::server_manager::ServerManager;
use serde::Serialize;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use infrarust_api::provider::PluginProviderSender;

use crate::config::ApiConfig;
use crate::health_cache::HealthCache;
use crate::health_checker::HealthChecker;
use crate::log_layer::LogEntry;
use crate::rate_limit::RateLimiter;
use crate::server_store::ApiServerStore;

pub struct ApiState {
    pub player_registry: Arc<dyn PlayerRegistry>,
    pub ban_service: Arc<dyn BanService>,
    pub server_manager: Arc<dyn ServerManager>,
    pub config_service: Arc<dyn ConfigService>,
    pub plugin_registry: Arc<dyn PluginRegistry>,
    pub config: ApiConfig,
    pub start_time: Instant,
    pub proxy_version: String,
    pub rate_limiter: RateLimiter,
    pub event_tx: broadcast::Sender<ApiEvent>,
    pub shutdown: CancellationToken,
    pub proxy_shutdown: CancellationToken,
    /// Log broadcast sender. `None` if `BroadcastLogLayer` is not installed.
    pub log_tx: Option<broadcast::Sender<LogEntry>>,
    /// Ring buffer of recent log entries. `None` if `BroadcastLogLayer` is not installed.
    pub log_history: Option<Arc<Mutex<VecDeque<LogEntry>>>>,
    /// In-memory store for API-created servers.
    pub server_store: Arc<ApiServerStore>,
    /// Sender for emitting config provider events (Added/Updated/Removed).
    pub provider_sender: Arc<tokio::sync::Mutex<Option<Box<dyn PluginProviderSender>>>>,
    /// Cache of last health check result per server.
    pub health_cache: Arc<HealthCache>,
    /// Health checker for pinging Minecraft backends.
    pub health_checker: Arc<HealthChecker>,
    /// Ring buffer of recent activity events (last 100).
    pub recent_events: Arc<Mutex<VecDeque<RecentEvent>>>,
}

/// A summarized event for the activity feed.
#[derive(Debug, Clone, Serialize)]
pub struct RecentEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub summary: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "data")]
#[non_exhaustive]
pub enum ApiEvent {
    PlayerJoin {
        player_id: u64,
        username: String,
        uuid: String,
        server: String,
        timestamp: String,
    },
    PlayerLeave {
        player_id: u64,
        username: String,
        last_server: Option<String>,
        timestamp: String,
    },
    PlayerSwitch {
        player_id: u64,
        username: String,
        from_server: Option<String>,
        to_server: String,
        timestamp: String,
    },
    ServerStateChange {
        server_id: String,
        old_state: String,
        new_state: String,
        timestamp: String,
    },
    ConfigReload {
        timestamp: String,
    },
    BanCreated {
        target_type: String,
        target_value: String,
        reason: Option<String>,
        source: String,
        timestamp: String,
    },
    BanRemoved {
        target_type: String,
        target_value: String,
        timestamp: String,
    },
    StatsTick {
        players_online: usize,
        servers_online: usize,
        bans_active: usize,
        uptime_seconds: u64,
        memory_rss_bytes: Option<u64>,
        timestamp: String,
    },
}

const MAX_RECENT_EVENTS: usize = 100;

impl ApiEvent {
    pub fn event_type(&self) -> &'static str {
        match self {
            ApiEvent::PlayerJoin { .. } => "player.join",
            ApiEvent::PlayerLeave { .. } => "player.leave",
            ApiEvent::PlayerSwitch { .. } => "player.switch",
            ApiEvent::ServerStateChange { .. } => "server.state_change",
            ApiEvent::ConfigReload { .. } => "config.reload",
            ApiEvent::BanCreated { .. } => "ban.created",
            ApiEvent::BanRemoved { .. } => "ban.removed",
            ApiEvent::StatsTick { .. } => "stats.tick",
        }
    }

    /// Convert to a summarized event for the activity feed.
    /// Returns `None` for events that shouldn't appear in the feed (e.g. stats ticks).
    pub fn to_recent(&self) -> Option<RecentEvent> {
        let (event_type, summary, timestamp) = match self {
            ApiEvent::PlayerJoin {
                username,
                server,
                timestamp,
                ..
            } => {
                let srv = if server.is_empty() {
                    String::new()
                } else {
                    format!(" {server}")
                };
                (
                    "player.join",
                    format!("{username} joined{srv}"),
                    timestamp.clone(),
                )
            }
            ApiEvent::PlayerLeave {
                username,
                timestamp,
                ..
            } => (
                "player.leave",
                format!("{username} disconnected"),
                timestamp.clone(),
            ),
            ApiEvent::PlayerSwitch {
                username,
                to_server,
                timestamp,
                ..
            } => (
                "player.switch",
                format!("{username} switched to {to_server}"),
                timestamp.clone(),
            ),
            ApiEvent::ServerStateChange {
                server_id,
                old_state,
                new_state,
                timestamp,
            } => (
                "server.state_change",
                format!("{server_id}: {old_state} → {new_state}"),
                timestamp.clone(),
            ),
            ApiEvent::ConfigReload { timestamp } => (
                "config.reload",
                "Config reloaded".to_string(),
                timestamp.clone(),
            ),
            ApiEvent::BanCreated {
                target_type,
                target_value,
                timestamp,
                ..
            } => (
                "ban.created",
                format!("Banned {target_value} ({target_type})"),
                timestamp.clone(),
            ),
            ApiEvent::BanRemoved {
                target_type: _,
                target_value,
                timestamp,
            } => (
                "ban.removed",
                format!("Unbanned {target_value}"),
                timestamp.clone(),
            ),
            ApiEvent::StatsTick { .. } => return None, // Too noisy for activity feed
        };
        Some(RecentEvent {
            event_type: event_type.to_string(),
            summary,
            timestamp,
        })
    }
}

/// Push an event to the recent events ring buffer.
pub fn push_recent_event(recent_events: &Mutex<VecDeque<RecentEvent>>, event: &ApiEvent) {
    if let Some(recent) = event.to_recent() {
        let mut buffer = recent_events.lock().unwrap_or_else(|p| p.into_inner());
        buffer.push_front(recent);
        buffer.truncate(MAX_RECENT_EVENTS);
    }
}

use infrarust_api::event::EventPriority;
use infrarust_api::event::bus::EventBusExt;
use infrarust_api::events::connection::ServerPostConnectEvent;
use infrarust_api::events::lifecycle::{DisconnectEvent, PostLoginEvent};
use infrarust_api::events::proxy::{BackendHealthEvent, ConfigReloadEvent, ServerStateChangeEvent};
use infrarust_api::plugin::PluginContext;
use infrarust_api::types::ServerId;
use tokio::sync::broadcast;

use crate::state::ApiEvent;
use crate::util::{format_address, now_iso8601};

/// Bridges proxy EventBus events to the API broadcast channel.
///
/// Subscribes to lifecycle and proxy events at `EventPriority::LAST`
/// (observe only, no modifications) and converts them to `ApiEvent`
/// variants that SSE clients consume.
pub struct EventBridge {
    event_tx: broadcast::Sender<ApiEvent>,
}

impl EventBridge {
    pub fn new(event_tx: broadcast::Sender<ApiEvent>) -> Self {
        Self { event_tx }
    }

    /// Registers all event listeners on the EventBus via the plugin context.
    /// Listeners are tracked by the context for automatic cleanup on disable.
    pub fn register_listeners(&self, ctx: &dyn PluginContext) {
        // PostLoginEvent → PlayerJoin
        let tx = self.event_tx.clone();
        ctx.event_bus()
            .subscribe::<PostLoginEvent, _>(EventPriority::LAST, move |event| {
                let _ = tx.send(ApiEvent::PlayerJoin {
                    player_id: event.player_id().as_u64(),
                    username: event.profile.username.clone(),
                    uuid: event.profile.uuid.to_string(),
                    server: String::new(), // Not yet routed at PostLogin
                    timestamp: now_iso8601(),
                });
            });

        // DisconnectEvent → PlayerLeave
        let tx = self.event_tx.clone();
        ctx.event_bus()
            .subscribe::<DisconnectEvent, _>(EventPriority::LAST, move |event| {
                let _ = tx.send(ApiEvent::PlayerLeave {
                    player_id: event.player_id().as_u64(),
                    username: event.username().to_string(),
                    last_server: event.last_server.as_ref().map(|s| s.as_str().to_string()),
                    timestamp: now_iso8601(),
                });
            });

        let tx = self.event_tx.clone();
        ctx.event_bus()
            .subscribe::<ServerPostConnectEvent, _>(EventPriority::LAST, move |event| {
                let Some(from) = event.switched_from() else {
                    return;
                };
                let _ = tx.send(ApiEvent::PlayerSwitch {
                    player_id: event.player_id().as_u64(),
                    username: event.player.profile().username.clone(),
                    from_server: Some(from.as_str().to_string()),
                    to_server: event.server.as_str().to_string(),
                    timestamp: now_iso8601(),
                });
            });

        // ServerStateChangeEvent → ServerStateChange
        let tx = self.event_tx.clone();
        ctx.event_bus()
            .subscribe::<ServerStateChangeEvent, _>(EventPriority::LAST, move |event| {
                let _ = tx.send(ApiEvent::ServerStateChange {
                    server_id: event.server.as_str().to_string(),
                    old_state: format!("{:?}", event.old_state),
                    new_state: format!("{:?}", event.new_state),
                    timestamp: now_iso8601(),
                });
            });

        // ConfigReloadEvent → ConfigReload
        let tx = self.event_tx.clone();
        ctx.event_bus()
            .subscribe::<ConfigReloadEvent, _>(EventPriority::LAST, move |event| {
                let ids = |servers: &[ServerId]| -> Vec<String> {
                    servers.iter().map(|s| s.as_str().to_string()).collect()
                };
                let _ = tx.send(ApiEvent::ConfigReload {
                    provider: event.provider.clone(),
                    added: ids(&event.added),
                    removed: ids(&event.removed),
                    updated: ids(&event.updated),
                    timestamp: now_iso8601(),
                });
            });

        // BackendHealthEvent → BackendHealthChange
        let tx = self.event_tx.clone();
        ctx.event_bus()
            .subscribe::<BackendHealthEvent, _>(EventPriority::LAST, move |event| {
                let _ = tx.send(ApiEvent::BackendHealthChange {
                    address: format_address(&event.address),
                    server_ids: event
                        .servers
                        .iter()
                        .map(|s| s.as_str().to_string())
                        .collect(),
                    state: event.state.as_str().to_string(),
                    timestamp: now_iso8601(),
                });
            });
    }
}

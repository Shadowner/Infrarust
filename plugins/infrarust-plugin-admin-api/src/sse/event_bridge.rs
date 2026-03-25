use std::sync::Arc;

use infrarust_api::event::EventPriority;
use infrarust_api::event::bus::EventBusExt;
use infrarust_api::events::connection::ServerSwitchEvent;
use infrarust_api::events::lifecycle::{DisconnectEvent, PostLoginEvent};
use infrarust_api::events::proxy::{ConfigReloadEvent, ServerStateChangeEvent};
use infrarust_api::plugin::PluginContext;
use infrarust_api::services::player_registry::PlayerRegistry;
use tokio::sync::broadcast;

use crate::state::ApiEvent;
use crate::util::now_iso8601;

/// Bridges proxy EventBus events to the API broadcast channel.
///
/// Subscribes to lifecycle and proxy events at `EventPriority::LAST`
/// (observe only, no modifications) and converts them to `ApiEvent`
/// variants that SSE clients consume.
pub struct EventBridge {
    event_tx: broadcast::Sender<ApiEvent>,
    player_registry: Arc<dyn PlayerRegistry>,
}

impl EventBridge {
    pub fn new(
        event_tx: broadcast::Sender<ApiEvent>,
        player_registry: Arc<dyn PlayerRegistry>,
    ) -> Self {
        Self {
            event_tx,
            player_registry,
        }
    }

    /// Registers all event listeners on the EventBus via the plugin context.
    /// Listeners are tracked by the context for automatic cleanup on disable.
    pub fn register_listeners(&self, ctx: &dyn PluginContext) {
        // PostLoginEvent → PlayerJoin
        let tx = self.event_tx.clone();
        ctx.event_bus()
            .subscribe::<PostLoginEvent, _>(EventPriority::LAST, move |event| {
                let _ = tx.send(ApiEvent::PlayerJoin {
                    player_id: event.player_id.as_u64(),
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
                    player_id: event.player_id.as_u64(),
                    username: event.username.clone(),
                    last_server: event.last_server.as_ref().map(|s| s.as_str().to_string()),
                    timestamp: now_iso8601(),
                });
            });

        // ServerSwitchEvent → PlayerSwitch
        let tx = self.event_tx.clone();
        let registry = Arc::clone(&self.player_registry);
        ctx.event_bus()
            .subscribe::<ServerSwitchEvent, _>(EventPriority::LAST, move |event| {
                let username = registry
                    .get_player_by_id(event.player_id)
                    .map(|p| p.profile().username.clone())
                    .unwrap_or_else(|| format!("Player({})", event.player_id.as_u64()));

                let _ = tx.send(ApiEvent::PlayerSwitch {
                    player_id: event.player_id.as_u64(),
                    username,
                    from_server: Some(event.previous_server.as_str().to_string()),
                    to_server: event.new_server.as_str().to_string(),
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
            .subscribe::<ConfigReloadEvent, _>(EventPriority::LAST, move |_event| {
                let _ = tx.send(ApiEvent::ConfigReload {
                    timestamp: now_iso8601(),
                });
            });
    }
}

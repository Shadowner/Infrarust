pub mod config;
pub mod handler;
pub mod state;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use infrarust_api::error::PluginError;
use infrarust_api::event::BoxFuture;
use infrarust_api::limbo::handler::HandlerResult;
use infrarust_api::plugin::{Plugin, PluginContext, PluginMetadata};
use infrarust_api::services::server_manager::ServerState;
use infrarust_api::types::Component;

use crate::config::load_or_create_config;
use crate::handler::ServerWakeHandler;
use crate::state::WakeState;

pub struct ServerWakePlugin {
    state: Mutex<Option<Arc<WakeState>>>,
}

impl ServerWakePlugin {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(None),
        }
    }
}

impl Default for ServerWakePlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for ServerWakePlugin {
    fn metadata(&self) -> PluginMetadata {
        PluginMetadata::new("server_wake", "Server Wake Plugin", "0.1.0")
            .author("Infrarust")
            .description("Holds players in limbo while their target server starts up")
    }

    fn on_enable<'a>(
        &'a self,
        ctx: &'a dyn PluginContext,
    ) -> BoxFuture<'a, Result<(), PluginError>> {
        Box::pin(async move {
            let data_dir = PathBuf::from("plugins/server_wake");
            let config_path = data_dir.join("config.toml");
            let config = load_or_create_config(&config_path)
                .await
                .map_err(|e| PluginError::InitFailed(e.to_string()))?;

            let state = Arc::new(WakeState::new(config));

            ctx.register_limbo_handler(Box::new(ServerWakeHandler {
                state: Arc::clone(&state),
                server_manager: ctx.server_manager_handle(),
                config_service: ctx.config_service_handle()
            }));

            let wake_state = Arc::clone(&state);
            let sm = ctx.server_manager_handle();
            sm.on_state_change(Box::new(move |server_id, _old_state, new_state| {
                handle_state_change(&wake_state, server_id, new_state);
            }));

            {
                let mut guard = self
                    .state
                    .lock()
                    .expect("server_wake plugin state mutex poisoned");
                *guard = Some(state);
            }

            tracing::info!("[ServerWakePlugin] Enabled — limbo handler 'server_wake' registered");
            Ok(())
        })
    }

    fn on_disable(&self) -> BoxFuture<'_, Result<(), PluginError>> {
        let state = self
            .state
            .lock()
            .expect("server_wake plugin state mutex poisoned")
            .take();

        Box::pin(async move {
            if let Some(state) = state {
                let keys: Vec<_> = state.waiting.iter().map(|e| *e.key()).collect();
                for player_id in keys {
                    if let Some((_, entry)) = state.waiting.remove(&player_id) {
                        entry.cancel.cancel();
                    }
                }
            }
            tracing::info!("[ServerWakePlugin] Disabled");
            Ok(())
        })
    }
}

fn handle_state_change(
    state: &WakeState,
    server_id: &infrarust_api::types::ServerId,
    new_state: ServerState,
) {
    match new_state {
        ServerState::Online => {
            let players = state.players_waiting_for(server_id);
            if players.is_empty() {
                return;
            }
            tracing::info!(
                server = %server_id,
                count = players.len(),
                "server online, releasing waiting player(s)"
            );
            let vars: &[(&str, &str)] = &[("server", server_id.as_str())];
            let ready_title = Component::from_legacy_format(
                &state.config.messages.ready_title, vars,
            );
            let ready_subtitle = Component::from_legacy_format(
                &state.config.messages.ready_subtitle, vars,
            );

            for player_id in players {
                if let Some((_, entry)) = state.waiting.remove(&player_id) {
                    entry.cancel.cancel();
                    let title = infrarust_api::types::TitleData::new(
                        ready_title.clone(),
                        ready_subtitle.clone(),
                    )
                    .fade_in(0)
                    .stay(40);
                    let _ = entry.session_handle.send_title(title);
                    entry.session_handle.complete(HandlerResult::Accept);
                }
            }
        }
        ServerState::Crashed => {
            let players = state.players_waiting_for(server_id);
            if players.is_empty() {
                return;
            }
            tracing::warn!(
                server = %server_id,
                count = players.len(),
                "server crashed, kicking waiting player(s)"
            );
            for player_id in players {
                if let Some((_, entry)) = state.waiting.remove(&player_id) {
                    entry.cancel.cancel();
                    entry.session_handle.complete(HandlerResult::Deny(
                        Component::from_legacy(&state.config.messages.failed_kick),
                    ));
                }
            }
        }
        _ => {}
    }
}

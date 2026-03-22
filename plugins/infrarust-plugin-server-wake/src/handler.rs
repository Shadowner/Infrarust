use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use infrarust_api::prelude::ConfigService;
use tokio_util::sync::CancellationToken;

use infrarust_api::event::BoxFuture;
use infrarust_api::limbo::context::LimboEntryContext;
use infrarust_api::limbo::handler::{HandlerResult, LimboHandler};
use infrarust_api::limbo::session::LimboSession;
use infrarust_api::services::server_manager::{ServerManager, ServerState};
use infrarust_api::types::{Component, PlayerId, ServerId, TitleData};

use crate::state::{WaitingEntry, WakeState, animated_dots};

pub struct ServerWakeHandler {
    pub(crate) state: Arc<WakeState>,
    pub(crate) server_manager: Arc<dyn ServerManager>,
    pub(crate) config_service: Arc<dyn ConfigService>,
}

impl ServerWakeHandler {
    fn target_server(session: &dyn LimboSession) -> Option<ServerId> {
        match session.entry_context() {
            LimboEntryContext::InitialConnection { target_server } => {
                Some(target_server.clone())
            }
            LimboEntryContext::KickedFromServer { server, .. } => Some(server.clone()),
            _ => None,
        }
    }

    fn build_title(title_tpl: &str, subtitle_tpl: &str, server: &ServerId, tick: u32, count: usize) -> TitleData {
        let dots = animated_dots(tick);
        let count_str = count.to_string();
        let vars = &[
            ("server", server.as_str()),
            ("dots", dots),
            ("count", count_str.as_str()),
        ];
        TitleData::new(
            Component::from_legacy_format(title_tpl, vars),
            Component::from_legacy_format(subtitle_tpl, vars),
        )
        .fade_in(0)
    }

    fn build_action_bar(state: &WakeState, server: &ServerId) -> Option<Component> {
        if !state.config.timing.show_waiting_count {
            return None;
        }
        let count = state.waiting_count_for(server).to_string();
        Some(Component::from_legacy_format(
            &state.config.messages.waiting_action_bar,
            &[("server", server.as_str()), ("count", &count)],
        ))
    }

    fn spawn_animation(
        state: Arc<WakeState>,
        handle: infrarust_api::limbo::handle::SessionHandle,
        player_id: PlayerId,
        server: ServerId,
        cancel: CancellationToken,
    ) {
        let interval_secs = state.config.timing.title_refresh_interval_seconds;
        let messages = state.config.messages.clone();
        let show_action_bar = state.config.timing.show_waiting_count;

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
            interval.tick().await;

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let tick = {
                            let Some(entry) = state.waiting.get(&player_id) else {
                                break;
                            };
                            entry.tick.fetch_add(1, Ordering::Relaxed) + 1
                        };

                        let dots = animated_dots(tick);
                        let count = state.waiting_count_for(&server).to_string();
                        let vars: &[(&str, &str)] = &[
                            ("server", server.as_str()),
                            ("dots", dots),
                            ("count", &count),
                        ];

                        let title = TitleData::new(
                            Component::from_legacy_format(&messages.starting_title, vars),
                            Component::from_legacy_format(&messages.starting_subtitle, vars),
                        )
                        .fade_in(0);
                        let _ = handle.send_title(title);

                        if show_action_bar {
                            let ab = Component::from_legacy_format(
                                &messages.waiting_action_bar,
                                &[("server", server.as_str()), ("count", &count)],
                            );
                            let _ = handle.send_action_bar(ab);
                        }
                    }
                    () = cancel.cancelled() => { break; }
                }
            }
        });
    }

    fn spawn_timeout(
        state: Arc<WakeState>,
        handle: infrarust_api::limbo::handle::SessionHandle,
        player_id: PlayerId,
        cancel: CancellationToken,
    ) {
        let timeout_secs = state.config.timing.start_timeout_seconds;
        if timeout_secs == 0 {
            return;
        }
        let kick_msg = state.config.messages.timeout_kick.clone();

        tokio::spawn(async move {
            tokio::select! {
                () = tokio::time::sleep(Duration::from_secs(timeout_secs)) => {
                    if state.waiting.remove(&player_id).is_some() {
                        cancel.cancel();
                        tracing::info!(
                            player_id = player_id.as_u64(),
                            "server wake timeout, kicking player"
                        );
                        handle.complete(HandlerResult::Deny(
                            Component::from_legacy(&kick_msg),
                        ));
                    }
                }
                () = cancel.cancelled() => {}
            }
        });
    }
}

impl LimboHandler for ServerWakeHandler {
    fn name(&self) -> &str {
        "server_wake"
    }

    fn on_player_enter<'a>(
        &'a self,
        session: &'a dyn LimboSession,
    ) -> BoxFuture<'a, HandlerResult> {
        Box::pin(async move {
            let Some(target) = Self::target_server(session) else {
                return HandlerResult::Deny(
                    Component::text("Server wake: no target server in entry context"),
                );
            };

            let Some(server_config) = self.config_service.get_server_config(&target) else {
                return HandlerResult::Deny(
                    Component::text("Server wake: unknown server"),
                );
            };

            if !server_config.has_server_manager {
                return HandlerResult::Accept;
            }

            let Some(current_state) = self.server_manager.get_state(&target) else {
                return HandlerResult::Accept;
            };

            match current_state {
                ServerState::Online => return HandlerResult::Accept,
                ServerState::Sleeping | ServerState::Crashed | ServerState::Offline => {
                    if let Err(e) = self.server_manager.start(&target).await {
                        tracing::error!(server = %target, error = %e, "failed to start server");
                        return HandlerResult::Deny(Component::from_legacy(
                            &self.state.config.messages.failed_kick,
                        ));
                    }
                }
                ServerState::Starting | ServerState::Stopping => {}
                _ => {}
            }

            let handle = session.handle();

            let (title_tpl, subtitle_tpl) = if current_state == ServerState::Stopping {
                (
                    &self.state.config.messages.stopping_title,
                    &self.state.config.messages.stopping_subtitle,
                )
            } else {
                (
                    &self.state.config.messages.starting_title,
                    &self.state.config.messages.starting_subtitle,
                )
            };

            let cancel = CancellationToken::new();
            let entry = WaitingEntry {
                target_server: target.clone(),
                session_handle: handle.clone(),
                started_waiting: tokio::time::Instant::now(),
                tick: std::sync::atomic::AtomicU32::new(0),
                cancel: cancel.clone(),
            };
            self.state.waiting.insert(session.player_id(), entry);

            let count = self.state.waiting_count_for(&target);
            let title = Self::build_title(title_tpl, subtitle_tpl, &target, 0, count);
            let _ = session.send_title(title);

            if let Some(ab) = Self::build_action_bar(&self.state, &target) {
                let _ = session.send_action_bar(ab);
            }

            Self::spawn_animation(
                Arc::clone(&self.state),
                handle.clone(),
                session.player_id(),
                target.clone(),
                cancel.clone(),
            );

            Self::spawn_timeout(
                Arc::clone(&self.state),
                handle,
                session.player_id(),
                cancel,
            );

            HandlerResult::Hold
        })
    }

    fn on_disconnect(&self, player_id: PlayerId) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            if let Some((_, entry)) = self.state.waiting.remove(&player_id) {
                entry.cancel.cancel();
                tracing::debug!(
                    player_id = player_id.as_u64(),
                    server = %entry.target_server,
                    "player disconnected while waiting for server wake"
                );
            }
        })
    }
}

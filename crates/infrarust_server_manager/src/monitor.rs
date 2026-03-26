//! Per-server monitoring: adaptive polling and auto-shutdown.

use std::sync::Arc;
use std::time::Instant;

use tokio_util::sync::CancellationToken;

use crate::error::ServerManagerError;
use crate::provider::ProviderStatus;
use crate::service::{PlayerCounter, ServerManagerService};
use crate::state::ServerState;

/// Per-server monitoring task.
///
/// Polls the provider for status changes, notifies waiters on transitions,
/// and triggers auto-shutdown when no players are connected.
pub async fn monitor_server(
    service: Arc<ServerManagerService>,
    server_id: String,
    player_counter: Arc<dyn PlayerCounter>,
    shutdown: CancellationToken,
) {
    tracing::info!(server = %server_id, "monitoring task started");

    let mut fast_poll = false;

    loop {
        let interval = if fast_poll {
            service.get_poll_interval(&server_id)
        } else {
            service.get_poll_interval(&server_id) * 6
        };

        tokio::select! {
            biased;
            () = shutdown.cancelled() => {
                tracing::info!(server = %server_id, "monitoring task shutting down");
                break;
            }
            () = tokio::time::sleep(interval) => {}
        }

        // Skip polling if server is Crashed (wait for ensure_started)
        if let Some(state) = service.get_state(&server_id)
            && state == ServerState::Crashed
        {
            continue;
        }

        // 1. Poll provider status
        let new_status = match service.check_provider_status(&server_id).await {
            Ok(status) => status,
            Err(e) => {
                tracing::warn!(server = %server_id, "status check failed: {e}");
                continue;
            }
        };

        // 2. Update state and detect transitions
        if let Some((_old, new_state)) = service.update_state(&server_id, new_status) {
            match new_state {
                ServerState::Online => {
                    service.notify_waiters(&server_id, &Ok(()));
                    fast_poll = false;
                }
                ServerState::Crashed | ServerState::Sleeping => {
                    service.notify_waiters(
                        &server_id,
                        &Err(ServerManagerError::ProcessExited {
                            server_id: server_id.clone(),
                            exit_code: None,
                        }),
                    );
                    fast_poll = false;
                }
                ServerState::Starting | ServerState::Stopping => {
                    fast_poll = true;
                }
                _ => {}
            }
        }

        // 3. Check auto-shutdown (only when running)
        if new_status == ProviderStatus::Running {
            let player_count = player_counter.count_by_server(&server_id);
            check_auto_shutdown(&service, &server_id, player_count).await;
        }
    }
}

/// Checks if a server should be auto-shutdown due to inactivity.
async fn check_auto_shutdown(service: &ServerManagerService, server_id: &str, player_count: usize) {
    let should_stop = {
        let Some(mut entry) = service.entries.get_mut(server_id) else {
            return;
        };

        if player_count > 0 {
            entry.last_player_seen = Some(Instant::now());
            return;
        }

        let Some(shutdown_after) = entry.shutdown_after else {
            return;
        };

        let Some(last_seen) = entry.last_player_seen else {
            entry.last_player_seen = Some(Instant::now());
            return;
        };

        let result = last_seen.elapsed() >= shutdown_after;
        drop(entry);
        result
    };

    if should_stop {
        tracing::info!(
            server = %server_id,
            "auto-shutdown: no players for shutdown_after duration"
        );
        if let Err(e) = service.stop_server(server_id).await {
            tracing::warn!(server = %server_id, "auto-shutdown failed: {e}");
        }
    }
}

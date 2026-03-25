use std::sync::Arc;
use std::time::{Duration, Instant};

use infrarust_api::services::ban_service::BanService;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::services::server_manager::{ServerManager, ServerState};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::state::ApiEvent;
use crate::util::{get_memory_rss, now_iso8601};

/// Periodic task that emits `ApiEvent::StatsTick` every 5 seconds.
pub struct StatsTicker {
    event_tx: broadcast::Sender<ApiEvent>,
    player_registry: Arc<dyn PlayerRegistry>,
    server_manager: Arc<dyn ServerManager>,
    ban_service: Arc<dyn BanService>,
    start_time: Instant,
    shutdown: CancellationToken,
}

impl StatsTicker {
    pub fn new(
        event_tx: broadcast::Sender<ApiEvent>,
        player_registry: Arc<dyn PlayerRegistry>,
        server_manager: Arc<dyn ServerManager>,
        ban_service: Arc<dyn BanService>,
        start_time: Instant,
        shutdown: CancellationToken,
    ) -> Self {
        Self {
            event_tx,
            player_registry,
            server_manager,
            ban_service,
            start_time,
            shutdown,
        }
    }

    pub async fn run(self) {
        let mut interval = tokio::time::interval(Duration::from_secs(5));

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let players_online = self.player_registry.online_count();
                    let servers = self.server_manager.get_all_servers();
                    let servers_online = servers
                        .iter()
                        .filter(|(_, state)| matches!(state, ServerState::Online))
                        .count();
                    let bans_active = self
                        .ban_service
                        .get_all_bans()
                        .await
                        .map(|b| b.len())
                        .unwrap_or(0);

                    let _ = self.event_tx.send(ApiEvent::StatsTick {
                        players_online,
                        servers_online,
                        bans_active,
                        uptime_seconds: self.start_time.elapsed().as_secs(),
                        memory_rss_bytes: get_memory_rss(),
                        timestamp: now_iso8601(),
                    });
                }
                _ = self.shutdown.cancelled() => break,
            }
        }
    }
}

use std::sync::atomic::AtomicU32;

use dashmap::DashMap;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use infrarust_api::limbo::handle::SessionHandle;
use infrarust_api::types::{PlayerId, ServerId};

use crate::config::ServerWakeConfig;

pub struct WakeState {
    pub waiting: DashMap<PlayerId, WaitingEntry>,
    pub config: ServerWakeConfig,
}

impl WakeState {
    pub fn new(config: ServerWakeConfig) -> Self {
        Self {
            waiting: DashMap::new(),
            config,
        }
    }

    pub fn waiting_count_for(&self, server: &ServerId) -> usize {
        self.waiting
            .iter()
            .filter(|entry| entry.value().target_server == *server)
            .count()
    }

    pub fn players_waiting_for(&self, server: &ServerId) -> Vec<PlayerId> {
        self.waiting
            .iter()
            .filter(|entry| entry.value().target_server == *server)
            .map(|entry| *entry.key())
            .collect()
    }
}

#[derive(Debug)]
pub struct WaitingEntry {
    pub target_server: ServerId,
    pub session_handle: SessionHandle,
    pub started_waiting: Instant,
    pub tick: AtomicU32,
    pub cancel: CancellationToken,
}

pub fn animated_dots(tick: u32) -> &'static str {
    match tick % 3 {
        0 => ".",
        1 => "..",
        _ => "...",
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn test_dots_animation_cycles() {
        assert_eq!(animated_dots(0), ".");
        assert_eq!(animated_dots(1), "..");
        assert_eq!(animated_dots(2), "...");
        assert_eq!(animated_dots(3), ".");
        assert_eq!(animated_dots(4), "..");
        assert_eq!(animated_dots(5), "...");
    }

    #[test]
    fn test_wake_state_empty() {
        let state = WakeState::new(ServerWakeConfig::default());
        assert_eq!(state.waiting_count_for(&ServerId::new("test")), 0);
        assert!(state.players_waiting_for(&ServerId::new("test")).is_empty());
    }

    #[test]
    fn test_animated_dots_large_tick() {
        assert_eq!(animated_dots(999), ".");
        assert_eq!(animated_dots(1000), "..");
        assert_eq!(animated_dots(1001), "...");
    }
}

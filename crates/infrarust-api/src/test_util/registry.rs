use std::net::IpAddr;
use std::sync::{Arc, Mutex};

use crate::player::Player;
use crate::services::player_registry::PlayerRegistry;
use crate::types::{PlayerId, ServerId};

use super::lock;

#[derive(Default)]
pub struct MockPlayerRegistry {
    players: Mutex<Vec<Arc<dyn Player>>>,
}

impl MockPlayerRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with<P: Player + 'static>(self, player: Arc<P>) -> Self {
        self.add(player);
        self
    }

    pub fn add<P: Player + 'static>(&self, player: Arc<P>) {
        self.add_dyn(player);
    }

    pub fn add_dyn(&self, player: Arc<dyn Player>) {
        let mut players = lock(&self.players);
        players.retain(|p| p.id() != player.id());
        players.push(player);
    }

    pub fn remove(&self, id: PlayerId) -> Option<Arc<dyn Player>> {
        let mut players = lock(&self.players);
        let at = players.iter().position(|p| p.id() == id)?;
        Some(players.remove(at))
    }

    fn find(&self, pick: impl Fn(&dyn Player) -> bool) -> Option<Arc<dyn Player>> {
        lock(&self.players)
            .iter()
            .find(|p| pick(p.as_ref()))
            .cloned()
    }

    fn filter(&self, pick: impl Fn(&dyn Player) -> bool) -> Vec<Arc<dyn Player>> {
        lock(&self.players)
            .iter()
            .filter(|p| pick(p.as_ref()))
            .cloned()
            .collect()
    }
}

impl std::fmt::Debug for MockPlayerRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<String> = lock(&self.players)
            .iter()
            .map(|p| p.profile().username.clone())
            .collect();
        f.debug_struct("MockPlayerRegistry")
            .field("players", &names)
            .finish()
    }
}

impl crate::services::player_registry::private::Sealed for MockPlayerRegistry {}

impl PlayerRegistry for MockPlayerRegistry {
    fn get_player(&self, username: &str) -> Option<Arc<dyn Player>> {
        self.find(|p| p.profile().username.eq_ignore_ascii_case(username))
    }

    fn get_player_by_uuid(&self, uuid: &uuid::Uuid) -> Option<Arc<dyn Player>> {
        self.find(|p| p.profile().uuid == *uuid)
    }

    fn get_player_by_id(&self, id: PlayerId) -> Option<Arc<dyn Player>> {
        self.find(|p| p.id() == id)
    }

    fn get_players_by_ip(&self, ip: IpAddr) -> Vec<Arc<dyn Player>> {
        let ip = ip.to_canonical();
        self.filter(|p| p.remote_addr().ip().to_canonical() == ip)
    }

    fn get_players_on_server(&self, server: &ServerId) -> Vec<Arc<dyn Player>> {
        self.filter(|p| p.current_server().as_ref() == Some(server))
    }

    fn get_all_players(&self) -> Vec<Arc<dyn Player>> {
        lock(&self.players).clone()
    }

    fn online_count(&self) -> usize {
        lock(&self.players).len()
    }

    fn online_count_on(&self, server: &ServerId) -> usize {
        self.get_players_on_server(server).len()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::test_util::MockPlayer;

    #[test]
    fn looks_players_up_like_the_proxy_does() {
        let steve = MockPlayer::new(1, "Steve").on_server("hub").into_arc();
        let alex = MockPlayer::new(2, "Alex")
            .with_remote_addr(([10, 0, 0, 2], 1).into())
            .into_arc();
        let registry = MockPlayerRegistry::new().with(steve.clone()).with(alex);

        assert_eq!(registry.get_player("steve").unwrap().id(), PlayerId::new(1));
        assert!(registry.get_player_by_uuid(&steve.profile().uuid).is_some());
        assert_eq!(registry.get_players_by_ip([10, 0, 0, 2].into()).len(), 1);
        assert_eq!(registry.online_count_on(&ServerId::new("hub")), 1);
        assert_eq!(registry.online_count(), 2);

        assert!(registry.remove(PlayerId::new(1)).is_some());
        assert!(registry.get_player_by_id(PlayerId::new(1)).is_none());
    }
}

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use crate::error::PlayerError;
use crate::event::BoxFuture;
use crate::player::Player;
use crate::types::{
    Component, GameProfile, PlayerId, ProtocolVersion, RawPacket, ServerId, TitleData,
};

use super::{MockPermissionChecker, lock};

#[derive(Debug, Default)]
struct Recorded {
    disconnected: bool,
    server: Option<ServerId>,
    messages: Vec<Component>,
    titles: Vec<TitleData>,
    action_bars: Vec<Component>,
    packets: Vec<RawPacket>,
    kicks: Vec<Component>,
    switches: Vec<ServerId>,
    permission_refreshes: usize,
}

#[derive(Debug)]
pub struct MockPlayer {
    id: PlayerId,
    profile: GameProfile,
    protocol_version: ProtocolVersion,
    remote_addr: SocketAddr,
    online_mode: bool,
    active: bool,
    connected_at: SystemTime,
    permissions: Arc<MockPermissionChecker>,
    recorded: Mutex<Recorded>,
}

impl MockPlayer {
    #[must_use]
    pub fn new(id: u64, username: &str) -> Self {
        Self {
            id: PlayerId::new(id),
            profile: GameProfile {
                uuid: uuid::Uuid::from_u128(u128::from(id)),
                username: username.to_string(),
                properties: Vec::new(),
            },
            protocol_version: ProtocolVersion::MINECRAFT_1_21,
            remote_addr: SocketAddr::from(([127, 0, 0, 1], 25565)),
            online_mode: false,
            active: true,
            connected_at: SystemTime::now(),
            permissions: Arc::new(MockPermissionChecker::new()),
            recorded: Mutex::new(Recorded::default()),
        }
    }

    #[must_use]
    pub fn with_profile(mut self, profile: GameProfile) -> Self {
        self.profile = profile;
        self
    }

    #[must_use]
    pub const fn with_protocol_version(mut self, version: ProtocolVersion) -> Self {
        self.protocol_version = version;
        self
    }

    #[must_use]
    pub const fn with_remote_addr(mut self, addr: SocketAddr) -> Self {
        self.remote_addr = addr;
        self
    }

    #[must_use]
    pub const fn online_mode(mut self, online: bool) -> Self {
        self.online_mode = online;
        self
    }

    #[must_use]
    pub const fn passive(mut self) -> Self {
        self.active = false;
        self
    }

    #[must_use]
    pub fn on_server(self, server: impl Into<ServerId>) -> Self {
        lock(&self.recorded).server = Some(server.into());
        self
    }

    #[must_use]
    pub fn with_permission(self, node: &str) -> Self {
        self.permissions.set(node, true);
        self
    }

    #[must_use]
    pub fn without_permission(self, node: &str) -> Self {
        self.permissions.set(node, false);
        self
    }

    #[must_use]
    pub fn with_all_permissions(self) -> Self {
        self.with_permission(crate::permissions::WILDCARD)
    }

    #[must_use]
    pub fn with_permissions(mut self, permissions: Arc<MockPermissionChecker>) -> Self {
        self.permissions = permissions;
        self
    }

    #[must_use]
    pub fn into_arc(self) -> Arc<Self> {
        Arc::new(self)
    }

    pub fn permissions(&self) -> &Arc<MockPermissionChecker> {
        &self.permissions
    }

    pub fn set_server(&self, server: Option<ServerId>) {
        lock(&self.recorded).server = server;
    }

    #[must_use]
    pub fn messages(&self) -> Vec<Component> {
        lock(&self.recorded).messages.clone()
    }

    #[must_use]
    pub fn sent_text(&self) -> String {
        lock(&self.recorded)
            .messages
            .iter()
            .map(Component::to_plain)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[must_use]
    pub fn titles(&self) -> Vec<TitleData> {
        lock(&self.recorded).titles.clone()
    }

    #[must_use]
    pub fn action_bars(&self) -> Vec<Component> {
        lock(&self.recorded).action_bars.clone()
    }

    #[must_use]
    pub fn packets(&self) -> Vec<RawPacket> {
        lock(&self.recorded).packets.clone()
    }

    #[must_use]
    pub fn kicks(&self) -> Vec<Component> {
        lock(&self.recorded).kicks.clone()
    }

    #[must_use]
    pub fn switches(&self) -> Vec<ServerId> {
        lock(&self.recorded).switches.clone()
    }

    #[must_use]
    pub fn permission_refreshes(&self) -> usize {
        lock(&self.recorded).permission_refreshes
    }

    fn deliver(&self, record: impl FnOnce(&mut Recorded)) -> Result<(), PlayerError> {
        let mut recorded = lock(&self.recorded);
        if recorded.disconnected {
            return Err(PlayerError::Disconnected);
        }
        if !self.active {
            return Err(PlayerError::NotActive);
        }
        record(&mut recorded);
        Ok(())
    }
}

impl crate::player::private::Sealed for MockPlayer {}

impl Player for MockPlayer {
    fn id(&self) -> PlayerId {
        self.id
    }

    fn profile(&self) -> &GameProfile {
        &self.profile
    }

    fn protocol_version(&self) -> ProtocolVersion {
        self.protocol_version
    }

    fn remote_addr(&self) -> SocketAddr {
        self.remote_addr
    }

    fn current_server(&self) -> Option<ServerId> {
        lock(&self.recorded).server.clone()
    }

    fn is_connected(&self) -> bool {
        !lock(&self.recorded).disconnected
    }

    fn is_active(&self) -> bool {
        self.active
    }

    fn disconnect(&self, reason: Component) -> BoxFuture<'_, ()> {
        let mut recorded = lock(&self.recorded);
        if !recorded.disconnected {
            recorded.disconnected = true;
            recorded.kicks.push(reason);
        }
        Box::pin(async {})
    }

    fn send_message(&self, message: Component) -> Result<(), PlayerError> {
        self.deliver(|r| r.messages.push(message))
    }

    fn send_title(&self, title: TitleData) -> Result<(), PlayerError> {
        self.deliver(|r| r.titles.push(title))
    }

    fn send_action_bar(&self, message: Component) -> Result<(), PlayerError> {
        self.deliver(|r| r.action_bars.push(message))
    }

    fn send_packet(&self, packet: RawPacket) -> Result<(), PlayerError> {
        self.deliver(|r| r.packets.push(packet))
    }

    fn switch_server(&self, target: ServerId) -> BoxFuture<'_, Result<(), PlayerError>> {
        let result = self.deliver(|r| {
            r.switches.push(target.clone());
            r.server = Some(target);
        });
        Box::pin(async move { result })
    }

    fn is_online_mode(&self) -> bool {
        self.online_mode
    }

    fn has_permission(&self, permission: &str) -> bool {
        crate::permissions::PermissionChecker::has_permission(&*self.permissions, permission)
    }

    fn refresh_permissions(&self) -> BoxFuture<'_, ()> {
        lock(&self.recorded).permission_refreshes += 1;
        Box::pin(async {})
    }

    fn connected_at(&self) -> SystemTime {
        self.connected_at
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::test_util::block_on;

    #[test]
    fn records_what_the_plugin_sent() {
        let player = MockPlayer::new(7, "Alex").on_server("hub");
        player.send_message(Component::text("one")).unwrap();
        player.send_message(Component::text("two")).unwrap();
        player.send_action_bar(Component::text("bar")).unwrap();
        block_on(player.switch_server(ServerId::new("pvp"))).unwrap();

        assert_eq!(player.sent_text(), "one\ntwo");
        assert_eq!(player.action_bars().len(), 1);
        assert_eq!(player.switches(), [ServerId::new("pvp")]);
        assert_eq!(player.current_server(), Some(ServerId::new("pvp")));
        assert_eq!(player.profile().username, "Alex");
        assert_eq!(player.id(), PlayerId::new(7));
    }

    #[test]
    fn a_kicked_player_refuses_further_messages() {
        let player = MockPlayer::new(1, "Steve");
        block_on(player.disconnect(Component::text("bye")));
        block_on(player.disconnect(Component::text("again")));
        assert!(!player.is_connected());
        assert_eq!(player.kicks().len(), 1);
        assert!(matches!(
            player.send_message(Component::text("late")),
            Err(PlayerError::Disconnected)
        ));
    }

    #[test]
    fn a_passive_player_is_not_active() {
        let player = MockPlayer::new(1, "Steve").passive();
        assert!(matches!(
            player.send_title(TitleData::new(Component::text("t"), Component::text("s"))),
            Err(PlayerError::NotActive)
        ));
    }

    #[test]
    fn permissions_are_configurable_and_mutable() {
        let player = MockPlayer::new(1, "Steve")
            .with_permission("hub.use")
            .without_permission("hub.admin");
        assert!(player.has_permission("hub.use"));
        assert!(!player.has_permission("hub.admin"));
        player.permissions().set("hub.admin", true);
        assert!(player.has_permission("hub.admin"));
        assert!(
            MockPlayer::new(2, "Op")
                .with_all_permissions()
                .has_permission("x.y")
        );
        block_on(player.refresh_permissions());
        assert_eq!(player.permission_refreshes(), 1);
    }
}

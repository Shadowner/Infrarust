use std::sync::Arc;

use uuid::Uuid;

use crate::event::Event;
use crate::player::{Player, ResourcePackStatus};
use crate::types::PlayerId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ResourcePackOrigin {
    Proxy,
    Backend,
}

impl ResourcePackOrigin {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Proxy => "proxy",
            Self::Backend => "backend",
        }
    }
}

#[non_exhaustive]
pub struct PlayerResourcePackStatusEvent {
    pub player: Arc<dyn Player>,
    pub pack_id: Option<Uuid>,
    pub status: ResourcePackStatus,
    pub origin: ResourcePackOrigin,
}

impl PlayerResourcePackStatusEvent {
    pub fn new(
        player: Arc<dyn Player>,
        pack_id: Option<Uuid>,
        status: ResourcePackStatus,
        origin: ResourcePackOrigin,
    ) -> Self {
        Self {
            player,
            pack_id,
            status,
            origin,
        }
    }

    pub fn player_id(&self) -> PlayerId {
        self.player.id()
    }
}

impl Event for PlayerResourcePackStatusEvent {}

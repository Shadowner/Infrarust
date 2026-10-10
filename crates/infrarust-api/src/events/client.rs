use std::sync::Arc;

use crate::event::Event;
use crate::events::packet::PacketDirection;
use crate::player::{ClientSettings, Player};

#[non_exhaustive]
pub struct PlayerClientBrandEvent {
    pub player: Arc<dyn Player>,
    pub brand: String,
}

impl PlayerClientBrandEvent {
    pub fn new(player: Arc<dyn Player>, brand: String) -> Self {
        Self { player, brand }
    }
}

crate::events::player_event!(PlayerClientBrandEvent);

impl Event for PlayerClientBrandEvent {}

#[non_exhaustive]
pub struct PlayerSettingsChangedEvent {
    pub player: Arc<dyn Player>,
    pub settings: ClientSettings,
}

impl PlayerSettingsChangedEvent {
    pub fn new(player: Arc<dyn Player>, settings: ClientSettings) -> Self {
        Self { player, settings }
    }
}

crate::events::player_event!(PlayerSettingsChangedEvent);

impl Event for PlayerSettingsChangedEvent {}

#[non_exhaustive]
pub struct PlayerChannelRegisterEvent {
    pub player: Arc<dyn Player>,
    pub channels: Vec<String>,
    pub direction: PacketDirection,
}

impl PlayerChannelRegisterEvent {
    pub fn new(player: Arc<dyn Player>, channels: Vec<String>, direction: PacketDirection) -> Self {
        Self {
            player,
            channels,
            direction,
        }
    }
}

crate::events::player_event!(PlayerChannelRegisterEvent);

impl Event for PlayerChannelRegisterEvent {}

use std::sync::Arc;

use bytes::Bytes;

use crate::messaging::{ChannelId, Endpoint, MessagePhase};
use crate::player::Player;

#[non_exhaustive]
pub struct PluginMessageEvent {
    pub player: Arc<dyn Player>,
    pub source: Endpoint,
    pub channel: ChannelId,
    pub raw_channel: String,
    pub data: Bytes,
    pub phase: MessagePhase,
    result: PluginMessageResult,
}

impl PluginMessageEvent {
    pub fn new(
        player: Arc<dyn Player>,
        source: Endpoint,
        channel: ChannelId,
        raw_channel: String,
        data: Bytes,
        phase: MessagePhase,
    ) -> Self {
        Self {
            player,
            source,
            channel,
            raw_channel,
            data,
            phase,
            result: PluginMessageResult::default(),
        }
    }

    pub fn from_client(&self) -> bool {
        self.source == Endpoint::Client
    }

    pub fn forward(&mut self) {
        self.result = PluginMessageResult::Forward;
    }

    pub fn handled(&mut self) {
        self.result = PluginMessageResult::Handled;
    }

    pub fn replace(&mut self, data: impl Into<Bytes>) {
        self.result = PluginMessageResult::Replace(data.into());
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum PluginMessageResult {
    #[default]
    Forward,
    Handled,
    Replace(Bytes),
}

crate::events::player_event!(PluginMessageEvent);

crate::event::resulted_event!(PluginMessageEvent, PluginMessageResult);

use crate::types::{Component, ServerId};

/// Describes why a player entered the limbo world.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum LimboEntryContext {
    InitialConnection {
        target_server: ServerId,
    },
    KickedFromServer {
        server: ServerId,
        reason: Component,
    },
    PluginRedirect {
        from_server: Option<ServerId>,
    },
}

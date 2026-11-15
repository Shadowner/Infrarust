pub(crate) mod bungeecord;
pub mod channels;
pub(crate) mod java_io;
pub mod messenger;
pub(crate) mod router;

use std::sync::Arc;

use infrarust_api::messaging::ChannelId;
use infrarust_api::types::ProtocolVersion as ApiVersion;
use infrarust_config::{BungeeCordChannelPermissions, PluginMessagingConfig, ServerConfig};
use infrarust_protocol::version::ProtocolVersion;

pub use channels::{ChannelRegistry, TrackingChannelRegistrar};
pub use messenger::ServerMessengerImpl;

pub struct PluginMessaging {
    channels: Arc<ChannelRegistry>,
    bungeecord: bool,
    permissions: BungeeCordChannelPermissions,
}

impl PluginMessaging {
    pub fn new(config: &PluginMessagingConfig) -> Self {
        Self {
            channels: Arc::new(ChannelRegistry::new()),
            bungeecord: config.bungeecord,
            permissions: config.bungeecord_permissions.clone(),
        }
    }

    pub const fn channels(&self) -> &Arc<ChannelRegistry> {
        &self.channels
    }

    pub const fn bungeecord_enabled(&self) -> bool {
        self.bungeecord
    }

    pub fn bungeecord_for(&self, server: &ServerConfig) -> bool {
        self.bungeecord && server.bungeecord_channel
    }

    pub(crate) const fn permissions(&self) -> &BungeeCordChannelPermissions {
        &self.permissions
    }

    pub(crate) fn announced_channels(
        &self,
        server: Option<&ServerConfig>,
        version: ProtocolVersion,
    ) -> Vec<String> {
        let mut names = self.channels.wire_names(version);
        if server.is_some_and(|server| self.bungeecord_for(server)) {
            let bungee = ChannelId::bungeecord()
                .wire_name(ApiVersion::new(version.0))
                .to_string();
            if !names.contains(&bungee) {
                names.push(bungee);
            }
        }
        names
    }
}

impl Default for PluginMessaging {
    fn default() -> Self {
        Self::new(&PluginMessagingConfig::default())
    }
}

pub struct PluginChannels {
    registrar: TrackingChannelRegistrar,
    messenger: Arc<ServerMessengerImpl>,
}

impl PluginChannels {
    pub fn new(
        plugin_id: &str,
        messaging: &PluginMessaging,
        players: Arc<crate::registry::ConnectionRegistry>,
    ) -> Self {
        Self {
            registrar: TrackingChannelRegistrar::new(Arc::clone(messaging.channels()), plugin_id),
            messenger: Arc::new(ServerMessengerImpl::new(players)),
        }
    }

    pub const fn registrar(&self) -> &TrackingChannelRegistrar {
        &self.registrar
    }

    pub fn messenger(&self) -> Arc<ServerMessengerImpl> {
        Arc::clone(&self.messenger)
    }

    pub fn cleanup(&self) {
        self.registrar.unregister_all();
    }
}

impl Default for PluginChannels {
    fn default() -> Self {
        Self::new(
            "",
            &PluginMessaging::default(),
            Arc::new(crate::registry::ConnectionRegistry::new()),
        )
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use infrarust_api::messaging::ChannelRegistrar;

    use super::*;

    #[test]
    fn a_plugin_loses_its_channels_on_cleanup() {
        let messaging = PluginMessaging::default();
        let players = Arc::new(crate::registry::ConnectionRegistry::new());
        let first = PluginChannels::new("first", &messaging, Arc::clone(&players));
        let second = PluginChannels::new("second", &messaging, players);
        let shared = ChannelId::modern("shared:main").unwrap();
        first.registrar().register(shared.clone());
        first
            .registrar()
            .register(ChannelId::pair("first:main", "First").unwrap());
        second.registrar().register(shared.clone());

        first.cleanup();
        assert!(first.registrar().channels().is_empty());
        assert!(messaging.channels().lookup("First").is_none());
        assert_eq!(messaging.channels().lookup("shared:main"), Some(shared));
        second.cleanup();
        assert!(messaging.channels().lookup("shared:main").is_none());
    }

    #[test]
    fn backends_hear_about_the_bungeecord_channel_only_when_enabled() {
        let server: ServerConfig =
            toml::from_str("addresses = [\"127.0.0.1:25565\"]\nbungeecord_channel = true").unwrap();
        let off = PluginMessaging::default();
        assert!(!off.bungeecord_for(&server));
        assert!(
            off.announced_channels(Some(&server), ProtocolVersion::V1_21)
                .is_empty()
        );
        let on = PluginMessaging::new(&PluginMessagingConfig {
            bungeecord: true,
            ..PluginMessagingConfig::default()
        });
        assert!(on.bungeecord_for(&server));
        assert_eq!(
            on.announced_channels(Some(&server), ProtocolVersion::V1_12_2),
            vec!["BungeeCord"]
        );
        assert_eq!(
            on.announced_channels(Some(&server), ProtocolVersion::V1_21),
            vec!["bungeecord:main"]
        );
        let not_opted: ServerConfig = toml::from_str("addresses = [\"127.0.0.1:25565\"]").unwrap();
        assert!(!on.bungeecord_for(&not_opted));
    }
}

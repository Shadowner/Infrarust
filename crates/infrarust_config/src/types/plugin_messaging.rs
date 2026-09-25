use serde::{Deserialize, Serialize};

use super::BungeeCordChannelPermissions;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginMessagingConfig {
    #[serde(default)]
    pub bungeecord: bool,

    #[serde(default)]
    pub bungeecord_permissions: BungeeCordChannelPermissions,
}

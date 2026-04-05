use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PermissionsConfig {
    pub admins: Vec<String>,

    /// Subcommands of `/ir` accessible to all players (not just admins).
    pub player_commands: Vec<String>,
}

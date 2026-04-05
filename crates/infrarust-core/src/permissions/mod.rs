//! Permission service — resolves admin UUIDs and provides permission checkers.

use std::collections::HashSet;
use std::sync::{Arc, OnceLock};

use dashmap::DashSet;
use uuid::Uuid;

use infrarust_api::permissions::{DefaultPermissionChecker, PermissionChecker, PermissionLevel};
use infrarust_config::PermissionsConfig;

const PERM_ADMIN: &str = "infrarust.admin";
const PERM_COMMAND_PREFIX: &str = "infrarust.command.";

struct SubcommandInfo {
    all: HashSet<String>,
    admin_only: HashSet<String>,
}

pub struct PermissionService {
    admin_uuids: Arc<DashSet<Uuid>>,
    player_commands: Arc<HashSet<String>>,
    subcommand_info: OnceLock<SubcommandInfo>,
}

impl PermissionService {
    pub fn new_sync(config: &PermissionsConfig) -> Self {
        Self::build(config, Arc::new(DashSet::new()))
    }

    pub async fn new(config: &PermissionsConfig) -> Self {
        let admin_uuids = Arc::new(DashSet::new());
        resolve_admins_into(&config.admins, &admin_uuids).await;

        if admin_uuids.is_empty() && config.admins.is_empty() {
            tracing::warn!(
                "No admins configured in [permissions]. \
                 All proxy commands will be inaccessible in-game. \
                 Use the console 'op <username>' command to add an admin."
            );
        }

        Self::build(config, admin_uuids)
    }

    fn build(config: &PermissionsConfig, admin_uuids: Arc<DashSet<Uuid>>) -> Self {
        let player_commands: HashSet<String> = config
            .player_commands
            .iter()
            .map(|c| c.to_lowercase())
            .collect();

        Self {
            admin_uuids,
            player_commands: Arc::new(player_commands),
            subcommand_info: OnceLock::new(),
        }
    }

    pub fn register_subcommands(&self, all: HashSet<String>, admin_only: HashSet<String>) {
        let _ = self.subcommand_info.set(SubcommandInfo { all, admin_only });

        if let Some(info) = self.subcommand_info.get() {
            for cmd in self.player_commands.iter() {
                if info.admin_only.contains(cmd) {
                    tracing::warn!(
                        "Command '{cmd}' is always admin-only and cannot be opened to players"
                    );
                }
            }
        }
    }

    pub fn build_checker(&self, player_uuid: Uuid) -> ConfigPermissionChecker {
        ConfigPermissionChecker {
            admin_uuids: Arc::clone(&self.admin_uuids),
            player_uuid,
            player_commands: Arc::clone(&self.player_commands),
        }
    }

    pub fn is_admin(&self, uuid: &Uuid) -> bool {
        self.admin_uuids.contains(uuid)
    }

    pub fn is_command_allowed(&self, command: &str, level: PermissionLevel) -> bool {
        if level >= PermissionLevel::Admin {
            return true;
        }
        let cmd_lower = command.to_lowercase();
        if let Some(info) = self.subcommand_info.get()
            && info.admin_only.contains(&cmd_lower)
        {
            return false;
        }
        self.player_commands.contains(&cmd_lower)
    }

    pub fn visible_subcommands(&self, level: PermissionLevel) -> HashSet<String> {
        let info = self.subcommand_info.get();
        if level >= PermissionLevel::Admin {
            return info.map(|i| i.all.clone()).unwrap_or_default();
        }
        let admin_only = info.map(|i| &i.admin_only);
        self.player_commands
            .iter()
            .filter(|cmd| admin_only.is_none_or(|ao| !ao.contains(cmd.as_str())))
            .cloned()
            .collect()
    }

    pub async fn reload(&self, config: &PermissionsConfig) {
        let new_set = DashSet::new();
        resolve_admins_into(&config.admins, &new_set).await;

        let new_uuids: HashSet<Uuid> = new_set.iter().map(|r| *r).collect();
        for uuid in &new_uuids {
            self.admin_uuids.insert(*uuid);
        }
        self.admin_uuids.retain(|uuid| new_uuids.contains(uuid));

        tracing::info!(count = self.admin_uuids.len(), "permissions reloaded");
    }

    pub fn add_admin(&self, uuid: Uuid) {
        self.admin_uuids.insert(uuid);
    }

    pub fn remove_admin(&self, uuid: &Uuid) -> bool {
        self.admin_uuids.remove(uuid).is_some()
    }

    pub fn admin_list(&self) -> Vec<Uuid> {
        self.admin_uuids.iter().map(|r| *r).collect()
    }
}

pub struct ConfigPermissionChecker {
    admin_uuids: Arc<DashSet<Uuid>>,
    player_uuid: Uuid,
    player_commands: Arc<HashSet<String>>,
}

impl PermissionChecker for ConfigPermissionChecker {
    fn permission_level(&self) -> PermissionLevel {
        if self.admin_uuids.contains(&self.player_uuid) {
            PermissionLevel::Admin
        } else {
            PermissionLevel::Player
        }
    }

    fn has_permission(&self, permission: &str) -> bool {
        let is_admin = self.admin_uuids.contains(&self.player_uuid);
        match permission {
            PERM_ADMIN => is_admin,
            p => match p.strip_prefix(PERM_COMMAND_PREFIX) {
                Some(cmd) => is_admin || self.player_commands.contains(cmd),
                None => is_admin,
            },
        }
    }
}

async fn resolve_admins_into(admins: &[String], target: &DashSet<Uuid>) {
    let client = reqwest::Client::new();

    for entry in admins {
        let trimmed = entry.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Ok(uuid) = Uuid::parse_str(trimmed) {
            target.insert(uuid);
            tracing::debug!(uuid = %uuid, "admin UUID added from config");
            continue;
        }

        match resolve_username(&client, trimmed).await {
            Ok(uuid) => {
                target.insert(uuid);
                tracing::info!(username = %trimmed, uuid = %uuid, "resolved admin username");
            }
            Err(e) => {
                tracing::warn!(
                    username = %trimmed,
                    error = %e,
                    "failed to resolve admin username — this admin will not be recognized"
                );
            }
        }
    }
}

async fn resolve_username(client: &reqwest::Client, username: &str) -> Result<Uuid, String> {
    #[derive(serde::Deserialize)]
    struct MojangProfile {
        id: String,
    }

    let url = format!("https://api.mojang.com/users/profiles/minecraft/{username}");

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {e}"))?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(format!("username '{username}' not found on Mojang"));
    }

    if !response.status().is_success() {
        return Err(format!("Mojang API returned status {}", response.status()));
    }

    let profile: MojangProfile = response
        .json()
        .await
        .map_err(|e| format!("failed to parse Mojang response: {e}"))?;

    let uuid_str = if profile.id.len() == 32 && !profile.id.contains('-') {
        format!(
            "{}-{}-{}-{}-{}",
            &profile.id[..8],
            &profile.id[8..12],
            &profile.id[12..16],
            &profile.id[16..20],
            &profile.id[20..]
        )
    } else {
        profile.id
    };

    Uuid::parse_str(&uuid_str).map_err(|e| format!("invalid UUID from Mojang: {e}"))
}

pub async fn resolve_username_to_uuid(username: &str) -> Result<Uuid, String> {
    let client = reqwest::Client::new();
    resolve_username(&client, username).await
}

pub fn default_checker() -> Arc<dyn PermissionChecker> {
    Arc::new(DefaultPermissionChecker)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_checker_admin() {
        let set = Arc::new(DashSet::new());
        let uuid = Uuid::new_v4();
        set.insert(uuid);

        let checker = ConfigPermissionChecker {
            admin_uuids: set,
            player_uuid: uuid,
            player_commands: Arc::new(HashSet::new()),
        };

        assert_eq!(checker.permission_level(), PermissionLevel::Admin);
        assert!(checker.has_permission(PERM_ADMIN));
        assert!(checker.has_permission("infrarust.command.kick"));
        assert!(checker.has_permission("anything"));
    }

    #[test]
    fn config_checker_player_no_commands() {
        let checker = ConfigPermissionChecker {
            admin_uuids: Arc::new(DashSet::new()),
            player_uuid: Uuid::new_v4(),
            player_commands: Arc::new(HashSet::new()),
        };

        assert_eq!(checker.permission_level(), PermissionLevel::Player);
        assert!(!checker.has_permission(PERM_ADMIN));
        assert!(!checker.has_permission("infrarust.command.list"));
    }

    #[test]
    fn config_checker_player_with_commands() {
        let checker = ConfigPermissionChecker {
            admin_uuids: Arc::new(DashSet::new()),
            player_uuid: Uuid::new_v4(),
            player_commands: Arc::new(HashSet::from(["list".into(), "help".into()])),
        };

        assert!(checker.has_permission("infrarust.command.list"));
        assert!(checker.has_permission("infrarust.command.help"));
        assert!(!checker.has_permission("infrarust.command.server"));
    }

    fn test_service(player_commands: &[&str]) -> PermissionService {
        let config = PermissionsConfig {
            admins: vec![],
            player_commands: player_commands.iter().map(|s| s.to_string()).collect(),
        };
        let svc = PermissionService::new_sync(&config);
        svc.register_subcommands(
            HashSet::from([
                "kick".into(),
                "list".into(),
                "help".into(),
                "server".into(),
                "reload".into(),
            ]),
            HashSet::from(["kick".into(), "reload".into()]),
        );
        svc
    }

    #[test]
    fn is_command_allowed_admin() {
        let svc = test_service(&[]);
        assert!(svc.is_command_allowed("kick", PermissionLevel::Admin));
        assert!(svc.is_command_allowed("list", PermissionLevel::Admin));
    }

    #[test]
    fn is_command_allowed_player() {
        let svc = test_service(&["list"]);
        assert!(svc.is_command_allowed("list", PermissionLevel::Player));
        assert!(!svc.is_command_allowed("kick", PermissionLevel::Player));
        assert!(!svc.is_command_allowed("server", PermissionLevel::Player));
    }

    #[test]
    fn visible_subcommands_admin_sees_all() {
        let svc = test_service(&[]);
        let visible = svc.visible_subcommands(PermissionLevel::Admin);
        assert!(visible.contains("kick"));
        assert!(visible.contains("list"));
        assert!(visible.contains("reload"));
    }

    #[test]
    fn visible_subcommands_player_sees_configured() {
        let svc = test_service(&["list", "help"]);
        let visible = svc.visible_subcommands(PermissionLevel::Player);
        assert!(visible.contains("list"));
        assert!(visible.contains("help"));
        assert!(!visible.contains("kick"));
    }

    #[test]
    fn add_remove_admin() {
        let svc = test_service(&[]);
        let uuid = Uuid::new_v4();
        assert!(!svc.is_admin(&uuid));

        svc.add_admin(uuid);
        assert!(svc.is_admin(&uuid));

        assert!(svc.remove_admin(&uuid));
        assert!(!svc.is_admin(&uuid));
    }

    #[test]
    fn live_dashset_update_reflected_in_checker() {
        let set = Arc::new(DashSet::new());
        let uuid = Uuid::new_v4();

        let checker = ConfigPermissionChecker {
            admin_uuids: Arc::clone(&set),
            player_uuid: uuid,
            player_commands: Arc::new(HashSet::new()),
        };

        assert_eq!(checker.permission_level(), PermissionLevel::Player);

        set.insert(uuid);
        assert_eq!(checker.permission_level(), PermissionLevel::Admin);

        set.remove(&uuid);
        assert_eq!(checker.permission_level(), PermissionLevel::Player);
    }
}

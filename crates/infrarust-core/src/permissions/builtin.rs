use std::collections::HashSet;
use std::sync::{Arc, PoisonError, RwLock};

use dashmap::DashSet;
use uuid::Uuid;

use infrarust_api::event::BoxFuture;
use infrarust_api::permissions::{
    AllPermissionsChecker, DefaultPermissionChecker, PermissionChecker, PermissionMap,
    PermissionProvider, PermissionSubject, Tristate,
};
use infrarust_config::PermissionsConfig;

use super::nodes::command_node;

struct Shared {
    admins: DashSet<Uuid>,
    grants: RwLock<Arc<PermissionMap>>,
    trust_offline_admins: bool,
}

pub struct ConfigPermissionProvider {
    shared: Arc<Shared>,
    player_commands: Vec<String>,
}

impl ConfigPermissionProvider {
    pub fn new(config: &PermissionsConfig) -> Self {
        let player_commands: Vec<String> = config
            .player_commands
            .iter()
            .map(|command| command.trim().to_lowercase())
            .filter(|command| !command.is_empty())
            .collect();
        let admins = DashSet::new();
        for entry in &config.admins {
            if let Ok(uuid) = Uuid::parse_str(entry.trim()) {
                admins.insert(uuid);
            }
        }
        let grants = player_grants(&player_commands, &HashSet::new());
        Self {
            shared: Arc::new(Shared {
                admins,
                grants: RwLock::new(Arc::new(grants)),
                trust_offline_admins: config.trust_offline_admins,
            }),
            player_commands,
        }
    }

    pub async fn resolve_admins(&self, admins: &[String]) {
        let client = reqwest::Client::new();
        for entry in admins {
            let trimmed = entry.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(uuid) = Uuid::parse_str(trimmed) {
                self.shared.admins.insert(uuid);
                tracing::debug!(uuid = %uuid, "admin UUID added from config");
                continue;
            }
            match resolve_username(&client, trimmed).await {
                Ok(uuid) => {
                    self.shared.admins.insert(uuid);
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

    pub fn checker_for(&self, subject: &PermissionSubject) -> Arc<dyn PermissionChecker> {
        match subject {
            PermissionSubject::Console => Arc::new(AllPermissionsChecker),
            PermissionSubject::Player {
                profile,
                online_mode,
                ..
            } => Arc::new(ConfigPermissionChecker {
                shared: Arc::clone(&self.shared),
                uuid: profile.uuid,
                admin_eligible: *online_mode || self.shared.trust_offline_admins,
            }),
            _ => Arc::new(DefaultPermissionChecker),
        }
    }

    pub(crate) fn player_commands(&self) -> &[String] {
        &self.player_commands
    }

    pub(crate) fn lock_subcommands(&self, admin_only: &HashSet<String>) {
        *self
            .shared
            .grants
            .write()
            .unwrap_or_else(PoisonError::into_inner) =
            Arc::new(player_grants(&self.player_commands, admin_only));
    }

    pub fn trusts_offline_admins(&self) -> bool {
        self.shared.trust_offline_admins
    }

    pub fn is_admin(&self, uuid: &Uuid) -> bool {
        self.shared.admins.contains(uuid)
    }

    pub fn add_admin(&self, uuid: Uuid) {
        self.shared.admins.insert(uuid);
    }

    pub fn remove_admin(&self, uuid: &Uuid) -> bool {
        self.shared.admins.remove(uuid).is_some()
    }

    pub fn admin_list(&self) -> Vec<Uuid> {
        self.shared.admins.iter().map(|r| *r).collect()
    }
}

impl PermissionProvider for ConfigPermissionProvider {
    fn create_checker<'a>(
        &'a self,
        subject: &'a PermissionSubject,
    ) -> BoxFuture<'a, Arc<dyn PermissionChecker>> {
        let checker = self.checker_for(subject);
        Box::pin(async move { checker })
    }
}

pub struct ConfigPermissionChecker {
    shared: Arc<Shared>,
    uuid: Uuid,
    admin_eligible: bool,
}

impl PermissionChecker for ConfigPermissionChecker {
    fn value(&self, node: &str) -> Tristate {
        if self.admin_eligible && self.shared.admins.contains(&self.uuid) {
            return Tristate::True;
        }
        let grants = Arc::clone(
            &self
                .shared
                .grants
                .read()
                .unwrap_or_else(PoisonError::into_inner),
        );
        grants.value(node)
    }
}

fn player_grants(player_commands: &[String], admin_only: &HashSet<String>) -> PermissionMap {
    let mut grants = PermissionMap::new();
    for command in player_commands {
        grants.set(&command_node(command), true);
    }
    for command in admin_only {
        let node = command_node(command);
        if grants.value(&node).is_true() {
            grants.set(&node, false);
        }
    }
    grants
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use infrarust_api::permissions::ADMIN_PERMISSION;
    use infrarust_api::types::{GameProfile, PlayerId};

    use super::*;

    fn config(admins: &[Uuid], player_commands: &[&str], trust: bool) -> PermissionsConfig {
        PermissionsConfig {
            admins: admins.iter().map(ToString::to_string).collect(),
            player_commands: player_commands.iter().map(|c| (*c).to_string()).collect(),
            trust_offline_admins: trust,
            ..PermissionsConfig::default()
        }
    }

    fn subject(uuid: Uuid, online_mode: bool) -> PermissionSubject {
        PermissionSubject::player(
            PlayerId::new(1),
            GameProfile {
                uuid,
                username: "Steve".into(),
                properties: vec![],
            },
            online_mode,
            "127.0.0.1:1".parse().unwrap(),
        )
    }

    #[test]
    fn an_online_admin_holds_every_node() {
        let steve = Uuid::new_v4();
        let provider = ConfigPermissionProvider::new(&config(&[steve], &[], false));
        let checker = provider.checker_for(&subject(steve, true));
        assert_eq!(checker.value(ADMIN_PERMISSION), Tristate::True);
        assert_eq!(checker.value("infrarust.command.kick"), Tristate::True);
        assert_eq!(checker.value("anything.at.all"), Tristate::True);
    }

    #[test]
    fn an_offline_admin_is_only_trusted_when_configured() {
        let steve = Uuid::new_v4();
        let wary = ConfigPermissionProvider::new(&config(&[steve], &[], false));
        assert_eq!(
            wary.checker_for(&subject(steve, false))
                .value(ADMIN_PERMISSION),
            Tristate::Undefined
        );
        let trusting = ConfigPermissionProvider::new(&config(&[steve], &[], true));
        assert!(trusting.trusts_offline_admins());
        assert_eq!(
            trusting
                .checker_for(&subject(steve, false))
                .value(ADMIN_PERMISSION),
            Tristate::True
        );
    }

    #[test]
    fn player_commands_grant_their_nodes_to_everyone() {
        let provider = ConfigPermissionProvider::new(&config(&[], &["List", "hello"], false));
        let checker = provider.checker_for(&subject(Uuid::new_v4(), false));
        assert_eq!(checker.value("infrarust.command.list"), Tristate::True);
        assert_eq!(checker.value("infrarust.command.LIST"), Tristate::True);
        assert_eq!(checker.value("infrarust.command.hello"), Tristate::True);
        assert_eq!(checker.value("infrarust.command.kick"), Tristate::Undefined);
        assert_eq!(checker.value(ADMIN_PERMISSION), Tristate::Undefined);
    }

    #[test]
    fn admin_only_subcommands_stay_closed_even_under_a_wildcard() {
        let provider = ConfigPermissionProvider::new(&config(&[], &["*", "kick"], false));
        let checker = provider.checker_for(&subject(Uuid::new_v4(), true));
        assert_eq!(checker.value("infrarust.command.kick"), Tristate::True);

        provider.lock_subcommands(&HashSet::from(["kick".to_string(), "reload".to_string()]));

        assert_eq!(checker.value("infrarust.command.kick"), Tristate::False);
        assert_eq!(checker.value("infrarust.command.reload"), Tristate::False);
        assert_eq!(checker.value("infrarust.command.list"), Tristate::True);
        assert_eq!(checker.value("infrarust.command.hello"), Tristate::True);
        assert_eq!(checker.value("infrarust.admin"), Tristate::Undefined);
    }

    #[test]
    fn op_and_deop_apply_to_live_checkers() {
        let provider = ConfigPermissionProvider::new(&config(&[], &[], false));
        let steve = Uuid::new_v4();
        let checker = provider.checker_for(&subject(steve, true));
        assert_eq!(checker.value(ADMIN_PERMISSION), Tristate::Undefined);

        provider.add_admin(steve);
        assert_eq!(checker.value(ADMIN_PERMISSION), Tristate::True);
        assert_eq!(provider.admin_list(), [steve]);

        assert!(provider.remove_admin(&steve));
        assert!(!provider.is_admin(&steve));
        assert_eq!(checker.value(ADMIN_PERMISSION), Tristate::Undefined);
    }

    #[test]
    fn the_console_holds_everything() {
        let provider = ConfigPermissionProvider::new(&PermissionsConfig::default());
        let console = provider.checker_for(&PermissionSubject::Console);
        assert_eq!(console.value("infrarust.command.kick"), Tristate::True);
        assert_eq!(console.value("demo.use"), Tristate::True);
    }
}

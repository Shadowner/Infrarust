mod builtin;
mod nodes;

use std::collections::HashSet;
use std::sync::{Arc, PoisonError, RwLock};

use uuid::Uuid;

use infrarust_api::command::CommandSource;
use infrarust_api::permissions::{
    ADMIN_PERMISSION, AllPermissionsChecker, DefaultPermissionChecker, PermissionChecker,
    PermissionDefault, PermissionNode, PermissionNodeError, PermissionNodeInfo, PermissionProvider,
    PermissionProviderRejected, PermissionSubject, Tristate, WILDCARD,
};
use infrarust_config::{PermissionProviderSelection, PermissionsConfig};

pub use builtin::{ConfigPermissionChecker, ConfigPermissionProvider, resolve_username_to_uuid};
pub use nodes::command_node;

use nodes::NodeRegistry;

pub struct PermissionService {
    selection: PermissionProviderSelection,
    builtin: Arc<ConfigPermissionProvider>,
    registered: RwLock<Option<Arc<dyn PermissionProvider>>>,
    nodes: Arc<NodeRegistry>,
    subcommands: RwLock<Vec<String>>,
}

impl PermissionService {
    pub fn new_sync(config: &PermissionsConfig) -> Self {
        Self::build(config, ConfigPermissionProvider::new(config))
    }

    pub async fn new(config: &PermissionsConfig) -> Self {
        let builtin = ConfigPermissionProvider::new(config);
        match &config.provider {
            PermissionProviderSelection::Builtin => {
                builtin.resolve_admins(&config.admins).await;
                if config.admins.is_empty() {
                    tracing::warn!(
                        "No admins configured in [permissions]. \
                         All proxy commands will be inaccessible in-game. \
                         Use the console 'op <username>' command to add an admin."
                    );
                }
            }
            PermissionProviderSelection::Plugin(id) => {
                tracing::info!(plugin = %id, "permissions are delegated to a plugin provider");
            }
        }
        Self::build(config, builtin)
    }

    fn build(config: &PermissionsConfig, builtin: ConfigPermissionProvider) -> Self {
        let nodes = NodeRegistry::default();
        if let Err(e) = nodes.register(
            None,
            PermissionNode::new(ADMIN_PERMISSION, PermissionDefault::False)
                .description("Every proxy command, and every node whose default is admin"),
        ) {
            tracing::error!("could not register {ADMIN_PERMISSION}: {e}");
        }
        Self {
            selection: config.provider.clone(),
            builtin: Arc::new(builtin),
            registered: RwLock::new(None),
            nodes: Arc::new(nodes),
            subcommands: RwLock::new(Vec::new()),
        }
    }

    pub const fn selection(&self) -> &PermissionProviderSelection {
        &self.selection
    }

    pub fn uses_builtin(&self) -> bool {
        self.selection == PermissionProviderSelection::Builtin
    }

    pub fn builtin(&self) -> &ConfigPermissionProvider {
        &self.builtin
    }

    pub fn register_provider(
        &self,
        plugin_id: &str,
        provider: Arc<dyn PermissionProvider>,
    ) -> Result<(), PermissionProviderRejected> {
        match &self.selection {
            PermissionProviderSelection::Plugin(id) if id == plugin_id => {
                *self
                    .registered
                    .write()
                    .unwrap_or_else(PoisonError::into_inner) = Some(provider);
                tracing::info!(plugin = %plugin_id, "permission provider registered, permissions now come from it");
                Ok(())
            }
            selected => {
                tracing::warn!(
                    plugin = %plugin_id,
                    selected = %selected,
                    "ignoring a permission provider: [permissions] provider selects another one"
                );
                Err(PermissionProviderRejected::NotSelected {
                    selected: selected.to_string(),
                })
            }
        }
    }

    pub fn unregister_provider(&self, plugin_id: &str) -> bool {
        if self.selection.plugin_id() != Some(plugin_id) {
            return false;
        }
        let removed = self
            .registered
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .is_some();
        if removed {
            tracing::error!(
                plugin = %plugin_id,
                "the permission provider plugin went away, players only get the node defaults until it registers again"
            );
        }
        removed
    }

    pub fn report_missing_provider(&self) {
        if let PermissionProviderSelection::Plugin(id) = &self.selection
            && self.active().is_none()
        {
            tracing::error!(
                plugin = %id,
                "[permissions] provider names a plugin that registered no permission provider; players only get the node defaults until it does (set provider = \"builtin\" to change this)"
            );
        }
    }

    fn active(&self) -> Option<Arc<dyn PermissionProvider>> {
        match &self.selection {
            PermissionProviderSelection::Builtin => {
                Some(Arc::clone(&self.builtin) as Arc<dyn PermissionProvider>)
            }
            PermissionProviderSelection::Plugin(_) => self
                .registered
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        }
    }

    pub async fn create_checker(&self, subject: &PermissionSubject) -> Arc<dyn PermissionChecker> {
        if let Some(provider) = self.active() {
            return provider.create_checker(subject).await;
        }
        if subject.is_console() {
            return Arc::new(AllPermissionsChecker);
        }
        tracing::debug!(
            provider = %self.selection,
            player = subject.profile().map_or("-", |profile| profile.username.as_str()),
            "no permission provider is registered, the player only gets the node defaults"
        );
        Arc::new(DefaultPermissionChecker)
    }

    pub async fn console_checker(&self) -> Arc<dyn PermissionChecker> {
        let checker = self.create_checker(&PermissionSubject::Console).await;
        self.resolved(checker)
    }

    pub fn resolved(&self, checker: Arc<dyn PermissionChecker>) -> Arc<dyn PermissionChecker> {
        Arc::new(Resolved {
            checker,
            nodes: Arc::clone(&self.nodes),
        })
    }

    pub fn value(&self, checker: &dyn PermissionChecker, node: &str) -> Tristate {
        self.nodes.resolve(checker, node)
    }

    pub fn register_node(
        &self,
        owner: Option<&str>,
        node: PermissionNode,
    ) -> Result<(), PermissionNodeError> {
        self.nodes.register(owner, node)
    }

    pub fn unregister_nodes(&self, owner: &str) -> usize {
        self.nodes.unregister_owned(owner)
    }

    pub fn nodes(&self) -> Vec<PermissionNodeInfo> {
        self.nodes.list()
    }

    pub fn node_default(&self, node: &str) -> Option<PermissionDefault> {
        self.nodes.default_of(node)
    }

    pub fn register_subcommands<'a>(
        &self,
        subcommands: impl IntoIterator<Item = (&'a str, &'a str, bool)>,
    ) {
        let opened: HashSet<&str> = self
            .builtin
            .player_commands()
            .iter()
            .map(String::as_str)
            .collect();
        let mut names = Vec::new();
        let mut admin_only = HashSet::new();
        for (name, description, locked) in subcommands {
            let name = name.to_lowercase();
            if locked && opened.contains(name.as_str()) {
                tracing::warn!(
                    "Command '{name}' is always admin-only and cannot be opened to players"
                );
            }
            let open = !locked && (opened.contains(name.as_str()) || opened.contains(WILDCARD));
            let default = if open {
                PermissionDefault::True
            } else {
                PermissionDefault::Admin
            };
            let node = PermissionNode::new(command_node(&name), default).description(description);
            if let Err(e) = self.nodes.register(None, node) {
                tracing::error!("could not register the node of /ir {name}: {e}");
            }
            if locked {
                admin_only.insert(name.clone());
            }
            names.push(name);
        }
        self.builtin.lock_subcommands(&admin_only);
        names.sort();
        *self
            .subcommands
            .write()
            .unwrap_or_else(PoisonError::into_inner) = names;
    }

    pub fn is_command_allowed(&self, command: &str, source: &CommandSource) -> bool {
        source.has_permission(&command_node(command))
    }

    pub fn visible_subcommands(&self, source: &CommandSource) -> HashSet<String> {
        let names = self
            .subcommands
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        names
            .into_iter()
            .filter(|name| self.is_command_allowed(name, source))
            .collect()
    }

    pub fn is_admin(&self, uuid: &Uuid) -> bool {
        self.builtin.is_admin(uuid)
    }

    pub fn add_admin(&self, uuid: Uuid) {
        self.builtin.add_admin(uuid);
    }

    pub fn remove_admin(&self, uuid: &Uuid) -> bool {
        self.builtin.remove_admin(uuid)
    }

    pub fn admin_list(&self) -> Vec<Uuid> {
        self.builtin.admin_list()
    }
}

struct Resolved {
    checker: Arc<dyn PermissionChecker>,
    nodes: Arc<NodeRegistry>,
}

impl PermissionChecker for Resolved {
    fn value(&self, node: &str) -> Tristate {
        self.nodes.resolve(self.checker.as_ref(), node)
    }
}

pub fn default_checker() -> Arc<dyn PermissionChecker> {
    Arc::new(DefaultPermissionChecker)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use infrarust_api::event::BoxFuture;
    use infrarust_api::permissions::PermissionMap;
    use infrarust_api::types::{GameProfile, PlayerId};

    use super::*;

    const SUBCOMMANDS: [(&str, &str, bool); 5] = [
        ("kick", "Kick", true),
        ("list", "List", false),
        ("help", "Help", false),
        ("server", "Server", false),
        ("reload", "Reload", true),
    ];

    fn service(config: &PermissionsConfig) -> PermissionService {
        let service = PermissionService::new_sync(config);
        service.register_subcommands(SUBCOMMANDS);
        service
    }

    fn player_commands(commands: &[&str]) -> PermissionsConfig {
        PermissionsConfig {
            player_commands: commands.iter().map(|c| (*c).to_string()).collect(),
            ..PermissionsConfig::default()
        }
    }

    fn plugin_selected(id: &str) -> PermissionsConfig {
        PermissionsConfig {
            provider: PermissionProviderSelection::Plugin(id.into()),
            ..PermissionsConfig::default()
        }
    }

    fn steve(online_mode: bool) -> PermissionSubject {
        PermissionSubject::player(
            PlayerId::new(1),
            GameProfile {
                uuid: Uuid::from_u128(1),
                username: "Steve".into(),
                properties: vec![],
            },
            online_mode,
            "127.0.0.1:1".parse().unwrap(),
        )
    }

    fn console(checker: Arc<dyn PermissionChecker>) -> CommandSource {
        CommandSource::console(checker)
    }

    struct Fixed(PermissionMap);

    impl PermissionProvider for Fixed {
        fn create_checker<'a>(
            &'a self,
            _subject: &'a PermissionSubject,
        ) -> BoxFuture<'a, Arc<dyn PermissionChecker>> {
            let checker: Arc<dyn PermissionChecker> = Arc::new(self.0.clone());
            Box::pin(async move { checker })
        }
    }

    #[test]
    fn builtin_subcommands_get_nodes_with_todays_defaults() {
        let svc = service(&player_commands(&["list", "kick"]));
        assert_eq!(
            svc.node_default("infrarust.command.list"),
            Some(PermissionDefault::True)
        );
        assert_eq!(
            svc.node_default("infrarust.command.help"),
            Some(PermissionDefault::Admin)
        );
        assert_eq!(
            svc.node_default("infrarust.command.kick"),
            Some(PermissionDefault::Admin)
        );
        assert_eq!(
            svc.node_default(ADMIN_PERMISSION),
            Some(PermissionDefault::False)
        );
    }

    #[test]
    fn a_wildcard_opens_every_subcommand_players_may_have() {
        let svc = service(&player_commands(&["*"]));
        for open in ["list", "help", "server"] {
            assert_eq!(
                svc.node_default(&command_node(open)),
                Some(PermissionDefault::True),
                "{open}"
            );
        }
        let player = console(svc.resolved(Arc::new(DefaultPermissionChecker)));
        let mut visible: Vec<String> = svc.visible_subcommands(&player).into_iter().collect();
        visible.sort();
        assert_eq!(visible, ["help", "list", "server"]);
    }

    #[test]
    fn subcommands_follow_the_sources_nodes() {
        let svc = service(&player_commands(&["list"]));
        let player = console(svc.resolved(Arc::new(DefaultPermissionChecker)));
        assert!(svc.is_command_allowed("list", &player));
        assert!(svc.is_command_allowed("LIST", &player));
        assert!(!svc.is_command_allowed("kick", &player));
        assert!(!svc.is_command_allowed("server", &player));
        assert!(!svc.is_command_allowed("unknown", &player));

        let admin =
            console(svc.resolved(Arc::new(PermissionMap::new().with(ADMIN_PERMISSION, true))));
        let mut visible: Vec<String> = svc.visible_subcommands(&admin).into_iter().collect();
        visible.sort();
        assert_eq!(visible, ["help", "kick", "list", "reload", "server"]);

        let moderator = console(
            svc.resolved(Arc::new(
                PermissionMap::new()
                    .with("infrarust.command.kick", true)
                    .with("infrarust.command.list", false),
            )),
        );
        assert!(svc.is_command_allowed("kick", &moderator));
        assert!(!svc.is_command_allowed("list", &moderator));
    }

    #[tokio::test]
    async fn the_builtin_provider_keeps_todays_admins() {
        let admin = Uuid::from_u128(1);
        let svc = service(&PermissionsConfig {
            admins: vec![admin.to_string()],
            ..player_commands(&["list"])
        });
        assert!(svc.is_admin(&admin));

        let online = svc.create_checker(&steve(true)).await;
        assert_eq!(
            svc.value(online.as_ref(), "infrarust.command.kick"),
            Tristate::True
        );
        let offline = svc.create_checker(&steve(false)).await;
        assert_eq!(
            svc.value(offline.as_ref(), "infrarust.command.kick"),
            Tristate::False
        );
        assert_eq!(
            svc.value(offline.as_ref(), "infrarust.command.list"),
            Tristate::True
        );
        assert_eq!(
            svc.value(offline.as_ref(), ADMIN_PERMISSION),
            Tristate::False
        );
    }

    #[tokio::test]
    async fn trust_offline_admins_lets_offline_admins_in() {
        let admin = Uuid::from_u128(1);
        let svc = service(&PermissionsConfig {
            admins: vec![admin.to_string()],
            trust_offline_admins: true,
            ..PermissionsConfig::default()
        });
        let offline = svc.create_checker(&steve(false)).await;
        assert_eq!(
            svc.value(offline.as_ref(), "infrarust.command.reload"),
            Tristate::True
        );
    }

    #[tokio::test]
    async fn only_the_selected_plugin_may_provide() {
        let svc = service(&plugin_selected("perms"));
        let provider: Arc<dyn PermissionProvider> =
            Arc::new(Fixed(PermissionMap::new().with("demo.use", true)));

        assert_eq!(
            svc.register_provider("other", Arc::clone(&provider)),
            Err(PermissionProviderRejected::NotSelected {
                selected: "perms".into()
            })
        );
        let builtin = service(&PermissionsConfig::default());
        assert_eq!(
            builtin.register_provider("perms", Arc::clone(&provider)),
            Err(PermissionProviderRejected::NotSelected {
                selected: "builtin".into()
            })
        );

        svc.register_provider("perms", provider).unwrap();
        let checker = svc.create_checker(&steve(true)).await;
        assert_eq!(svc.value(checker.as_ref(), "demo.use"), Tristate::True);

        assert!(!svc.unregister_provider("other"));
        assert!(svc.unregister_provider("perms"));
        let orphan = svc.create_checker(&steve(true)).await;
        assert_eq!(svc.value(orphan.as_ref(), "demo.use"), Tristate::Undefined);
    }

    #[tokio::test]
    async fn a_missing_provider_leaves_players_with_the_node_defaults() {
        let svc = service(&PermissionsConfig {
            admins: vec![Uuid::from_u128(1).to_string()],
            ..plugin_selected("perms")
        });
        svc.register_node(
            Some("demo"),
            PermissionNode::new("demo.open", PermissionDefault::True),
        )
        .unwrap();
        let checker = svc.create_checker(&steve(true)).await;
        assert_eq!(svc.value(checker.as_ref(), "demo.open"), Tristate::True);
        assert_eq!(
            svc.value(checker.as_ref(), "infrarust.command.kick"),
            Tristate::False
        );
        assert_eq!(
            svc.value(checker.as_ref(), ADMIN_PERMISSION),
            Tristate::False
        );

        let console = svc.console_checker().await;
        assert!(console.has_permission("infrarust.command.kick"));
    }

    #[tokio::test]
    async fn the_console_checker_applies_node_defaults() {
        let svc = service(&plugin_selected("perms"));
        svc.register_node(
            Some("demo"),
            PermissionNode::new("demo.open", PermissionDefault::True),
        )
        .unwrap();
        svc.register_provider(
            "perms",
            Arc::new(Fixed(PermissionMap::new().with("demo.use", false))),
        )
        .unwrap();

        let console = svc.console_checker().await;
        assert!(!console.has_permission("demo.use"));
        assert!(console.has_permission("demo.open"));
        assert!(!console.has_permission("infrarust.command.kick"));

        let builtin = service(&PermissionsConfig::default())
            .console_checker()
            .await;
        assert!(builtin.has_permission("demo.use"));
        assert!(builtin.has_permission("infrarust.command.kick"));
    }
}

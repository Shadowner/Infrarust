use std::collections::{HashMap, HashSet};
use std::sync::{Arc, PoisonError, RwLock};

use tokio::sync::watch;

use infrarust_api::command::{
    CommandContext, CommandError, CommandHandler, CommandInfo, CommandRegistration, CommandSource,
    CommandSpec, SuggestContext, Suggestion, split_label,
};
use infrarust_api::event::BoxFuture;
use infrarust_api::message::ProxyMessage;

pub const COMMAND_DENIED: &str = "You don't have permission to use this command.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchOutcome {
    Executed,
    Denied,
    Unknown,
}

pub enum Prepared<T> {
    Unknown,
    Denied,
    Ready(T),
}

pub struct Invocation {
    handler: Arc<dyn CommandHandler>,
    owner: Option<String>,
    ctx: CommandContext,
}

impl Invocation {
    pub fn owner(&self) -> Option<&str> {
        self.owner.as_deref()
    }

    pub fn label(&self) -> &str {
        &self.ctx.label
    }

    pub fn run(self) -> BoxFuture<'static, ()> {
        let Self { handler, ctx, .. } = self;
        Box::pin(async move { handler.execute(ctx).await })
    }
}

pub struct Completion {
    handler: Arc<dyn CommandHandler>,
    owner: Option<String>,
    ctx: SuggestContext,
}

impl Completion {
    pub fn owner(&self) -> Option<&str> {
        self.owner.as_deref()
    }

    pub fn label(&self) -> &str {
        &self.ctx.label
    }

    pub fn run(self) -> BoxFuture<'static, Vec<Suggestion>> {
        let Self { handler, ctx, .. } = self;
        Box::pin(async move { handler.suggest(ctx).await })
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProxyTree {
    pub commands: Vec<Vec<String>>,
    pub shadowed: HashSet<String>,
}

#[derive(Clone)]
struct Entry {
    handler: Arc<dyn CommandHandler>,
    spec: CommandSpec,
    owner: Option<String>,
}

impl Entry {
    fn namespaced(&self) -> Option<String> {
        self.owner
            .as_deref()
            .map(|owner| namespaced(owner, &self.spec.name))
    }

    fn labels(&self) -> Vec<String> {
        let mut labels = vec![self.spec.name.clone()];
        labels.extend(self.spec.aliases.iter().cloned());
        labels.extend(self.namespaced());
        labels
    }

    fn info(&self) -> CommandInfo {
        CommandInfo::new(self.spec.clone(), self.owner.clone())
    }
}

#[derive(Default)]
struct Table {
    commands: HashMap<String, Entry>,
    labels: HashMap<String, String>,
}

impl Table {
    fn resolve(&self, label: &str) -> Option<&Entry> {
        let key = self.labels.get(&label.to_lowercase())?;
        self.commands.get(key)
    }

    fn remove(&mut self, key: &str) -> Option<Entry> {
        let entry = self.commands.remove(key)?;
        self.labels.retain(|_, target| target != key);
        Some(entry)
    }

    fn claim(&self, label: &str, key: &str) -> Result<(), CommandError> {
        match self.labels.get(label) {
            None => Ok(()),
            Some(existing) if existing == key => Ok(()),
            Some(existing) => match self
                .commands
                .get(existing)
                .and_then(|entry| entry.owner.clone())
            {
                None => Err(CommandError::Reserved(label.to_string())),
                Some(plugin) => Err(CommandError::OwnedBy {
                    name: label.to_string(),
                    plugin,
                }),
            },
        }
    }
}

pub struct CommandManagerImpl {
    table: RwLock<Table>,
    generation: watch::Sender<u64>,
}

impl CommandManagerImpl {
    pub fn new() -> Self {
        Self {
            table: RwLock::new(Table::default()),
            generation: watch::Sender::new(0),
        }
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.generation.subscribe()
    }

    pub fn generation(&self) -> u64 {
        *self.generation.borrow()
    }

    fn bump(&self) {
        self.generation.send_modify(|generation| *generation += 1);
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Table> {
        self.table.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Table> {
        self.table.write().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn register_builtin(&self, spec: CommandSpec, handler: Box<dyn CommandHandler>) {
        let Some(name) = normalize(&spec.name) else {
            tracing::error!(name = %spec.name, "invalid built-in command name");
            return;
        };
        let aliases: Vec<String> = spec.aliases.iter().filter_map(|a| normalize(a)).collect();
        {
            let mut table = self.write();
            table.remove(&name);
            for label in std::iter::once(&name).chain(&aliases) {
                if let Some(previous) = table.labels.insert(label.clone(), name.clone())
                    && previous != name
                {
                    tracing::warn!(label = %label, displaced = %previous, "built-in command label replaced another command");
                }
            }
            let mut spec = spec;
            spec.aliases = aliases;
            spec.name.clone_from(&name);
            table.commands.insert(
                name,
                Entry {
                    handler: Arc::from(handler),
                    spec,
                    owner: None,
                },
            );
        }
        self.bump();
    }

    pub fn register_owned(
        &self,
        owner: &str,
        spec: CommandSpec,
        handler: Box<dyn CommandHandler>,
    ) -> Result<CommandRegistration, CommandError> {
        let name =
            normalize(&spec.name).ok_or_else(|| CommandError::InvalidName(spec.name.clone()))?;
        let key = namespaced(owner, &name);
        let registration = {
            let mut table = self.write();
            table.claim(&name, &key)?;
            table.remove(&key);

            let mut aliases = Vec::new();
            let mut rejected = Vec::new();
            for requested in &spec.aliases {
                match normalize(requested) {
                    Some(alias) if alias == name || aliases.contains(&alias) => {}
                    Some(alias) if table.labels.contains_key(&alias) => rejected.push(alias),
                    Some(alias) => aliases.push(alias),
                    None => rejected.push(requested.clone()),
                }
            }

            for label in [&name, &key].into_iter().chain(&aliases) {
                table.labels.insert(label.clone(), key.clone());
            }
            let mut spec = spec;
            spec.name.clone_from(&name);
            spec.aliases.clone_from(&aliases);
            table.commands.insert(
                key.clone(),
                Entry {
                    handler: Arc::from(handler),
                    spec,
                    owner: Some(owner.to_string()),
                },
            );
            CommandRegistration::new(name, key, aliases, rejected)
        };
        self.bump();
        Ok(registration)
    }

    pub fn unregister_owned(&self, owner: &str, name: &str) -> Result<String, CommandError> {
        let removed = {
            let mut table = self.write();
            let key = match table.resolve(name) {
                Some(entry) if entry.owner.as_deref() == Some(owner) => entry.namespaced(),
                _ => None,
            };
            key.filter(|key| table.remove(key).is_some())
        };
        let key = removed.ok_or_else(|| CommandError::NotOwned(name.to_string()))?;
        self.bump();
        Ok(key)
    }

    pub fn list(&self) -> Vec<CommandInfo> {
        let mut infos: Vec<CommandInfo> = self.read().commands.values().map(Entry::info).collect();
        infos.sort_by(|a, b| a.name.cmp(&b.name).then(a.plugin_id.cmp(&b.plugin_id)));
        infos
    }

    pub fn commands_for_plugin(&self, plugin_id: &str) -> Vec<CommandInfo> {
        self.list()
            .into_iter()
            .filter(|info| info.plugin_id.as_deref() == Some(plugin_id))
            .collect()
    }

    pub fn contains(&self, label: &str) -> bool {
        self.read().resolve(label).is_some()
    }

    pub fn is_plugin_command(&self, label: &str) -> bool {
        self.read()
            .resolve(label)
            .is_some_and(|entry| entry.owner.is_some())
    }

    pub fn tree_for(&self, source: Option<&CommandSource>) -> ProxyTree {
        let table = self.read();
        let mut entries: Vec<&Entry> = table
            .commands
            .values()
            .filter(|entry| {
                entry.owner.is_some() && source.is_some_and(|source| visible_to(entry, source))
            })
            .collect();
        entries.sort_by_key(|entry| entry.namespaced());
        ProxyTree {
            commands: entries.into_iter().map(Entry::labels).collect(),
            shadowed: table.labels.keys().cloned().collect(),
        }
    }

    fn resolve(&self, label: &str) -> Option<Entry> {
        self.read().resolve(label).cloned()
    }

    pub fn prepare(&self, source: CommandSource, input: &str) -> Prepared<Invocation> {
        let input = input.trim();
        let (label, rest) = split_label(input);
        if label.is_empty() {
            return Prepared::Unknown;
        }
        let Some(entry) = self.resolve(label) else {
            return Prepared::Unknown;
        };
        if !permitted(&entry, &source) {
            source.send_message(ProxyMessage::error(COMMAND_DENIED));
            return Prepared::Denied;
        }
        let mut ctx = CommandContext::new(source, label, rest.trim_start());
        input.clone_into(&mut ctx.raw);
        Prepared::Ready(Invocation {
            handler: entry.handler,
            owner: entry.owner,
            ctx,
        })
    }

    pub async fn dispatch(&self, source: CommandSource, input: &str) -> DispatchOutcome {
        match self.prepare(source, input) {
            Prepared::Unknown => DispatchOutcome::Unknown,
            Prepared::Denied => DispatchOutcome::Denied,
            Prepared::Ready(invocation) => {
                invocation.run().await;
                DispatchOutcome::Executed
            }
        }
    }

    pub fn prepare_suggestion(&self, source: CommandSource, input: &str) -> Prepared<Completion> {
        let input = input.trim_start();
        let Some((label, rest)) = input.split_once(char::is_whitespace) else {
            return Prepared::Unknown;
        };
        let Some(entry) = self.resolve(label) else {
            return Prepared::Unknown;
        };
        if !permitted(&entry, &source) {
            return Prepared::Denied;
        }
        Prepared::Ready(Completion {
            handler: entry.handler,
            owner: entry.owner,
            ctx: SuggestContext::new(source, label, rest),
        })
    }

    pub async fn suggest(&self, source: CommandSource, input: &str) -> Option<Vec<Suggestion>> {
        match self.prepare_suggestion(source, input) {
            Prepared::Unknown => None,
            Prepared::Denied => Some(Vec::new()),
            Prepared::Ready(completion) => Some(completion.run().await),
        }
    }
}

impl Default for CommandManagerImpl {
    fn default() -> Self {
        Self::new()
    }
}

fn permitted(entry: &Entry, source: &CommandSource) -> bool {
    entry
        .spec
        .permission
        .as_deref()
        .is_none_or(|node| source.has_permission(node))
}

fn visible_to(entry: &Entry, source: &CommandSource) -> bool {
    !entry.spec.hidden && permitted(entry, source)
}

fn namespaced(owner: &str, name: &str) -> String {
    format!("{}:{name}", owner.to_lowercase())
}

fn normalize(name: &str) -> Option<String> {
    let name = name.trim();
    let valid = !name.is_empty()
        && !name.starts_with('/')
        && !name
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == ':');
    valid.then(|| name.to_lowercase())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::Mutex;

    use infrarust_api::event::BoxFuture;
    use infrarust_api::permissions::{AllPermissionsChecker, PermissionMap};

    use super::*;

    fn console() -> CommandSource {
        CommandSource::console(Arc::new(AllPermissionsChecker))
    }

    type Calls = Arc<Mutex<Vec<String>>>;

    struct Recording {
        tag: &'static str,
        calls: Calls,
    }

    impl CommandHandler for Recording {
        fn execute<'a>(&'a self, ctx: CommandContext) -> BoxFuture<'a, ()> {
            self.calls.lock().unwrap().push(format!(
                "{}:{}:{}",
                self.tag,
                ctx.label,
                ctx.args.join(",")
            ));
            Box::pin(async {})
        }

        fn suggest<'a>(&'a self, ctx: SuggestContext) -> BoxFuture<'a, Vec<Suggestion>> {
            Box::pin(
                async move { vec![Suggestion::new(format!("{}-{}", self.tag, ctx.partial()))] },
            )
        }
    }

    fn handler(tag: &'static str, calls: &Calls) -> Box<dyn CommandHandler> {
        Box::new(Recording {
            tag,
            calls: Arc::clone(calls),
        })
    }

    fn manager(calls: &Calls) -> CommandManagerImpl {
        let manager = CommandManagerImpl::new();
        manager.register_builtin(
            CommandSpec::new("infrarust").alias("ir"),
            handler("builtin", calls),
        );
        manager
    }

    fn taken(calls: &Calls) -> Vec<String> {
        std::mem::take(&mut *calls.lock().unwrap())
    }

    #[tokio::test]
    async fn a_second_plugin_cannot_shadow_or_unregister_the_first() {
        let calls = Calls::default();
        let m = manager(&calls);
        m.register_owned("a", CommandSpec::new("hello"), handler("a", &calls))
            .unwrap();

        assert_eq!(
            m.register_owned("b", CommandSpec::new("Hello"), handler("b", &calls)),
            Err(CommandError::OwnedBy {
                name: "hello".into(),
                plugin: "a".into()
            })
        );
        assert_eq!(
            m.unregister_owned("b", "hello"),
            Err(CommandError::NotOwned("hello".into()))
        );
        assert_eq!(
            m.unregister_owned("b", "a:hello"),
            Err(CommandError::NotOwned("a:hello".into()))
        );

        assert_eq!(
            m.dispatch(console(), "hello x").await,
            DispatchOutcome::Executed
        );
        assert_eq!(taken(&calls), ["a:hello:x"]);
    }

    #[tokio::test]
    async fn builtin_names_and_aliases_are_reserved() {
        let calls = Calls::default();
        let m = manager(&calls);

        assert_eq!(
            m.register_owned("p", CommandSpec::new("IR"), handler("p", &calls)),
            Err(CommandError::Reserved("ir".into()))
        );
        let registration = m
            .register_owned(
                "p",
                CommandSpec::new("tool").aliases(["ir", "infrarust", "t", "bad name"]),
                handler("p", &calls),
            )
            .unwrap();
        assert_eq!(registration.aliases, ["t"]);
        assert_eq!(
            registration.rejected_aliases,
            ["ir", "infrarust", "bad name"]
        );

        m.dispatch(console(), "ir").await;
        m.dispatch(console(), "t").await;
        assert_eq!(taken(&calls), ["builtin:ir:", "p:t:"]);
    }

    #[tokio::test]
    async fn an_alias_clashing_with_another_plugin_is_skipped_not_stolen() {
        let calls = Calls::default();
        let m = manager(&calls);
        m.register_owned(
            "a",
            CommandSpec::new("home").alias("h"),
            handler("a", &calls),
        )
        .unwrap();
        let registration = m
            .register_owned(
                "b",
                CommandSpec::new("help2").alias("h"),
                handler("b", &calls),
            )
            .unwrap();

        assert_eq!(registration.rejected_aliases, ["h"]);
        m.dispatch(console(), "h").await;
        assert_eq!(taken(&calls), ["a:h:"]);
    }

    #[tokio::test]
    async fn the_namespaced_name_always_reaches_the_owner() {
        let calls = Calls::default();
        let m = manager(&calls);
        m.register_owned("A", CommandSpec::new("cmd"), handler("a", &calls))
            .unwrap();
        assert!(
            m.register_owned("b", CommandSpec::new("cmd"), handler("b", &calls))
                .is_err()
        );

        assert_eq!(
            m.dispatch(console(), "a:cmd 1 2").await,
            DispatchOutcome::Executed
        );
        assert_eq!(
            m.dispatch(console(), "b:cmd").await,
            DispatchOutcome::Unknown
        );
        assert_eq!(taken(&calls), ["a:a:cmd:1,2"]);
    }

    #[tokio::test]
    async fn aliases_resolve_everywhere() {
        let calls = Calls::default();
        let m = manager(&calls);
        m.register_owned(
            "p",
            CommandSpec::new("hello").alias("hi"),
            handler("p", &calls),
        )
        .unwrap();

        assert!(m.is_plugin_command("HI"));
        assert!(m.is_plugin_command("p:hello"));
        assert!(!m.is_plugin_command("ir"));
        assert!(m.contains("ir"));
        assert_eq!(
            m.suggest(console(), "hi wo").await,
            Some(vec![Suggestion::new("p-wo")])
        );
        assert_eq!(m.suggest(console(), "nope x").await, None);
        assert_eq!(m.suggest(console(), "hi").await, None);
    }

    #[tokio::test]
    async fn unregistering_frees_every_label_and_bumps_the_generation() {
        let calls = Calls::default();
        let m = manager(&calls);
        let before = m.generation();
        m.register_owned(
            "p",
            CommandSpec::new("hello").alias("hi"),
            handler("p", &calls),
        )
        .unwrap();
        assert!(m.generation() > before);

        let registered = m.generation();
        m.unregister_owned("p", "hi").unwrap();
        assert!(m.generation() > registered);
        assert!(!m.contains("hello"));
        assert!(!m.contains("hi"));
        assert!(!m.contains("p:hello"));
        m.register_owned("q", CommandSpec::new("hi"), handler("q", &calls))
            .unwrap();
    }

    #[tokio::test]
    async fn the_same_plugin_can_replace_its_own_command() {
        let calls = Calls::default();
        let m = manager(&calls);
        m.register_owned(
            "p",
            CommandSpec::new("hello").alias("hi"),
            handler("old", &calls),
        )
        .unwrap();
        m.register_owned("p", CommandSpec::new("hello"), handler("new", &calls))
            .unwrap();

        m.dispatch(console(), "hello").await;
        assert_eq!(taken(&calls), ["new:hello:"]);
        assert!(!m.contains("hi"));
    }

    #[test]
    fn invalid_names_are_refused() {
        let calls = Calls::default();
        let m = manager(&calls);
        for bad in ["", "  ", "two words", "a:b", "/slash"] {
            assert_eq!(
                m.register_owned("p", CommandSpec::new(bad), handler("p", &calls)),
                Err(CommandError::InvalidName(bad.into())),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn the_tree_hides_hidden_and_forbidden_commands_but_shadows_all_labels() {
        let calls = Calls::default();
        let m = manager(&calls);
        m.register_owned(
            "p",
            CommandSpec::new("open").alias("o"),
            handler("p", &calls),
        )
        .unwrap();
        m.register_owned(
            "p",
            CommandSpec::new("secret").hidden(true),
            handler("p", &calls),
        )
        .unwrap();
        m.register_owned(
            "p",
            CommandSpec::new("admin").permission("p.admin"),
            handler("p", &calls),
        )
        .unwrap();

        let console = m.tree_for(Some(&console()));
        assert!(m.tree_for(None).commands.is_empty());
        assert_eq!(
            console.commands,
            [
                vec!["admin".to_string(), "p:admin".into()],
                vec!["open".to_string(), "o".into(), "p:open".into()],
            ]
        );
        for label in ["infrarust", "ir", "secret", "p:secret", "admin", "o"] {
            assert!(console.shadowed.contains(label), "{label}");
        }
    }

    #[tokio::test]
    async fn the_console_is_held_to_its_checker() {
        let calls = Calls::default();
        let m = manager(&calls);
        m.register_owned(
            "p",
            CommandSpec::new("admin").permission("p.admin"),
            handler("p", &calls),
        )
        .unwrap();
        let restricted =
            CommandSource::console(Arc::new(PermissionMap::new().with("p.admin", false)));

        assert_eq!(
            m.dispatch(restricted.clone(), "admin").await,
            DispatchOutcome::Denied
        );
        assert_eq!(m.suggest(restricted, "admin x").await, Some(Vec::new()));
        assert_eq!(
            m.dispatch(console(), "admin").await,
            DispatchOutcome::Executed
        );
        assert_eq!(taken(&calls), ["p:admin:"]);
    }
}

#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use infrarust_api::command::{
    CommandContext, CommandError, CommandHandler, CommandInfo, CommandSource, CommandSpec,
};
use infrarust_api::event::BoxFuture;
use infrarust_api::permissions::AllPermissionsChecker;
use infrarust_api::plugin::PluginContext;
use infrarust_core::plugin::context::PluginContextImpl;
use infrarust_core::plugin::context_factory::PluginContextFactoryImpl;
use infrarust_core::plugin::manager::PluginServices;
use infrarust_core::services::command_manager::{CommandManagerImpl, DispatchOutcome};

type Calls = Arc<Mutex<Vec<String>>>;

struct Recording {
    tag: &'static str,
    calls: Calls,
}

impl CommandHandler for Recording {
    fn execute<'a>(&'a self, _ctx: CommandContext) -> BoxFuture<'a, ()> {
        self.calls.lock().unwrap().push(self.tag.to_string());
        Box::pin(async {})
    }
}

fn handler(tag: &'static str, calls: &Calls) -> Box<dyn CommandHandler> {
    Box::new(Recording {
        tag,
        calls: Arc::clone(calls),
    })
}

fn factory(commands: &Arc<CommandManagerImpl>) -> PluginContextFactoryImpl {
    let services = PluginServices {
        command_manager: Arc::clone(commands),
        ..PluginServices::for_tests()
    };
    PluginContextFactoryImpl::new(services, HashMap::new())
}

fn disable(ctx: &Arc<PluginContextImpl>) {
    ctx.cleanup();
}

fn tracked(ctx: &Arc<PluginContextImpl>) -> Vec<String> {
    ctx.tracked_commands()
}

async fn run(commands: &CommandManagerImpl, input: &str) -> bool {
    let console = CommandSource::console(Arc::new(AllPermissionsChecker));
    commands.dispatch(console, input).await == DispatchOutcome::Executed
}

fn with_builtin(calls: &Calls) -> Arc<CommandManagerImpl> {
    let commands = Arc::new(CommandManagerImpl::new());
    commands.register_builtin(
        CommandSpec::new("infrarust").alias("ir"),
        handler("builtin", calls),
    );
    commands
}

#[tokio::test]
async fn disabling_a_plugin_whose_registration_was_refused_keeps_the_builtin() {
    let calls = Calls::default();
    let commands = with_builtin(&calls);
    let f = factory(&commands);
    let evil = f.context("evil");

    assert_eq!(
        evil.command_manager()
            .register(CommandSpec::new("infrarust"), handler("evil", &calls)),
        Err(CommandError::Reserved("infrarust".into()))
    );
    assert!(tracked(&evil).is_empty());
    disable(&evil);

    assert!(run(&commands, "infrarust").await, "/infrarust was deleted");
    assert!(run(&commands, "ir").await, "/ir was deleted");
    assert_eq!(*calls.lock().unwrap(), ["builtin", "builtin"]);
}

#[tokio::test]
async fn a_plugin_cannot_take_over_or_remove_another_plugins_command() {
    let calls = Calls::default();
    let commands = Arc::new(CommandManagerImpl::new());
    let f = factory(&commands);
    let a = f.context("a");
    let b = f.context("b");

    a.command_manager()
        .register(CommandSpec::new("hello"), handler("a", &calls))
        .unwrap();
    assert_eq!(
        b.command_manager()
            .register(CommandSpec::new("hello"), handler("b", &calls)),
        Err(CommandError::OwnedBy {
            name: "hello".into(),
            plugin: "a".into()
        })
    );
    assert_eq!(
        b.command_manager().unregister("hello"),
        Err(CommandError::NotOwned("hello".into()))
    );
    assert_eq!(
        b.command_manager().unregister("a:hello"),
        Err(CommandError::NotOwned("a:hello".into()))
    );
    disable(&b);

    assert!(run(&commands, "hello").await, "a's /hello was removed");
    assert!(run(&commands, "a:hello").await);
    assert_eq!(*calls.lock().unwrap(), ["a", "a"]);
}

#[tokio::test]
async fn an_alias_cannot_shadow_a_builtin_alias() {
    let calls = Calls::default();
    let commands = with_builtin(&calls);
    let f = factory(&commands);
    let p = f.context("p");

    let registration = p
        .command_manager()
        .register(
            CommandSpec::new("tool").alias("ir"),
            handler("plugin", &calls),
        )
        .unwrap();
    assert_eq!(registration.rejected_aliases, ["ir"]);

    assert!(run(&commands, "ir").await);
    assert!(run(&commands, "p:tool").await);
    assert_eq!(*calls.lock().unwrap(), ["builtin", "plugin"]);
}

#[tokio::test]
async fn disabling_a_plugin_removes_only_its_own_commands() {
    let calls = Calls::default();
    let commands = with_builtin(&calls);
    let f = factory(&commands);
    let a = f.context("a");
    let b = f.context("b");
    a.command_manager()
        .register(CommandSpec::new("home").alias("h"), handler("a", &calls))
        .unwrap();
    b.command_manager()
        .register(CommandSpec::new("warp"), handler("b", &calls))
        .unwrap();
    assert_eq!(tracked(&a), ["a:home"]);

    disable(&a);

    assert!(!run(&commands, "home").await);
    assert!(!run(&commands, "h").await);
    assert!(run(&commands, "warp").await);
    assert!(run(&commands, "infrarust").await);
    b.command_manager()
        .register(CommandSpec::new("home"), handler("b", &calls))
        .unwrap();
    assert!(run(&commands, "b:home").await);
}

#[test]
fn a_plugin_sees_every_command_but_lists_only_its_own() {
    let calls = Calls::default();
    let commands = with_builtin(&calls);
    let f = factory(&commands);
    let a = f.context("a");
    let b = f.context("b");
    a.command_manager()
        .register(CommandSpec::new("home").alias("h"), handler("a", &calls))
        .unwrap();
    a.command_manager()
        .register(CommandSpec::new("spawn"), handler("a", &calls))
        .unwrap();
    b.command_manager()
        .register(CommandSpec::new("warp"), handler("b", &calls))
        .unwrap();

    let names = |infos: Vec<CommandInfo>| -> Vec<String> {
        infos.iter().map(|info| info.name().to_string()).collect()
    };
    assert_eq!(names(a.command_manager().list_owned()), ["home", "spawn"]);
    assert_eq!(names(b.command_manager().list_owned()), ["warp"]);

    let seen_by_b = b.command_manager();
    assert_eq!(
        seen_by_b.get_by_alias("h").and_then(|info| info.plugin_id),
        Some("a".into())
    );
    assert_eq!(
        seen_by_b.get_by_name("ir"),
        None,
        "ir is the alias of a built-in"
    );
    assert!(seen_by_b.contains("a:spawn"));
    assert!(seen_by_b.get("infrarust").is_some());

    disable(&a);
    assert!(a.command_manager().list_owned().is_empty());
    assert_eq!(seen_by_b.get("home"), None);
    assert!(!seen_by_b.contains("h"));
}

#[tokio::test]
async fn a_handle_kept_after_enable_registers_for_the_same_plugin() {
    let calls = Calls::default();
    let commands = Arc::new(CommandManagerImpl::new());
    let f = factory(&commands);
    let late = f.context("late");
    let handle = late.command_manager();

    handle
        .register(CommandSpec::new("later"), handler("late", &calls))
        .unwrap();
    assert!(run(&commands, "late:later").await);
    disable(&late);
    assert!(!run(&commands, "later").await);
}

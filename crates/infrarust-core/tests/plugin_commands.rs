#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use infrarust_api::command::{
    CommandContext, CommandError, CommandHandler, CommandSource, CommandSpec,
};
use infrarust_api::event::BoxFuture;
use infrarust_api::permissions::AllPermissionsChecker;
use infrarust_api::plugin::PluginContext;
use infrarust_core::event_bus::EventBusImpl;
use infrarust_core::plugin::context::PluginContextImpl;
use infrarust_core::plugin::context_factory::{PluginContextFactory, PluginContextFactoryImpl};
use infrarust_core::plugin::manager::PluginServices;
use infrarust_core::services::command_manager::{CommandManagerImpl, DispatchOutcome};
use infrarust_core::services::scheduler::SchedulerImpl;
use infrarust_core::services::server_manager_bridge::NoopServerManager;

mod mock_services;
use mock_services::{
    MockBanService, MockConfigService, MockLoadBalancerService, MockPlayerRegistry,
};

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
        event_bus: Arc::new(EventBusImpl::new()),
        player_registry: Arc::new(MockPlayerRegistry),
        server_manager: Arc::new(NoopServerManager),
        ban_service: Arc::new(MockBanService),
        command_manager: Arc::clone(commands),
        scheduler: Arc::new(SchedulerImpl::new()),
        config_service: Arc::new(MockConfigService),
        load_balancer_service: Arc::new(MockLoadBalancerService),
        plugin_registry: Arc::new(infrarust_core::plugin::PluginRegistryImpl::new()),
        codec_filter_registry: Arc::new(
            infrarust_core::filter::codec_registry::CodecFilterRegistryImpl::new(),
        ),
        transport_filter_registry: Arc::new(
            infrarust_core::filter::transport_registry::TransportFilterRegistryImpl::new(),
        ),
        domain_router: Arc::new(infrarust_core::routing::DomainRouter::new()),
        proxy_shutdown: tokio_util::sync::CancellationToken::new(),
        proxy_info: infrarust_api::services::proxy_info::ProxyInfo::default(),
        plugins_dir: std::path::PathBuf::from("plugins"),
    };
    PluginContextFactoryImpl::new(services, HashMap::new())
}

fn disable(ctx: &Arc<dyn PluginContext>) {
    ctx.as_any()
        .downcast_ref::<PluginContextImpl>()
        .expect("real PluginContextImpl")
        .cleanup();
}

fn tracked(ctx: &Arc<dyn PluginContext>) -> Vec<String> {
    ctx.as_any()
        .downcast_ref::<PluginContextImpl>()
        .expect("real PluginContextImpl")
        .tracked_commands()
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
    let evil = f.create_context("evil");

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
    let a = f.create_context("a");
    let b = f.create_context("b");

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
    let p = f.create_context("p");

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
    let a = f.create_context("a");
    let b = f.create_context("b");
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

#[tokio::test]
async fn a_handle_kept_after_enable_registers_for_the_same_plugin() {
    let calls = Calls::default();
    let commands = Arc::new(CommandManagerImpl::new());
    let f = factory(&commands);
    let late = f.create_context("late");
    let handle = late.command_manager_handle();

    handle
        .register(CommandSpec::new("later"), handler("late", &calls))
        .unwrap();
    assert!(run(&commands, "late:later").await);
    disable(&late);
    assert!(!run(&commands, "later").await);
}

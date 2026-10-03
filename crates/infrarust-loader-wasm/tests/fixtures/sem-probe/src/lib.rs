use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::io::Write;
use std::rc::Rc;
use std::time::{Duration, Instant};

use infrarust_plugin_sdk::prelude::*;

const CONFIG: &str = "/probe.txt";
const LOG: &str = "/log.txt";

thread_local! {
    static STARTED: Cell<Option<Instant>> = const { Cell::new(None) };
    static HELD: RefCell<BTreeMap<u64, SessionHandle>> = const { RefCell::new(BTreeMap::new()) };
    static KEEPER: Cell<bool> = const { Cell::new(false) };
}

fn log(line: &str) {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(LOG)
        .expect("opening the fixture log");
    writeln!(file, "{line}").expect("writing the fixture log");
}

fn elapsed_ms() -> u128 {
    STARTED
        .with(Cell::get)
        .map_or(0, |started| started.elapsed().as_millis())
}

fn outcome<T>(result: &Result<T, Error>) -> &'static str {
    match result {
        Ok(_) => "ok",
        Err(error) => error.kind().as_str(),
    }
}

fn sleep_for(name: &str) -> Option<Duration> {
    name.strip_prefix("Sleep")
        .and_then(|ms| ms.parse().ok())
        .map(Duration::from_millis)
}

fn counter() -> Rc<Cell<u32>> {
    Rc::new(Cell::new(0))
}

fn bump(count: &Cell<u32>) -> u32 {
    let next = count.get() + 1;
    count.set(next);
    next
}

fn nest(ctx: &Context) -> Result<(), PluginError> {
    let count = counter();
    ctx.on::<PostLoginEvent>(EventPriority::NORMAL, move |_| {
        let n = bump(&count);
        log(&format!("outer {n}"));
        let added = Context::new().on::<PostLoginEvent>(EventPriority::LATE, move |_| {
            log(&format!("inner {n}"));
        });
        if let Err(error) = added {
            log(&format!("inner refused {n} {}", error.kind().as_str()));
        }
    })?;
    Ok(())
}

fn self_cancel(ctx: &Context) -> Result<(), PluginError> {
    let slot: Rc<RefCell<Option<EventSubscription>>> = Rc::default();
    let inside = Rc::clone(&slot);
    let count = counter();
    let subscription = ctx.on::<PostLoginEvent>(EventPriority::NORMAL, move |_| {
        let n = bump(&count);
        log(&format!("selfcancel {n}"));
        if let Some(subscription) = inside.borrow_mut().take() {
            subscription.cancel();
        }
    })?;
    *slot.borrow_mut() = Some(subscription);
    let after = counter();
    ctx.on::<PostLoginEvent>(EventPriority::LAST, move |_| {
        log(&format!("after {}", bump(&after)));
    })?;
    Ok(())
}

fn cancel_later(ctx: &Context) -> Result<(), PluginError> {
    let count = counter();
    let later = ctx.on::<PostLoginEvent>(EventPriority::LATE, move |_| {
        log(&format!("later {}", bump(&count)));
    })?;
    let slot = Rc::new(RefCell::new(Some(later)));
    ctx.on::<PostLoginEvent>(EventPriority::EARLY, move |_| {
        log("canceller");
        if let Some(later) = slot.borrow_mut().take() {
            later.cancel();
        }
    })?;
    Ok(())
}

fn many(ctx: &Context, count: u32) -> Result<(), PluginError> {
    for index in 0..count {
        ctx.on::<PostLoginEvent>(EventPriority::NORMAL, move |_| {
            log(&format!("many {index}"));
        })?;
    }
    Ok(())
}

fn interval(ctx: &Context, name: String, period: u64, busy: u64, max: u32) -> Result<(), PluginError> {
    let slot: Rc<Cell<Option<TaskHandle>>> = Rc::default();
    let inside = Rc::clone(&slot);
    let runs = counter();
    let label = name.clone();
    let handle = ctx.interval(Duration::from_millis(period), move || {
        let n = bump(&runs);
        log(&format!("tick {name} {n} {}", elapsed_ms()));
        if busy > 0 {
            std::thread::sleep(Duration::from_millis(busy));
        }
        if max > 0 && n >= max {
            if let Some(handle) = inside.get() {
                handle.cancel();
            }
            log(&format!("stop {name} {n}"));
        }
    })?;
    slot.set(Some(handle));
    log(&format!("scheduled {label}"));
    Ok(())
}

fn delay(ctx: &Context, name: String, after: u64) -> Result<(), PluginError> {
    ctx.delay(Duration::from_millis(after), move || {
        log(&format!("fired {name} {}", elapsed_ms()));
    })?;
    Ok(())
}

fn invocation_line(name: &str, invocation: &CommandInvocation) -> String {
    format!(
        "ran {name} label={} args={} raw={} sender={}",
        invocation.label,
        invocation.args.join(","),
        invocation.raw,
        invocation.sender.name()
    )
}

fn command(ctx: &Context, words: &[&str]) -> Result<(), PluginError> {
    let [name, options @ ..] = words else {
        return Err("cmd needs a name".into());
    };
    let label = (*name).to_owned();
    let mut builder = ctx.command(label.clone());
    let mut rest = options.iter();
    while let Some(option) = rest.next() {
        match *option {
            "perm" => builder = builder.permission(*rest.next().ok_or("perm needs a node")?),
            "alias" => builder = builder.alias(*rest.next().ok_or("alias needs a name")?),
            "hidden" => builder = builder.hidden(true),
            other => return Err(format!("unknown cmd option {other}").into()),
        }
    }
    let registered = builder
        .handler(move |invocation| log(&invocation_line(&label, &invocation)))
        .register();
    match &registered {
        Ok(registration) => log(&format!(
            "reg {name} ok name={} namespaced={} aliases={} rejected={}",
            registration.name,
            registration.namespaced,
            registration.aliases.join(","),
            registration.rejected_aliases.join(",")
        )),
        Err(error) => log(&format!("reg {name} {}", error.kind().as_str())),
    }
    Ok(())
}

fn tab(ctx: &Context, name: &str) -> Result<(), PluginError> {
    let label = name.to_owned();
    ctx.command(name)
        .handler(|_| {})
        .completer(move |completion| {
            log(&format!(
                "tab {label} n={} args=[{}] cursor={} partial={}",
                completion.args.len(),
                completion.args.join("|"),
                completion.cursor,
                completion.partial()
            ));
            vec![format!("s{}", completion.args.len())]
        })
        .register()?;
    Ok(())
}

fn reregister(ctx: &Context, name: &str, node: &str) -> Result<(), PluginError> {
    let first = ctx
        .command(name)
        .handler(|invocation| log(&format!("ran-v1 {}", invocation.label)))
        .register();
    log(&format!("rereg {name} v1 {}", outcome(&first)));
    let second = ctx
        .command(name)
        .permission(node)
        .handler(|invocation| log(&format!("ran-v2 {}", invocation.label)))
        .register();
    log(&format!("rereg {name} v2 {}", outcome(&second)));
    Ok(())
}

fn nested_commands(ctx: &Context) -> Result<(), PluginError> {
    ctx.command("mk")
        .handler(|_| {
            let made = Context::new()
                .command("made")
                .handler(|_| log("made ran"))
                .register();
            log(&format!("mk {}", outcome(&made)));
        })
        .register()?;
    ctx.command("once")
        .handler(|_| {
            log("once ran");
            let gone = Context::new().unregister_command("once");
            match gone {
                Ok(removed) => log(&format!("once unregistered {removed}")),
                Err(error) => log(&format!("once unregister {}", error.kind().as_str())),
            }
        })
        .register()?;
    Ok(())
}

fn unregister(ctx: &Context, name: &str) {
    match ctx.unregister_command(name) {
        Ok(removed) => log(&format!("unreg {name} {removed}")),
        Err(error) => log(&format!("unreg {name} {}", error.kind().as_str())),
    }
}

#[derive(Default)]
struct BanState {
    bans: Vec<BanRecord>,
    next: u32,
    ranges: bool,
}

#[derive(Clone, Default)]
struct ProbeBans(Rc<RefCell<BanState>>);

fn misbehave(name: &str) -> Result<(), PluginError> {
    if let Some(pause) = sleep_for(name) {
        std::thread::sleep(pause);
    }
    match name {
        "Trap" | "TrapBan" => panic!("the sem-probe provider traps on purpose"),
        "Fail" | "FailBan" => Err(format!("the provider refuses {name}").into()),
        _ => Ok(()),
    }
}

fn address_alias(attempt: &LoginAttempt) -> String {
    match attempt.ip.to_string().as_str() {
        "198.51.100.1" => "Fail".to_owned(),
        "198.51.100.2" => "Trap".to_owned(),
        "198.51.100.3" => "Sleep3000".to_owned(),
        _ => String::new(),
    }
}

fn target_name(target: &BanTarget) -> String {
    match target {
        BanTarget::Username(name) => name.clone(),
        _ => String::new(),
    }
}

impl BanProvider for ProbeBans {
    fn check(&self, attempt: &LoginAttempt) -> Result<Option<BanVerdict>, PluginError> {
        let name = attempt
            .username
            .clone()
            .unwrap_or_else(|| address_alias(attempt));
        log(&format!("check {:?} {name}", attempt.stage));
        misbehave(&name)?;
        let state = self.0.borrow();
        Ok(state
            .bans
            .iter()
            .find(|record| record.target.matches(attempt))
            .map(|record| BanVerdict::new(record.clone()).message("probe: banned")))
    }

    fn ban(&self, request: BanRequest, source: BanSource) -> Result<BanRecord, PluginError> {
        misbehave(&target_name(&request.target))?;
        let mut state = self.0.borrow_mut();
        state.next += 1;
        let record = BanRecord::new(format!("b{}", state.next), request.target, source);
        log(&format!("stored {}", record.id));
        state.bans.push(record.clone());
        Ok(record)
    }

    fn unban(&self, request: UnbanRequest) -> Result<Option<BanRecord>, PluginError> {
        misbehave(&target_name(&request.target))?;
        let mut state = self.0.borrow_mut();
        let at = state
            .bans
            .iter()
            .position(|record| record.target == request.target);
        Ok(at.map(|at| state.bans.remove(at)))
    }

    fn get(&self, target: &BanTarget) -> Result<Option<BanRecord>, PluginError> {
        misbehave(&target_name(target))?;
        Ok(self
            .0
            .borrow()
            .bans
            .iter()
            .find(|record| &record.target == target)
            .cloned())
    }

    fn list(&self, _query: &BanQuery) -> Result<BanRecordPage, PluginError> {
        Ok(BanRecordPage::new(self.0.borrow().bans.clone(), None))
    }

    fn features(&self) -> BanFeatures {
        BanFeatures::new().ip_ranges(self.0.borrow().ranges)
    }
}

#[derive(Clone, Default)]
struct ProbePerms {
    console_trap: bool,
}

impl PermissionProvider for ProbePerms {
    fn snapshot_for(&self, subject: &PermissionSubject) -> PermissionSnapshot {
        let Some(profile) = subject.profile() else {
            log("snapshot console");
            if self.console_trap {
                panic!("the sem-probe permission provider traps for the console");
            }
            return PermissionSnapshot::admin();
        };
        let name = profile.username.clone();
        log(&format!("snapshot {name}"));
        if let Some(pause) = sleep_for(&name) {
            std::thread::sleep(pause);
        }
        match name.as_str() {
            "Trap" => panic!("the sem-probe permission provider traps on purpose"),
            "Big" => {
                let mut snapshot = PermissionSnapshot::new();
                for index in 0..70_000 {
                    snapshot.set(&format!("bulk.node{index}"), true);
                }
                snapshot.grant("demo.use")
            }
            "Mixed" => PermissionSnapshot::new()
                .with(" Warps.Use ", true)
                .with("WARPS.ADMIN", false),
            _ => PermissionSnapshot::new().grant("demo.use"),
        }
    }
}

struct Keeper;

struct Answers(fn() -> HandlerOutcome);

impl LimboHandler for Answers {
    fn on_player_enter(&self, _session: &LimboSession) -> HandlerOutcome {
        (self.0)()
    }
}

impl LimboHandler for Keeper {
    fn on_player_enter(&self, session: &LimboSession) -> HandlerOutcome {
        let player = session.player_id();
        HELD.with(|held| held.borrow_mut().insert(player.as_u64(), session.handle()));
        log(&format!("enter {}", player.as_u64()));
        HandlerOutcome::Hold
    }

    fn on_disconnect(&self, player: PlayerId) {
        log(&format!("left {}", player.as_u64()));
    }

    fn on_session_end(&self, player: PlayerId, _reason: SessionEndReason) {
        log(&format!("ended {}", player.as_u64()));
    }
}

fn with_held(id: &str, act: impl FnOnce(&SessionHandle) -> String) -> String {
    let Ok(key) = id.parse::<u64>() else {
        return format!("{id} unknown");
    };
    HELD.with(|held| {
        held.borrow()
            .get(&key)
            .map_or_else(|| format!("{id} unknown"), act)
    })
}

fn limbo_tools(ctx: &Context) -> Result<(), PluginError> {
    ctx.command("hsend")
        .handler(|invocation| {
            for id in &invocation.args {
                let line = with_held(id, |handle| {
                    let sent = handle.send_message("still there?");
                    format!("{id} {} cancelled={}", outcome(&sent), handle.cancelled())
                });
                log(&format!("hsend {line}"));
            }
        })
        .register()?;
    ctx.command("hdone")
        .handler(|invocation| {
            for id in &invocation.args {
                let line = with_held(id, |handle| {
                    let done = handle.complete(HandlerOutcome::Accept);
                    format!("{id} {} cancelled={}", outcome(&done), handle.cancelled())
                });
                log(&format!("hdone {line}"));
            }
        })
        .register()?;
    ctx.command("hrearm")
        .handler(|invocation| {
            for id in &invocation.args {
                let line = with_held(id, |handle| {
                    let rearmed = handle.complete(HandlerOutcome::HoldWithTimeout {
                        after: Duration::from_secs(30),
                        on_timeout: TimeoutOutcome::Deny(Component::text("too slow")),
                    });
                    format!("{id} {} cancelled={}", outcome(&rearmed), handle.cancelled())
                });
                log(&format!("hrearm {line}"));
            }
        })
        .register()?;
    ctx.command("hcount")
        .handler(|_| {
            let (count, cancelled) = HELD.with(|held| {
                let held = held.borrow();
                (held.len(), held.values().filter(|h| h.cancelled()).count())
            });
            log(&format!("hcount {count} cancelled={cancelled}"));
        })
        .register()?;
    Ok(())
}

fn named_uncancel(ctx: &Context, name: String) -> Result<(), PluginError> {
    ctx.on_named(name.clone(), EventPriority::LATE, move |event| {
        log(&format!(
            "named {name} saw cancelled={} response={}",
            event.is_cancelled(),
            event.response().and_then(NamedResponse::text).unwrap_or("-")
        ));
        event.uncancel();
        event.clear_response();
    })?;
    Ok(())
}

fn describe_setup(result: &PermissionsSetupResult) -> String {
    match result {
        PermissionsSetupResult::UseDefault => "use-default".to_owned(),
        PermissionsSetupResult::Custom(snapshot) => {
            let rules: Vec<String> = snapshot
                .rules()
                .map(|(node, value)| format!("{node}={value}"))
                .collect();
            format!("custom admin={} rules={}", snapshot.is_admin(), rules.join(","))
        }
        _ => "other".to_owned(),
    }
}

fn permission_setup(ctx: &Context, priority: u8, action: String) -> Result<(), PluginError> {
    ctx.on::<PermissionsSetupEvent>(EventPriority::custom(priority), move |event| {
        log(&format!("permsetup@{priority} saw {}", describe_setup(event.result())));
        match action.as_str() {
            "provide" => event.provide(PermissionSnapshot::new().grant("probe.node")),
            "reset" => event.use_default(),
            _ => {}
        }
    })?;
    Ok(())
}

fn chat_append(ctx: &Context, priority: u8, tag: String) -> Result<(), PluginError> {
    ctx.on::<ChatMessageEvent>(EventPriority::custom(priority), move |event| {
        let base = match event.result() {
            ChatMessageResult::Modify(message) => message.clone(),
            _ => event.message.clone(),
        };
        event.modify(format!("{base}|{tag}"));
    })?;
    Ok(())
}

fn describe_login(result: &PreLoginResult) -> String {
    match result {
        PreLoginResult::Allowed => "allowed".to_owned(),
        PreLoginResult::Denied(reason) => format!("denied:{}", reason.to_plain()),
        PreLoginResult::ForceOffline => "force-offline".to_owned(),
        PreLoginResult::ForceOnline => "force-online".to_owned(),
        _ => "other".to_owned(),
    }
}

fn pre_login(ctx: &Context, priority: u8, action: String) -> Result<(), PluginError> {
    ctx.on::<PreLoginEvent>(EventPriority::custom(priority), move |event| {
        log(&format!("prelogin@{priority} saw {}", describe_login(event.result())));
        match action.as_str() {
            "allow" => event.allow(),
            "deny" => event.deny(Component::text("probe says no")),
            _ => {}
        }
    })?;
    Ok(())
}

fn tools(ctx: &Context) -> Result<(), PluginError> {
    ctx.command("unreg")
        .handler(|invocation| {
            for name in &invocation.args {
                unregister(&Context::new(), name);
            }
        })
        .register()?;
    ctx.command("trap")
        .handler(|_| panic!("the sem-probe fixture traps on purpose"))
        .register()?;
    Ok(())
}

fn fire(ctx: &Context, command: &str, event: String) -> Result<(), PluginError> {
    ctx.command(command)
        .handler(move |_| {
            let started = Instant::now();
            let fired = Context::new().fire_named_text(&event, "ping");
            let took = started.elapsed().as_millis() / 100 * 100;
            match fired {
                Ok(outcome) => log(&format!(
                    "fired {event} cancelled={} response={}",
                    outcome.cancelled,
                    outcome.response.as_ref().and_then(NamedResponse::text).unwrap_or("-")
                )),
                Err(error) => log(&format!("fire {event} {} after {took}ms", error.kind().as_str())),
            }
        })
        .register()?;
    Ok(())
}

macro_rules! dump_events {
    ($ctx:expr, $($event:ident),* $(,)?) => {
        $(
            $ctx.on::<$event>(EventPriority::NORMAL, |event| {
                log(&format!("{} {:?}", stringify!($event), event).replace('\n', " "));
            })?;
        )*
    };
}

fn dump(ctx: &Context) -> Result<(), PluginError> {
    dump_events!(
        ctx,
        PreLoginEvent,
        PostLoginEvent,
        DisconnectEvent,
        OnlineAuthFailedEvent,
        PermissionsSetupEvent,
        PlayerChooseInitialServerEvent,
        ServerPreConnectEvent,
        ServerConnectedEvent,
        ServerPostConnectEvent,
        KickedFromServerEvent,
        ChatMessageEvent,
        ProxyPingEvent,
        ProxyInitializeEvent,
        ProxyShutdownEvent,
        ConfigReloadEvent,
        ServerStateChangeEvent,
        BackendHealthEvent,
        LoginEvent,
        GameProfileRequestEvent,
        CommandExecuteEvent,
        ConnectionHandshakeEvent,
        ConnectionRejectedEvent,
        LimboEnterEvent,
        LimboExitEvent,
        PlayerClientBrandEvent,
        PlayerSettingsChangedEvent,
        PlayerChannelRegisterEvent,
        PluginMessageEvent,
        BanIssuedEvent,
        BanRevokedEvent,
        PluginEnabledEvent,
        PluginDisabledEvent,
        PreTransferEvent,
        PlayerResourcePackStatusEvent,
        NamedEvent,
    );
    Ok(())
}

fn answer(ctx: &Context) -> Result<(), PluginError> {
    ctx.on::<ChatMessageEvent>(EventPriority::LATE, ChatMessageEvent::deny_silently)?;
    ctx.on::<CommandExecuteEvent>(EventPriority::LATE, CommandExecuteEvent::deny_silently)?;
    ctx.on::<ConnectionHandshakeEvent>(
        EventPriority::LATE,
        ConnectionHandshakeEvent::deny_silently,
    )?;
    ctx.on::<KickedFromServerEvent>(EventPriority::LATE, |event| {
        event.set_result(KickedFromServerResult::DisconnectPlayer(None));
    })?;
    ctx.on::<ProxyPingEvent>(EventPriority::LATE, |event| {
        event.set_max_players(-1);
        event.set_online_players(i32::MAX);
        event.set_version_protocol(5);
        event.set_version_name(String::new());
        event.set_favicon(None);
        event.set_player_sample(vec![("Zed".to_owned(), Uuid::from_u128(9))]);
    })?;
    ctx.on::<GameProfileRequestEvent>(EventPriority::LATE, |event| {
        let profile = event.profile_mut();
        profile.uuid = Uuid::from_u128(99);
        profile.properties.clear();
    })?;
    ctx.on::<PermissionsSetupEvent>(EventPriority::LATE, PermissionsSetupEvent::use_default)?;
    Ok(())
}

fn whitelist(ctx: &Context) -> Result<(), PluginError> {
    ctx.on::<PreLoginEvent>(EventPriority::NORMAL, |event| {
        let name = event.profile.username.clone();
        if let Some(pause) = sleep_for(&name) {
            std::thread::sleep(pause);
        }
        log(&format!("whitelist {name}"));
        if name != "Friend" {
            event.deny(Component::text("not whitelisted"));
        }
    })?;
    Ok(())
}

fn configure(ctx: &Context, line: &str) -> Result<(), PluginError> {
    let words: Vec<&str> = line.split_whitespace().collect();
    match words.as_slice() {
        ["nest"] => nest(ctx),
        ["selfcancel"] => self_cancel(ctx),
        ["cancel-later"] => cancel_later(ctx),
        ["many", count] => many(ctx, count.parse().map_err(|_| "many needs a count")?),
        ["interval", name, period, busy, max] => interval(
            ctx,
            (*name).to_owned(),
            period.parse().map_err(|_| "bad period")?,
            busy.parse().map_err(|_| "bad busy")?,
            max.parse().map_err(|_| "bad max")?,
        ),
        ["delay", name, after] => delay(
            ctx,
            (*name).to_owned(),
            after.parse().map_err(|_| "bad delay")?,
        ),
        ["cmd", rest @ ..] => command(ctx, rest),
        ["tab", name] => tab(ctx, name),
        ["reregister", name, node] => reregister(ctx, name, node),
        ["nested-commands"] => nested_commands(ctx),
        ["unregister", name] => {
            unregister(ctx, name);
            Ok(())
        }
        ["bans", options @ ..] => {
            let store = ProbeBans::default();
            store.0.borrow_mut().ranges = !options.contains(&"noranges");
            let registered = ctx.provide_bans(store);
            log(&format!("bans {}", outcome(&registered)));
            Ok(())
        }
        ["perms", options @ ..] => {
            let registered = ctx.provide_permissions(ProbePerms {
                console_trap: options.contains(&"console-trap"),
            });
            log(&format!("perms {}", outcome(&registered)));
            Ok(())
        }
        ["tools"] => tools(ctx),
        ["dump"] => dump(ctx),
        ["answer"] => answer(ctx),
        ["whitelist"] => whitelist(ctx),
        ["fire", command, event] => fire(ctx, command, (*event).to_owned()),
        ["keeper"] => {
            KEEPER.with(|keeper| keeper.set(true));
            limbo_tools(ctx)
        }
        ["named-uncancel", name] => named_uncancel(ctx, (*name).to_owned()),
        ["permsetup", priority, action] => permission_setup(
            ctx,
            priority.parse().map_err(|_| "bad priority")?,
            (*action).to_owned(),
        ),
        ["chat-append", priority, tag] => chat_append(
            ctx,
            priority.parse().map_err(|_| "bad priority")?,
            (*tag).to_owned(),
        ),
        ["prelogin", priority, action] => pre_login(
            ctx,
            priority.parse().map_err(|_| "bad priority")?,
            (*action).to_owned(),
        ),
        other => Err(format!("unknown sem-probe directive {other:?}").into()),
    }
}

#[derive(Default)]
struct SemProbe;

#[plugin(id = "sem-probe", name = "Semantics Probe Fixture")]
impl Plugin for SemProbe {
    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
        STARTED.with(|started| started.set(Some(Instant::now())));
        let config = std::fs::read_to_string(CONFIG).unwrap_or_default();
        for line in config
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            configure(ctx, line)?;
        }
        let line = match ctx.enable_reason() {
            Some(EnableReason::Recovered(info)) => format!("enable recovered {}", info.attempt),
            _ => "enable".to_owned(),
        };
        log(&line);
        Ok(())
    }

    fn on_disable(&self, _ctx: &Context) -> Result<(), PluginError> {
        log("disable");
        Ok(())
    }

    fn register_limbo_handlers(reg: &mut LimboRegistrar) {
        if KEEPER.with(Cell::get) {
            reg.add("keeper", Keeper);
            reg.add(
                "denier",
                Answers(|| HandlerOutcome::Deny(Component::text("no entry"))),
            );
            reg.add("redirector", Answers(|| HandlerOutcome::Redirect("hub".into())));
            reg.add(
                "chainer",
                Answers(|| HandlerOutcome::SendToLimbo(vec!["keeper".to_owned()])),
            );
            reg.add(
                "timed",
                Answers(|| HandlerOutcome::HoldWithTimeout {
                    after: Duration::from_millis(1500),
                    on_timeout: TimeoutOutcome::Redirect("fallback".into()),
                }),
            );
        }
    }
}

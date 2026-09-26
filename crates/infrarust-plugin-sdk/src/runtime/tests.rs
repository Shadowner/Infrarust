use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use super::*;
use crate::bindings::types as wt;
use crate::command::Suggestion;
use crate::component::Component;
use crate::context::{EventSubscription, TaskHandle};
use crate::error::{ErrorKind, PluginError};
use crate::event::ProxyShutdownEvent;
use crate::host;
use crate::limbo::{HandlerOutcome, LimboHandler, LimboSession};
use crate::plugin::PluginMetadata;

type Invocation = (Vec<String>, Option<u64>);

struct DropProbe(Rc<Cell<bool>>);

impl Drop for DropProbe {
    fn drop(&mut self) {
        self.0.set(true);
    }
}

fn probe() -> (DropProbe, Rc<Cell<bool>>) {
    let dropped = Rc::new(Cell::new(false));
    (DropProbe(Rc::clone(&dropped)), dropped)
}

fn counter() -> Rc<Cell<u32>> {
    Rc::new(Cell::new(0))
}

fn bump(count: &Cell<u32>) {
    count.set(count.get() + 1);
}

fn last_task() -> (u64, u64) {
    host::with_fake(|h| {
        h.tasks
            .iter()
            .max_by_key(|(handle, _)| **handle)
            .map(|(handle, (callback, _))| (*handle, *callback))
            .expect("a task was scheduled on the host")
    })
}

fn fire(callback_id: u64) {
    host::with_fake(|h| {
        h.tasks
            .retain(|_, (callback, repeating)| *callback != callback_id || *repeating);
    });
    on_scheduled_task(callback_id);
}

fn host_cancelled(handle: u64) -> bool {
    host::with_fake(|h| h.cancelled.contains(&handle))
}

fn command_callback(name: &str) -> u64 {
    host::with_fake(|h| h.commands.get(name).copied())
        .unwrap_or_else(|| panic!("command `{name}` registered on the host"))
}

fn last_listener() -> u64 {
    host::with_fake(|h| h.listeners.keys().max().copied()).expect("a listener was subscribed")
}

fn is_subscribed(listener: u64) -> bool {
    host::with_fake(|h| h.listeners.contains_key(&listener))
}

fn refuse(name: &str) {
    host::with_fake(|h| h.refused.insert(name.to_owned()));
}

fn steve(id: u64) -> wt::PlayerRef {
    wt::PlayerRef {
        id,
        uuid: wt::Uuid { hi: 0, lo: id },
        username: "Steve".into(),
    }
}

fn sender(player: Option<u64>) -> wg::CommandSender {
    player.map_or(wg::CommandSender::Console, |id| {
        wg::CommandSender::Player(steve(id))
    })
}

fn invoke(handler: u64, args: Vec<String>, player: Option<u64>) {
    handle_command(
        handler,
        wg::CommandInvocation {
            label: "cmd".into(),
            raw: args.join(" "),
            args,
            sender: sender(player),
        },
    );
}

fn complete(handler: u64, args: Vec<String>) -> Vec<String> {
    tab_complete(handler, sender(None), args, 0)
        .into_iter()
        .map(|suggestion| suggestion.text)
        .collect()
}

fn register(name: &str, handler: impl FnMut(CommandInvocation) + 'static) {
    Context::new()
        .command(name)
        .handler(handler)
        .register()
        .expect("the host accepts the command");
}

#[test]
fn delay_closure_is_dropped_after_firing() {
    let ctx = Context::new();
    let (token, dropped) = probe();
    let runs = counter();
    let seen = Rc::clone(&runs);
    ctx.delay(Duration::from_millis(5), move || {
        let _ = &token;
        bump(&seen);
    })
    .unwrap();
    let (_, callback) = last_task();
    assert!(!dropped.get(), "pending delay keeps its closure");

    fire(callback);
    assert_eq!(runs.get(), 1);
    assert!(dropped.get(), "one-shot closure dropped once it has run");

    on_scheduled_task(callback);
    assert_eq!(runs.get(), 1, "a stale fire of a spent delay is ignored");
}

#[test]
fn cancel_drops_a_pending_delay_and_tells_the_host() {
    let ctx = Context::new();
    let (token, dropped) = probe();
    let runs = counter();
    let seen = Rc::clone(&runs);
    let handle = ctx
        .delay(Duration::from_secs(60), move || {
            let _ = &token;
            bump(&seen);
        })
        .unwrap();
    let (host_handle, callback) = last_task();

    ctx.cancel(handle);
    assert!(dropped.get(), "cancelled closure dropped on the guest");
    assert!(host_cancelled(host_handle), "host told to cancel");

    on_scheduled_task(callback);
    assert_eq!(runs.get(), 0, "a fire racing the cancel runs nothing");
}

#[test]
fn cancel_drops_an_interval_closure() {
    let ctx = Context::new();
    let (token, dropped) = probe();
    let runs = counter();
    let seen = Rc::clone(&runs);
    let handle = ctx
        .interval(Duration::from_millis(10), move || {
            let _ = &token;
            bump(&seen);
        })
        .unwrap();
    let (host_handle, callback) = last_task();
    fire(callback);
    fire(callback);
    assert_eq!(runs.get(), 2, "interval keeps firing");
    assert!(!dropped.get());

    handle.cancel();
    assert!(dropped.get());
    assert!(host_cancelled(host_handle));
    fire(callback);
    assert_eq!(runs.get(), 2);
}

#[test]
fn interval_cancelled_from_its_own_callback() {
    let ctx = Context::new();
    let (token, dropped) = probe();
    let runs = counter();
    let own: Rc<Cell<Option<TaskHandle>>> = Rc::new(Cell::new(None));
    let seen = Rc::clone(&runs);
    let slot = Rc::clone(&own);
    let handle = ctx
        .interval_with_delay(
            Duration::from_millis(10),
            Duration::from_millis(1),
            move || {
                let _ = &token;
                bump(&seen);
                if let Some(handle) = slot.get() {
                    Context::new().cancel(handle);
                }
            },
        )
        .unwrap();
    own.set(Some(handle));
    let (host_handle, callback) = last_task();

    fire(callback);
    assert_eq!(runs.get(), 1);
    assert!(
        host_cancelled(host_handle),
        "host told to stop the interval"
    );
    assert!(
        dropped.get(),
        "closure dropped once its in-flight call returned"
    );

    fire(callback);
    assert_eq!(runs.get(), 1, "cancelled interval never runs again");
}

#[test]
fn delay_can_schedule_another_delay_while_firing() {
    let ctx = Context::new();
    let runs = counter();
    let seen = Rc::clone(&runs);
    ctx.delay(Duration::from_millis(1), move || {
        bump(&seen);
        let again = Rc::clone(&seen);
        Context::new()
            .delay(Duration::from_millis(1), move || bump(&again))
            .unwrap();
    })
    .unwrap();
    let (_, first) = last_task();
    fire(first);
    let (_, second) = last_task();
    assert_ne!(first, second);
    fire(second);
    assert_eq!(runs.get(), 2);
}

#[test]
fn a_refused_schedule_keeps_no_closure() {
    refuse("scheduler");
    let (token, dropped) = probe();
    let refused = Context::new().delay(Duration::from_millis(1), move || {
        let _ = &token;
    });
    assert_eq!(refused.unwrap_err().kind(), ErrorKind::Conflict);
    assert!(dropped.get(), "the refused task is dropped at once");
}

#[test]
fn completer_can_register_a_command_with_a_completer() {
    let ctx = Context::new();
    ctx.command("outer")
        .completer(|_| {
            Context::new()
                .command("inner")
                .completer(|_| vec!["deep"])
                .register()
                .unwrap();
            vec!["registered"]
        })
        .register()
        .unwrap();

    let outer = command_callback("outer");
    assert_eq!(complete(outer, Vec::new()), ["registered"]);
    let inner = command_callback("inner");
    assert_eq!(complete(inner, Vec::new()), ["deep"]);
    assert_eq!(complete(outer, Vec::new()), ["registered"]);
}

#[test]
fn tab_complete_without_a_completer_is_empty() {
    Context::new().command("bare").register().unwrap();
    assert!(complete(command_callback("bare"), vec!["x".to_owned()]).is_empty());
    assert!(complete(u64::MAX, Vec::new()).is_empty());
}

#[test]
fn a_completion_sees_the_sender_and_the_partial_argument() {
    Context::new()
        .command("who")
        .completer(|completion| {
            vec![
                Suggestion::new(format!(
                    "{}:{}",
                    completion.sender.name(),
                    completion.partial()
                ))
                .with_tooltip(Component::text("tip")),
            ]
        })
        .register()
        .unwrap();
    let suggestions = tab_complete(
        command_callback("who"),
        sender(Some(3)),
        vec!["a".into(), "b".into()],
        3,
    );
    assert_eq!(suggestions.len(), 1);
    assert_eq!(suggestions[0].text, "Steve:b");
    assert_eq!(
        suggestions[0].tooltip,
        Some(Component::text("tip").to_arena())
    );
}

#[test]
fn command_handler_receives_the_invocation() {
    let seen: Rc<RefCell<Vec<Invocation>>> = Rc::default();
    let sink = Rc::clone(&seen);
    register("echo", move |inv| {
        let player = inv.player().map(|p| p.id.as_u64());
        sink.borrow_mut().push((inv.args, player));
    });
    let callback = command_callback("echo");
    invoke(callback, vec!["a".to_owned(), "b".to_owned()], Some(4));
    invoke(callback, Vec::new(), None);
    assert_eq!(
        *seen.borrow(),
        [
            (vec!["a".to_owned(), "b".to_owned()], Some(4)),
            (Vec::new(), None)
        ]
    );
}

#[test]
fn command_handler_can_register_a_command_while_dispatching() {
    let runs = counter();
    let seen = Rc::clone(&runs);
    register("spawn", move |_| {
        let inner = Rc::clone(&seen);
        register("spawned", move |_| bump(&inner));
    });
    invoke(command_callback("spawn"), Vec::new(), None);
    invoke(command_callback("spawned"), Vec::new(), None);
    assert_eq!(runs.get(), 1);
}

#[test]
fn reregistering_a_command_name_drops_the_replaced_closure() {
    let (first_token, first_dropped) = probe();
    let first_runs = counter();
    let second_runs = counter();
    let seen = Rc::clone(&first_runs);
    register("dup", move |_| {
        let _ = &first_token;
        bump(&seen);
    });
    let first = command_callback("dup");
    let seen = Rc::clone(&second_runs);
    register("DUP", move |_| bump(&seen));
    let second = command_callback("dup");

    assert_ne!(first, second);
    assert!(first_dropped.get(), "replaced command closure dropped");
    invoke(first, Vec::new(), None);
    invoke(second, Vec::new(), None);
    assert_eq!((first_runs.get(), second_runs.get()), (0, 1));
}

#[test]
fn a_refused_registration_keeps_the_previous_handler() {
    let runs = counter();
    let seen = Rc::clone(&runs);
    register("kept", move |_| bump(&seen));
    let first = command_callback("kept");
    refuse("kept");
    let (token, dropped) = probe();
    let refused = Context::new()
        .command("kept")
        .handler(move |_| {
            let _ = &token;
        })
        .register();

    assert_eq!(refused.unwrap_err().kind(), ErrorKind::Conflict);
    assert!(dropped.get(), "the refused handler is dropped");
    invoke(first, Vec::new(), None);
    assert_eq!(runs.get(), 1, "the handler the host kept still runs");
    assert_eq!(
        Context::new().unregister_command("kept"),
        Ok(true),
        "the guest still owns the command the host kept"
    );
}

#[test]
fn a_registration_reports_what_the_host_registered() {
    let registration = Context::new()
        .command("report")
        .aliases(["rep", "r"])
        .description("reports")
        .usage("/report <player>")
        .permission("example.report")
        .hidden(true)
        .register()
        .unwrap();
    assert_eq!(registration.name, "report");
    assert_eq!(registration.aliases, ["rep", "r"]);
    assert!(registration.unregister().unwrap());
    assert!(host::with_fake(|h| !h.commands.contains_key("report")));
}

#[test]
fn cancel_after_a_delay_fired_is_a_guest_side_no_op() {
    let ctx = Context::new();
    let handle = ctx.delay(Duration::from_millis(1), || {}).unwrap();
    let (host_handle, callback) = last_task();
    fire(callback);
    ctx.cancel(handle);
    assert!(
        !host_cancelled(host_handle),
        "spent delay is not cancelled twice"
    );
}

#[test]
fn unregister_command_drops_the_closure_and_tells_the_host() {
    let ctx = Context::new();
    let (token, dropped) = probe();
    let runs = counter();
    let seen = Rc::clone(&runs);
    ctx.command("bye")
        .handler(move |_| {
            let _ = &token;
            bump(&seen);
        })
        .completer(|_| vec!["now"])
        .register()
        .unwrap();
    let callback = command_callback("bye");

    assert_eq!(ctx.unregister_command("Bye"), Ok(true));
    assert!(dropped.get(), "unregistered command closure dropped");
    assert!(host::with_fake(|h| !h.commands.contains_key("bye")));
    invoke(callback, Vec::new(), None);
    assert_eq!(runs.get(), 0);
    assert!(complete(callback, Vec::new()).is_empty());
    assert_eq!(
        ctx.unregister_command("bye"),
        Ok(false),
        "second unregister finds nothing"
    );
}

#[test]
fn unregister_leaves_commands_this_plugin_does_not_own_alone() {
    host::with_fake(|h| h.commands.insert("foreign".to_owned(), 77));
    assert_eq!(Context::new().unregister_command("foreign"), Ok(false));
    assert!(host::with_fake(|h| h.commands.contains_key("foreign")));
}

#[test]
fn command_unregistering_itself_is_dropped_after_its_call() {
    let (token, dropped) = probe();
    let outcome: Rc<Cell<Option<(bool, bool)>>> = Rc::default();
    let seen = Rc::clone(&outcome);
    let flag = Rc::clone(&dropped);
    register("once", move |_| {
        let _ = &token;
        let removed = Context::new().unregister_command("once") == Ok(true);
        seen.set(Some((removed, flag.get())));
    });
    let callback = command_callback("once");

    invoke(callback, Vec::new(), None);
    assert_eq!(
        outcome.get(),
        Some((true, false)),
        "unregistered from inside, still alive while running"
    );
    assert!(dropped.get(), "dropped once the in-flight call returned");
    invoke(callback, Vec::new(), None);
    assert_eq!(outcome.get(), Some((true, false)));
}

#[test]
fn command_can_unregister_another_command_mid_dispatch() {
    let (token, dropped) = probe();
    let victim_runs = counter();
    let seen = Rc::clone(&victim_runs);
    register("victim", move |_| {
        let _ = &token;
        bump(&seen);
    });
    register("reaper", |_| {
        assert_eq!(Context::new().unregister_command("victim"), Ok(true));
    });
    let victim = command_callback("victim");

    invoke(command_callback("reaper"), Vec::new(), None);
    assert!(dropped.get());
    invoke(victim, Vec::new(), None);
    assert_eq!(victim_runs.get(), 0);
}

#[test]
fn completer_can_unregister_its_own_command() {
    let (token, dropped) = probe();
    Context::new()
        .command("fleeting")
        .completer(move |_| {
            let _ = &token;
            let removed = Context::new().unregister_command("fleeting") == Ok(true);
            vec![removed.to_string()]
        })
        .register()
        .unwrap();
    let callback = command_callback("fleeting");

    assert_eq!(complete(callback, Vec::new()), ["true"]);
    assert!(dropped.get());
    assert!(complete(callback, Vec::new()).is_empty());
}

#[test]
fn command_reregistering_its_own_name_mid_dispatch_swaps_the_handler() {
    let (token, dropped) = probe();
    let second_runs = counter();
    let seen = Rc::clone(&second_runs);
    register("phoenix", move |_| {
        let _ = &token;
        let again = Rc::clone(&seen);
        register("phoenix", move |_| bump(&again));
    });
    let first = command_callback("phoenix");

    invoke(first, Vec::new(), None);
    assert!(dropped.get(), "replaced handler dropped after its call");
    let second = command_callback("phoenix");
    assert_ne!(first, second);
    invoke(second, Vec::new(), None);
    assert_eq!(second_runs.get(), 1);
}

#[test]
fn reentrant_dispatch_of_a_running_event_handler_is_skipped() {
    let runs = counter();
    let own = Rc::new(Cell::new(0));
    let seen = Rc::clone(&runs);
    let listener_slot = Rc::clone(&own);
    Context::new()
        .on::<ProxyShutdownEvent>(EventPriority::Normal, move |_| {
            bump(&seen);
            let nested = handle_event(listener_slot.get(), Event::ProxyShutdown);
            assert_eq!(nested, EventOutcome::Unchanged);
        })
        .unwrap();
    own.set(last_listener());
    handle_event(own.get(), Event::ProxyShutdown);
    assert_eq!(runs.get(), 1);
}

#[test]
fn event_handler_unsubscribing_itself_is_dropped_after_the_call() {
    let (token, dropped) = probe();
    let runs = counter();
    let own: Rc<RefCell<Option<EventSubscription>>> = Rc::default();
    let seen = Rc::clone(&runs);
    let slot = Rc::clone(&own);
    let sub = Context::new()
        .on::<ProxyShutdownEvent>(EventPriority::Normal, move |_| {
            let _ = &token;
            bump(&seen);
            if let Some(sub) = slot.borrow_mut().take() {
                sub.cancel();
            }
        })
        .unwrap();
    *own.borrow_mut() = Some(sub);
    let listener = last_listener();

    let outcome = handle_event(listener, Event::ProxyShutdown);
    assert_eq!(outcome, EventOutcome::Unchanged);
    assert_eq!(runs.get(), 1);
    assert!(
        dropped.get(),
        "self-unsubscribed closure dropped, not reinserted"
    );
    assert!(!is_subscribed(listener), "host unsubscribed");

    handle_event(listener, Event::ProxyShutdown);
    assert_eq!(runs.get(), 1);
}

#[test]
fn event_handler_can_unsubscribe_another_handler_mid_dispatch() {
    let ctx = Context::new();
    let (b_token, b_dropped) = probe();
    let a_runs = counter();
    let b_runs = counter();
    let b_sub: Rc<RefCell<Option<EventSubscription>>> = Rc::default();

    let seen = Rc::clone(&a_runs);
    let victim = Rc::clone(&b_sub);
    ctx.on::<ProxyShutdownEvent>(EventPriority::First, move |_| {
        bump(&seen);
        if let Some(sub) = victim.borrow_mut().take() {
            sub.cancel();
        }
    })
    .unwrap();
    let a = last_listener();
    let seen = Rc::clone(&b_runs);
    *b_sub.borrow_mut() = Some(
        ctx.on::<ProxyShutdownEvent>(EventPriority::Last, move |_| {
            let _ = &b_token;
            bump(&seen);
        })
        .unwrap(),
    );
    let b = last_listener();

    handle_event(a, Event::ProxyShutdown);
    assert!(b_dropped.get(), "other handler dropped");
    assert!(!is_subscribed(b));
    handle_event(b, Event::ProxyShutdown);
    handle_event(a, Event::ProxyShutdown);
    assert_eq!((a_runs.get(), b_runs.get()), (2, 0));
}

#[test]
fn event_handler_can_subscribe_while_dispatching() {
    let runs = counter();
    let seen = Rc::clone(&runs);
    Context::new()
        .on::<ProxyShutdownEvent>(EventPriority::Normal, move |_| {
            let inner = Rc::clone(&seen);
            Context::new()
                .on::<ProxyShutdownEvent>(EventPriority::Normal, move |_| bump(&inner))
                .unwrap();
        })
        .unwrap();
    let outer = last_listener();
    handle_event(outer, Event::ProxyShutdown);
    let inner = last_listener();
    assert_ne!(outer, inner);
    handle_event(inner, Event::ProxyShutdown);
    assert_eq!(runs.get(), 1);
}

#[test]
fn event_of_another_kind_is_ignored() {
    let runs = counter();
    let seen = Rc::clone(&runs);
    Context::new()
        .on::<ProxyShutdownEvent>(EventPriority::Normal, move |_| bump(&seen))
        .unwrap();
    let outcome = handle_event(last_listener(), Event::ProxyInitialize);
    assert_eq!(outcome, EventOutcome::Unchanged);
    assert_eq!(runs.get(), 0);
}

#[test]
fn a_refused_subscription_keeps_no_handler() {
    refuse("subscribe");
    let (token, dropped) = probe();
    let refused = Context::new().on::<ProxyShutdownEvent>(EventPriority::Normal, move |_| {
        let _ = &token;
    });
    assert!(refused.is_err());
    assert!(dropped.get(), "the refused handler is dropped at once");
}

struct Noop;
impl LimboHandler for Noop {
    fn on_player_enter(&self, _session: &LimboSession) -> HandlerOutcome {
        HandlerOutcome::Accept
    }
}

struct Reregistering {
    calls: Rc<Cell<u32>>,
    registered_id: Rc<Cell<Option<u64>>>,
}
impl LimboHandler for Reregistering {
    fn on_player_enter(&self, _session: &LimboSession) -> HandlerOutcome {
        HandlerOutcome::Accept
    }
    fn on_disconnect(&self, _player: PlayerId) {
        bump(&self.calls);
        register_limbo_handler("again", Box::new(Noop));
        let id = host::with_fake(|h| h.limbo_handlers.last().map(|(_, id)| *id));
        self.registered_id.set(id);
    }
}

#[test]
fn limbo_callback_can_register_a_handler() {
    let calls = counter();
    let registered_id = Rc::new(Cell::new(None));
    register_limbo_handler(
        "gate",
        Box::new(Reregistering {
            calls: Rc::clone(&calls),
            registered_id: Rc::clone(&registered_id),
        }),
    );
    let id = host::with_fake(|h| h.limbo_handlers.first().map(|(_, id)| *id))
        .expect("limbo handler registered on the host");

    limbo_on_disconnect(id, 1);
    assert_eq!(calls.get(), 1);
    let new_id = registered_id.get().expect("callback ran re-registration");
    assert_ne!(id, new_id);

    limbo_on_disconnect(id, 1);
    assert_eq!(
        calls.get(),
        2,
        "handler still dispatchable after the nested registration"
    );
}

#[test]
fn a_refused_limbo_handler_is_dropped() {
    refuse("refused-gate");
    register_limbo_handler("refused-gate", Box::new(Noop));
    assert!(host::with_fake(|h| h.limbo_handlers.is_empty()));
    assert_eq!(LIMBO_HANDLERS.with(|handlers| handlers.len()), 0);
}

#[test]
fn codec_factories_get_sequential_ids_from_zero() {
    let mut registrar = CodecRegistrar { notify: true };
    registrar.add("first", crate::codec::FilterPriority::Normal, |_| {
        Box::new(PassthroughFilter)
    });
    registrar.add("second", crate::codec::FilterPriority::Late, |_| {
        Box::new(PassthroughFilter)
    });
    let declared = host::with_fake(|h| h.codec_filters.clone());
    assert_eq!(
        declared,
        [("first".to_owned(), 0), ("second".to_owned(), 1)]
    );
}

#[test]
fn a_refused_codec_filter_is_dropped_from_the_factory_table() {
    refuse("stolen");
    let mut registrar = CodecRegistrar { notify: true };
    registrar.add("stolen", crate::codec::FilterPriority::Normal, |_| {
        Box::new(PassthroughFilter)
    });
    assert!(host::with_fake(|h| h.codec_filters.is_empty()));
    assert_eq!(CODEC_FACTORIES.with(|factories| factories.len()), 0);
}

#[test]
fn unregistering_a_codec_filter_reports_what_the_host_answered() {
    let mut registrar = CodecRegistrar { notify: true };
    registrar.add("mine", crate::codec::FilterPriority::Normal, |_| {
        Box::new(PassthroughFilter)
    });
    assert_eq!(unregister_codec_filter("mine"), Ok(()));
    assert!(host::with_fake(|h| h.codec_filters.is_empty()));
    assert_eq!(
        unregister_codec_filter("mine").unwrap_err().kind(),
        ErrorKind::NotFound
    );
    refuse("theirs");
    assert_eq!(
        unregister_codec_filter("theirs").unwrap_err().kind(),
        ErrorKind::Conflict
    );
}

#[derive(Default)]
struct Lifecycle;

thread_local! {
    static SEEN: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

impl Plugin for Lifecycle {
    fn metadata(&self) -> PluginMetadata {
        PluginMetadata::new("lifecycle", "Lifecycle", "0.0.0")
    }

    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
        let seen = format!("{:?}", ctx.enable_reason());
        SEEN.with(|s| s.borrow_mut().push(seen));
        Err(Error::new(ErrorKind::PermissionDenied, "missing capability: ban").into())
    }

    fn on_disable(&self, ctx: &Context) -> Result<(), PluginError> {
        let seen = format!("{:?}", ctx.disable_reason());
        SEEN.with(|s| s.borrow_mut().push(seen));
        Ok(())
    }
}

#[test]
fn lifecycle_reasons_reach_the_plugin_and_errors_become_strings() {
    let failed = on_enable::<Lifecycle>(wg::EnableReason::Recovered(wg::RecoveryInfo {
        attempt: 1,
        cause: "trap".into(),
    }));
    assert_eq!(
        failed,
        Err("permission-denied: missing capability: ban".to_owned())
    );
    assert_eq!(on_disable(wg::DisableReason::Shutdown), Ok(()));
    SEEN.with(|seen| {
        assert_eq!(
            *seen.borrow(),
            [
                "Some(Recovered(RecoveryInfo { attempt: 1, cause: \"trap\" }))",
                "Some(Shutdown)"
            ]
        );
    });
}

fn named_event(name: &str, result: crate::bindings::events::NamedEventResult) -> Event {
    Event::NamedEvent(crate::bindings::events::NamedEventEvent {
        name: name.into(),
        source_plugin: "relay".into(),
        content_type: "text/plain".into(),
        payload: b"ping".to_vec(),
        result,
    })
}

fn open_result() -> crate::bindings::events::NamedEventResult {
    crate::bindings::events::NamedEventResult {
        cancelled: false,
        response: None,
    }
}

#[test]
fn a_named_listener_is_subscribed_by_name_and_answers_through_its_outcome() {
    let subscription = Context::new()
        .on_named("echo", EventPriority::Late, |event| {
            let text = event.text().unwrap_or_default().to_owned();
            event.respond_text(text.replace("ping", "pong"));
        })
        .expect("the host accepts the subscription");
    let listener = subscription.id();
    assert_eq!(
        host::with_fake(|h| h.named.get(&listener).cloned()).as_deref(),
        Some("echo")
    );

    let outcome = handle_event(listener, named_event("echo", open_result()));
    let EventOutcome::NamedEvent(result) = outcome else {
        panic!("a response answers named-event, got {outcome:?}");
    };
    assert!(!result.cancelled);
    assert_eq!(result.response.map(|r| r.payload), Some(b"pong".to_vec()));

    subscription.cancel();
    assert!(host::with_fake(|h| !h.named.contains_key(&listener)));
    assert_eq!(
        handle_event(listener, named_event("echo", open_result())),
        EventOutcome::Unchanged
    );
}

#[test]
fn fire_named_hands_the_bytes_to_the_host_and_returns_its_answer() {
    host::with_fake(|h| {
        h.answer = Some(crate::bindings::events::NamedEventResult {
            cancelled: true,
            response: Some(crate::bindings::events::NamedEventResponse {
                content_type: "application/json".into(),
                payload: b"{}".to_vec(),
            }),
        });
    });
    let outcome = Context::new()
        .fire_named_text("chat:relay", "hello")
        .expect("the host fires the event");
    assert!(outcome.cancelled);
    assert_eq!(
        outcome.response.map(|r| (r.content_type, r.payload)),
        Some(("application/json".to_owned(), b"{}".to_vec()))
    );
    assert_eq!(
        host::with_fake(|h| h.fired.clone()),
        [(
            "chat:relay".to_owned(),
            "text/plain".to_owned(),
            b"hello".to_vec()
        )]
    );

    refuse("fire-named");
    let refused = Context::new()
        .fire_named("x", "text/plain", b"")
        .expect_err("a refused fire is an error");
    assert_eq!(refused.kind(), ErrorKind::Conflict);
}

#[test]
fn a_packet_listener_sends_its_filters_and_can_drop_packets() {
    use crate::codec::ConnectionState;
    use crate::event::{PacketFilter, RawPacketEvent};
    use crate::types::PacketDirection;

    let subscription = Context::new()
        .on_packets(
            &[PacketFilter::serverbound(5, ConnectionState::Play)],
            EventPriority::Normal,
            |event: &mut RawPacketEvent| {
                if event.data.first() == Some(&0xff) {
                    event.drop_packet();
                }
            },
        )
        .expect("the host accepts the packet subscription");
    let listener = subscription.id();
    let filters = host::with_fake(|h| h.packets.get(&listener).cloned()).unwrap();
    assert_eq!(filters.len(), 1);
    assert_eq!(filters[0].packet_id, 5);
    assert_eq!(filters[0].direction, PacketDirection::Serverbound.to_wit());

    let packet = |first: u8| {
        Event::RawPacket(crate::bindings::events::RawPacketEvent {
            player: 1,
            direction: wt::PacketDirection::Serverbound,
            packet: wt::RawPacket {
                packet_id: 5,
                data: vec![first],
            },
            result: crate::bindings::events::RawPacketResult::Pass,
        })
    };
    assert_eq!(handle_event(listener, packet(1)), EventOutcome::Unchanged);
    assert_eq!(
        handle_event(listener, packet(0xff)),
        EventOutcome::RawPacket(crate::bindings::events::RawPacketResult::Drop)
    );

    refuse("subscribe-packets");
    assert!(
        Context::new()
            .on_packets(
                &[PacketFilter::clientbound(1, ConnectionState::Play)],
                EventPriority::Normal,
                |_: &mut RawPacketEvent| {}
            )
            .is_err()
    );
}

struct Guard {
    banned: &'static str,
    asked: Rc<Cell<u32>>,
}

fn guard_record(name: &str) -> crate::ban_provider::BanRecord {
    crate::ban_provider::BanRecord::new(
        "g1",
        BanTarget::Username(name.to_owned()),
        BanSource::Console,
    )
    .reason("griefing")
}

impl BanProvider for Guard {
    fn check(
        &self,
        attempt: &LoginAttempt,
    ) -> Result<Option<crate::ban_provider::BanVerdict>, PluginError> {
        bump(&self.asked);
        Ok((attempt.username.as_deref() == Some(self.banned)).then(|| {
            crate::ban_provider::BanVerdict::new(guard_record(self.banned))
                .message("you are banned")
        }))
    }

    fn ban(
        &self,
        request: BanRequest,
        source: BanSource,
    ) -> Result<crate::ban_provider::BanRecord, PluginError> {
        let mut record = crate::ban_provider::BanRecord::new("g2", request.target, source);
        record.reason = request.reason;
        Ok(record)
    }

    fn unban(
        &self,
        _request: UnbanRequest,
    ) -> Result<Option<crate::ban_provider::BanRecord>, PluginError> {
        Err(PluginError::from("storage offline"))
    }

    fn get(
        &self,
        target: &BanTarget,
    ) -> Result<Option<crate::ban_provider::BanRecord>, PluginError> {
        Ok((target == &BanTarget::Username(self.banned.to_owned()))
            .then(|| guard_record(self.banned)))
    }

    fn list(&self, query: &BanQuery) -> Result<crate::ban_provider::BanRecordPage, PluginError> {
        Ok(crate::ban_provider::BanRecordPage::new(
            vec![guard_record(self.banned)],
            query.cursor.clone(),
        ))
    }

    fn features(&self) -> crate::ban_provider::BanFeatures {
        crate::ban_provider::BanFeatures::new().ip_ranges(true)
    }
}

fn attempt(username: &str) -> wb::LoginAttempt {
    wb::LoginAttempt {
        stage: wb::LoginStage::PreAuth,
        ip: wt::IpAddress::Ipv4((203, 0, 113, 7)),
        username: Some(username.to_owned()),
        uuid: None,
        uuid_verified: false,
        virtual_host: Some("play.example.com".into()),
        server: None,
    }
}

#[test]
fn without_a_provider_the_ban_exports_answer_an_error() {
    assert_eq!(
        ban_provider_check(attempt("Steve")),
        Err(NO_BAN_PROVIDER.to_owned())
    );
    assert_eq!(
        ban_provider_get(wb::BanTarget::Username("Steve".into())),
        Err(NO_BAN_PROVIDER.to_owned())
    );
}

#[test]
fn a_ban_provider_is_registered_with_its_features_and_answers_the_exports() {
    let asked = counter();
    Context::new()
        .provide_bans(Guard {
            banned: "Griefer",
            asked: Rc::clone(&asked),
        })
        .unwrap();
    assert_eq!(
        host::with_fake(|h| h.ban_providers.clone()),
        [wb::BanFeatures {
            ip_ranges: true,
            pagination: false,
        }]
    );

    assert_eq!(ban_provider_check(attempt("Steve")), Ok(None));
    let verdict = ban_provider_check(attempt("Griefer"))
        .unwrap()
        .expect("the griefer is banned");
    assert_eq!(verdict.entry.id, "g1");
    assert_eq!(
        verdict.kick_message,
        Some(Component::text("you are banned").to_arena())
    );
    assert_eq!(asked.get(), 2);

    let record = ban_provider_ban(
        wb::BanRequest {
            target: wb::BanTarget::Username("Alex".into()),
            reason: Some("spam".into()),
            duration_ms: None,
            kick: true,
            silent: false,
        },
        wb::BanSource::Plugin("web".into()),
    )
    .unwrap();
    assert_eq!(record.source, wb::BanSource::Plugin("web".into()));
    assert_eq!(record.reason.as_deref(), Some("spam"));
    assert_eq!(
        ban_provider_unban(wb::UnbanRequest {
            target: wb::BanTarget::Username("Alex".into()),
            source: wb::BanSource::Console,
            silent: false,
        }),
        Err("storage offline".to_owned())
    );
    let page = ban_provider_list(wb::BanQuery {
        cursor: Some("next".into()),
        limit: 10,
    })
    .unwrap();
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.next_cursor.as_deref(), Some("next"));
}

#[test]
fn a_refused_ban_provider_is_not_kept() {
    refuse("ban-provider");
    let refused = Context::new().provide_bans(Guard {
        banned: "Griefer",
        asked: counter(),
    });
    assert_eq!(refused.unwrap_err().kind(), ErrorKind::Conflict);
    assert_eq!(
        ban_provider_check(attempt("Griefer")),
        Err(NO_BAN_PROVIDER.to_owned())
    );
}

struct Grants;

impl PermissionProvider for Grants {
    fn snapshot_for(&self, subject: &PermissionSubject) -> PermissionSnapshot {
        match subject.profile() {
            Some(profile) if profile.username == "Steve" => {
                PermissionSnapshot::new().grant("demo.use")
            }
            Some(_) => PermissionSnapshot::new(),
            None => PermissionSnapshot::admin(),
        }
    }
}

fn player_subject(username: &str) -> wp::PermissionSubject {
    wp::PermissionSubject::Player(wp::PlayerSubject {
        id: 1,
        profile: wt::GameProfile {
            uuid: wt::Uuid { hi: 0, lo: 1 },
            username: username.to_owned(),
            properties: vec![],
        },
        online_mode: false,
        virtual_host: None,
        remote_addr: wt::SocketAddress {
            ip: wt::IpAddress::Ipv4((127, 0, 0, 1)),
            port: 1,
        },
    })
}

#[test]
fn a_permission_provider_answers_snapshots_and_none_means_empty() {
    assert_eq!(
        permission_snapshot_for(player_subject("Steve")),
        PermissionSnapshot::new().to_wit()
    );
    Context::new().provide_permissions(Grants).unwrap();
    assert_eq!(host::with_fake(|h| h.permission_providers), 1);
    assert_eq!(
        permission_snapshot_for(player_subject("Steve")),
        PermissionSnapshot::new().grant("demo.use").to_wit()
    );
    assert_eq!(
        permission_snapshot_for(player_subject("Alex")),
        PermissionSnapshot::new().to_wit()
    );
    assert!(permission_snapshot_for(wp::PermissionSubject::Console).admin);
}

#[test]
fn set_snapshot_and_release_reach_the_host() {
    let snapshot = PermissionSnapshot::new().grant("demo.use");
    crate::permissions::Permissions::set_snapshot(PlayerId::new(4), &snapshot).unwrap();
    assert_eq!(
        host::with_fake(|h| h.snapshots.get(&4).cloned()),
        Some(snapshot.to_wit())
    );
    crate::permissions::Permissions::release(PlayerId::new(4)).unwrap();
    assert!(host::with_fake(|h| h.snapshots.is_empty()));
    refuse("set-snapshot");
    assert!(crate::permissions::Permissions::set_snapshot(PlayerId::new(4), &snapshot).is_err());
}

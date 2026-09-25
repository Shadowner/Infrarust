use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use super::*;
use crate::context::{EventSubscription, TaskHandle};
use crate::event::ProxyShutdownEvent;
use crate::host;
use crate::limbo::{HandlerOutcome, LimboHandler, LimboSession};

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

#[test]
fn delay_closure_is_dropped_after_firing() {
    let ctx = Context::new();
    let (token, dropped) = probe();
    let runs = counter();
    let seen = Rc::clone(&runs);
    ctx.delay(Duration::from_millis(5), move || {
        let _ = &token;
        bump(&seen);
    });
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
    let handle = ctx.delay(Duration::from_secs(60), move || {
        let _ = &token;
        bump(&seen);
    });
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
    let handle = ctx.interval(Duration::from_millis(10), move || {
        let _ = &token;
        bump(&seen);
    });
    let (host_handle, callback) = last_task();
    fire(callback);
    fire(callback);
    assert_eq!(runs.get(), 2, "interval keeps firing");
    assert!(!dropped.get());

    ctx.cancel(handle);
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
    let handle = ctx.interval(Duration::from_millis(10), move || {
        let _ = &token;
        bump(&seen);
        if let Some(handle) = slot.get() {
            Context::new().cancel(handle);
        }
    });
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
        Context::new().delay(Duration::from_millis(1), move || bump(&again));
    });
    let (_, first) = last_task();
    fire(first);
    let (_, second) = last_task();
    assert_ne!(first, second);
    fire(second);
    assert_eq!(runs.get(), 2);
}

#[test]
fn completer_can_register_a_command_with_a_completer() {
    let ctx = Context::new();
    ctx.command("outer", |_| {})
        .completer(|_, _| {
            Context::new()
                .command("inner", |_| {})
                .completer(|_, _| vec!["deep".to_owned()])
                .register();
            vec!["registered".to_owned()]
        })
        .register();

    let outer = command_callback("outer");
    assert_eq!(tab_complete(outer, Vec::new(), 0), ["registered"]);
    let inner = command_callback("inner");
    assert_eq!(tab_complete(inner, Vec::new(), 0), ["deep"]);
    assert_eq!(tab_complete(outer, Vec::new(), 0), ["registered"]);
}

#[test]
fn tab_complete_without_a_completer_is_empty() {
    Context::new().command("bare", |_| {}).register();
    assert!(tab_complete(command_callback("bare"), vec!["x".to_owned()], 1).is_empty());
    assert!(tab_complete(u64::MAX, Vec::new(), 0).is_empty());
}

#[test]
fn command_handler_receives_the_invocation() {
    let seen: Rc<RefCell<Vec<Invocation>>> = Rc::default();
    let sink = Rc::clone(&seen);
    Context::new()
        .command("echo", move |inv| {
            sink.borrow_mut().push((inv.args, inv.player))
        })
        .register();
    let callback = command_callback("echo");
    handle_command(callback, vec!["a".to_owned(), "b".to_owned()], Some(4));
    handle_command(callback, Vec::new(), None);
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
    Context::new()
        .command("spawn", move |_| {
            let inner = Rc::clone(&seen);
            Context::new()
                .command("spawned", move |_| bump(&inner))
                .register();
        })
        .register();
    handle_command(command_callback("spawn"), Vec::new(), None);
    handle_command(command_callback("spawned"), Vec::new(), None);
    assert_eq!(runs.get(), 1);
}

#[test]
fn reregistering_a_command_name_drops_the_replaced_closure() {
    let ctx = Context::new();
    let (first_token, first_dropped) = probe();
    let first_runs = counter();
    let second_runs = counter();
    let seen = Rc::clone(&first_runs);
    ctx.command("dup", move |_| {
        let _ = &first_token;
        bump(&seen);
    })
    .register();
    let first = command_callback("dup");
    let seen = Rc::clone(&second_runs);
    ctx.command("DUP", move |_| bump(&seen)).register();
    let second = command_callback("dup");

    assert_ne!(first, second);
    assert!(first_dropped.get(), "replaced command closure dropped");
    handle_command(first, Vec::new(), None);
    handle_command(second, Vec::new(), None);
    assert_eq!((first_runs.get(), second_runs.get()), (0, 1));
}

#[test]
fn cancel_after_a_delay_fired_is_a_guest_side_no_op() {
    let ctx = Context::new();
    let handle = ctx.delay(Duration::from_millis(1), || {});
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
    ctx.command("bye", move |_| {
        let _ = &token;
        bump(&seen);
    })
    .completer(|_, _| vec!["now".to_owned()])
    .register();
    let callback = command_callback("bye");

    assert!(ctx.unregister_command("Bye"));
    assert!(dropped.get(), "unregistered command closure dropped");
    assert!(host::with_fake(|h| !h.commands.contains_key("bye")));
    handle_command(callback, Vec::new(), None);
    assert_eq!(runs.get(), 0);
    assert!(tab_complete(callback, Vec::new(), 0).is_empty());
    assert!(
        !ctx.unregister_command("bye"),
        "second unregister finds nothing"
    );
}

#[test]
fn unregister_leaves_commands_this_plugin_does_not_own_alone() {
    host::with_fake(|h| h.commands.insert("foreign".to_owned(), 77));
    assert!(!Context::new().unregister_command("foreign"));
    assert!(host::with_fake(|h| h.commands.contains_key("foreign")));
}

#[test]
fn command_unregistering_itself_is_dropped_after_its_call() {
    let (token, dropped) = probe();
    let outcome: Rc<Cell<Option<(bool, bool)>>> = Rc::default();
    let seen = Rc::clone(&outcome);
    let flag = Rc::clone(&dropped);
    Context::new()
        .command("once", move |_| {
            let _ = &token;
            let removed = Context::new().unregister_command("once");
            seen.set(Some((removed, flag.get())));
        })
        .register();
    let callback = command_callback("once");

    handle_command(callback, Vec::new(), None);
    assert_eq!(
        outcome.get(),
        Some((true, false)),
        "unregistered from inside, still alive while running"
    );
    assert!(dropped.get(), "dropped once the in-flight call returned");
    handle_command(callback, Vec::new(), None);
    assert_eq!(outcome.get(), Some((true, false)));
}

#[test]
fn command_can_unregister_another_command_mid_dispatch() {
    let ctx = Context::new();
    let (token, dropped) = probe();
    let victim_runs = counter();
    let seen = Rc::clone(&victim_runs);
    ctx.command("victim", move |_| {
        let _ = &token;
        bump(&seen);
    })
    .register();
    ctx.command("reaper", |_| {
        assert!(Context::new().unregister_command("victim"));
    })
    .register();
    let victim = command_callback("victim");

    handle_command(command_callback("reaper"), Vec::new(), None);
    assert!(dropped.get());
    handle_command(victim, Vec::new(), None);
    assert_eq!(victim_runs.get(), 0);
}

#[test]
fn completer_can_unregister_its_own_command() {
    let (token, dropped) = probe();
    Context::new()
        .command("fleeting", |_| {})
        .completer(move |_, _| {
            let _ = &token;
            let removed = Context::new().unregister_command("fleeting");
            vec![removed.to_string()]
        })
        .register();
    let callback = command_callback("fleeting");

    assert_eq!(tab_complete(callback, Vec::new(), 0), ["true"]);
    assert!(dropped.get());
    assert!(tab_complete(callback, Vec::new(), 0).is_empty());
}

#[test]
fn command_reregistering_its_own_name_mid_dispatch_swaps_the_handler() {
    let (token, dropped) = probe();
    let second_runs = counter();
    let seen = Rc::clone(&second_runs);
    Context::new()
        .command("phoenix", move |_| {
            let _ = &token;
            let again = Rc::clone(&seen);
            Context::new()
                .command("phoenix", move |_| bump(&again))
                .register();
        })
        .register();
    let first = command_callback("phoenix");

    handle_command(first, Vec::new(), None);
    assert!(dropped.get(), "replaced handler dropped after its call");
    let second = command_callback("phoenix");
    assert_ne!(first, second);
    handle_command(second, Vec::new(), None);
    assert_eq!(second_runs.get(), 1);
}

#[test]
fn reentrant_dispatch_of_a_running_event_handler_is_skipped() {
    let runs = counter();
    let own = Rc::new(Cell::new(0));
    let seen = Rc::clone(&runs);
    let listener_slot = Rc::clone(&own);
    Context::new().on::<ProxyShutdownEvent>(EventPriority::Normal, move |_| {
        bump(&seen);
        let nested = handle_event(listener_slot.get(), Event::ProxyShutdown);
        assert!(matches!(nested, EventOutcome::None));
    });
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
    let sub = Context::new().on::<ProxyShutdownEvent>(EventPriority::Normal, move |_| {
        let _ = &token;
        bump(&seen);
        if let Some(sub) = slot.borrow_mut().take() {
            sub.cancel();
        }
    });
    *own.borrow_mut() = Some(sub);
    let listener = last_listener();

    let outcome = handle_event(listener, Event::ProxyShutdown);
    assert!(matches!(outcome, EventOutcome::None));
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
    });
    let a = last_listener();
    let seen = Rc::clone(&b_runs);
    *b_sub.borrow_mut() = Some(ctx.on::<ProxyShutdownEvent>(EventPriority::Last, move |_| {
        let _ = &b_token;
        bump(&seen);
    }));
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
    Context::new().on::<ProxyShutdownEvent>(EventPriority::Normal, move |_| {
        let inner = Rc::clone(&seen);
        Context::new().on::<ProxyShutdownEvent>(EventPriority::Normal, move |_| bump(&inner));
    });
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
    Context::new().on::<ProxyShutdownEvent>(EventPriority::Normal, move |_| bump(&seen));
    let outcome = handle_event(last_listener(), Event::ConfigReload);
    assert!(matches!(outcome, EventOutcome::None));
    assert_eq!(runs.get(), 0);
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
    fn on_disconnect(&self, _player_id: u64) {
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

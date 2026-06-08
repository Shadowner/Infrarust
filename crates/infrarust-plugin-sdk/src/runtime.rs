//! Dispatch glue and the per-instance registries the macro-generated `Guest`
//! impl delegates to. The guest is single-threaded, so `thread_local!` +
//! `RefCell` is the cheapest correct storage.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use crate::bindings::codec_filter::{
    CodecContext, CodecSessionInit, ConnectionState, FilterOutput, GuestFilterInstance, RawPacket,
};
use crate::bindings::guest::{Event, EventOutcome};
use crate::codec::{
    CodecFilter, CodecRegistrar, FilterConstructor, Injections, Packet, Verdict, build_filter_output,
    packet_from_wit,
};
use crate::context::{CommandInvocation, Context};
use crate::event::{EventPriority, GuestEvent};
use crate::limbo::{HandlerOutcome, LimboHandler, LimboRegistrar, LimboSession};
use crate::plugin::Plugin;

type EventClosure = Box<dyn FnMut(Event) -> EventOutcome>;
type CommandClosure = Box<dyn FnMut(CommandInvocation)>;
type CompletionClosure = Box<dyn Fn(&[String], u32) -> Vec<String>>;
type TaskClosure = Box<dyn FnMut()>;

thread_local! {
    static EVENT_HANDLERS: RefCell<HashMap<u64, EventClosure>> = RefCell::new(HashMap::new());
    static COMMANDS: RefCell<HashMap<u64, CommandClosure>> = RefCell::new(HashMap::new());
    static COMPLETIONS: RefCell<HashMap<u64, CompletionClosure>> = RefCell::new(HashMap::new());
    static TASKS: RefCell<HashMap<u64, TaskClosure>> = RefCell::new(HashMap::new());
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
    static PLUGIN: RefCell<Option<Box<dyn Plugin>>> = const { RefCell::new(None) };
    static CODEC_FACTORIES: RefCell<Vec<FilterConstructor>> = RefCell::new(Vec::new());
    static CODEC_DECLARED: Cell<bool> = const { Cell::new(false) };
    static LIMBO_HANDLERS: RefCell<HashMap<u64, Box<dyn LimboHandler>>> = RefCell::new(HashMap::new());
    static LIMBO_DECLARED: Cell<bool> = const { Cell::new(false) };
}

fn next_id() -> u64 {
    NEXT_ID.with(|c| {
        let id = c.get();
        c.set(id + 1);
        id
    })
}

/// Subscribe a typed handler for `E`, returning its `listener_id`. Each call adds
/// an independent native listener, so multiple handlers may share a kind and the
/// host routes every fire to the exact closure by its id.
pub fn register_event<E: GuestEvent>(
    priority: EventPriority,
    mut handler: impl FnMut(&mut E) + 'static,
) -> u64 {
    let closure: EventClosure = Box::new(move |ev| match E::from_event(ev) {
        Some(mut typed) => {
            handler(&mut typed);
            typed.into_outcome()
        }
        None => EventOutcome::None,
    });
    let handle = crate::bindings::event_bus::subscribe(E::KIND, priority.value());
    EVENT_HANDLERS.with(|hs| hs.borrow_mut().insert(handle, closure));
    handle
}

/// Drop a single event handler by its `listener_id` (see [`register_event`]).
pub fn unsubscribe_event(handle: u64) {
    EVENT_HANDLERS.with(|hs| hs.borrow_mut().remove(&handle));
    crate::bindings::event_bus::unsubscribe(handle);
}

pub fn register_command(
    name: &str,
    aliases: &[String],
    description: &str,
    handler: CommandClosure,
    completer: Option<CompletionClosure>,
) {
    let id = next_id();
    COMMANDS.with(|c| c.borrow_mut().insert(id, handler));
    if let Some(completer) = completer {
        COMPLETIONS.with(|c| c.borrow_mut().insert(id, completer));
    }
    crate::bindings::command_manager::register(name, aliases, description, id);
}

pub fn schedule_delay(after_ms: u64, task: TaskClosure) -> u64 {
    let id = next_id();
    TASKS.with(|t| t.borrow_mut().insert(id, task));
    crate::bindings::scheduler::delay(after_ms, id)
}

pub fn schedule_interval(period_ms: u64, task: TaskClosure) -> u64 {
    let id = next_id();
    TASKS.with(|t| t.borrow_mut().insert(id, task));
    crate::bindings::scheduler::interval(period_ms, id)
}

pub fn handle_event(listener: u64, ev: Event) -> EventOutcome {
    let closure = EVENT_HANDLERS.with(|hs| hs.borrow_mut().remove(&listener));
    match closure {
        Some(mut closure) => {
            let outcome = closure(ev);
            EVENT_HANDLERS.with(|hs| {
                hs.borrow_mut().entry(listener).or_insert(closure);
            });
            outcome
        }
        None => EventOutcome::None,
    }
}

pub fn handle_command(callback_id: u64, args: Vec<String>, player: Option<u64>) {
    let task = COMMANDS.with(|c| c.borrow_mut().remove(&callback_id));
    if let Some(mut handler) = task {
        handler(CommandInvocation { args, player });
        COMMANDS.with(|c| {
            c.borrow_mut().entry(callback_id).or_insert(handler);
        });
    }
}

pub fn on_scheduled_task(callback_id: u64) {
    let task = TASKS.with(|t| t.borrow_mut().remove(&callback_id));
    if let Some(mut task) = task {
        task();
        TASKS.with(|t| {
            t.borrow_mut().entry(callback_id).or_insert(task);
        });
    }
}

pub fn tab_complete(callback_id: u64, partial: Vec<String>, cursor: u32) -> Vec<String> {
    COMPLETIONS.with(|c| {
        c.borrow()
            .get(&callback_id)
            .map_or_else(Vec::new, |f| f(&partial, cursor))
    })
}

pub fn on_enable<P: Plugin + Default>() -> Result<(), String> {
    let plugin = P::default();
    let result = plugin.on_enable(&Context::new());
    if result.is_ok() {
        declare_codec_filters::<P>(true);
        declare_limbo_handlers::<P>();
    }
    PLUGIN.with(|p| *p.borrow_mut() = Some(Box::new(plugin)));
    result
}

pub fn on_disable() -> Result<(), String> {
    let plugin = PLUGIN.with(|p| p.borrow_mut().take());
    match plugin {
        Some(plugin) => plugin.on_disable(&Context::new()),
        None => Ok(()),
    }
}

pub fn register_codec_factory(
    notify: bool,
    metadata: crate::bindings::codec_registry::CodecFilterMetadata,
    constructor: FilterConstructor,
) {
    let id = CODEC_FACTORIES.with(|f| {
        let mut f = f.borrow_mut();
        let id = u64::try_from(f.len()).unwrap_or(u64::MAX);
        f.push(constructor);
        id
    });
    if notify {
        crate::bindings::codec_registry::register_codec_filter(&metadata, id);
    }
}
fn declare_codec_filters<P: Plugin>(notify: bool) {
    if CODEC_DECLARED.with(Cell::get) {
        return;
    }
    let mut registrar = CodecRegistrar { notify };
    P::register_codec_filters(&mut registrar);
    CODEC_DECLARED.with(|c| c.set(true));
}

pub fn register_limbo_handler(name: &str, handler: Box<dyn LimboHandler>) {
    let id = next_id();
    LIMBO_HANDLERS.with(|h| h.borrow_mut().insert(id, handler));
    crate::bindings::limbo::register_limbo_handler(name, id);
}

fn declare_limbo_handlers<P: Plugin>() {
    if LIMBO_DECLARED.with(Cell::get) {
        return;
    }
    let mut registrar = LimboRegistrar::new();
    P::register_limbo_handlers(&mut registrar);
    LIMBO_DECLARED.with(|c| c.set(true));
}

pub fn limbo_on_player_enter(
    handler: u64,
    session: &crate::bindings::guest::LimboSession,
) -> crate::bindings::guest::HandlerResult {
    LIMBO_HANDLERS.with(|h| match h.borrow().get(&handler) {
        Some(hdlr) => hdlr.on_player_enter(&LimboSession::new(session)).into_wit(),
        None => HandlerOutcome::Accept.into_wit(),
    })
}

pub fn limbo_on_command(
    handler: u64,
    session: &crate::bindings::guest::LimboSession,
    command: String,
    args: Vec<String>,
) {
    LIMBO_HANDLERS.with(|h| {
        if let Some(hdlr) = h.borrow().get(&handler) {
            hdlr.on_command(&LimboSession::new(session), &command, &args);
        }
    });
}

pub fn limbo_on_chat(handler: u64, session: &crate::bindings::guest::LimboSession, message: String) {
    LIMBO_HANDLERS.with(|h| {
        if let Some(hdlr) = h.borrow().get(&handler) {
            hdlr.on_chat(&LimboSession::new(session), &message);
        }
    });
}

pub fn limbo_on_disconnect(handler: u64, player: u64) {
    LIMBO_HANDLERS.with(|h| {
        if let Some(hdlr) = h.borrow().get(&handler) {
            hdlr.on_disconnect(player);
        }
    });
}
pub fn create_codec_filter<P: Plugin>(factory: u64, init: CodecSessionInit) -> FilterInstanceProxy {
    declare_codec_filters::<P>(false);
    let inner = CODEC_FACTORIES.with(|f| {
        f.borrow()
            .get(usize::try_from(factory).unwrap_or(usize::MAX))
            .map(|constructor| constructor(&init))
    });
    FilterInstanceProxy::new(inner)
}

pub struct FilterInstanceProxy {
    inner: RefCell<Box<dyn CodecFilter>>,
}

impl FilterInstanceProxy {
    fn new(inner: Option<Box<dyn CodecFilter>>) -> Self {
        Self {
            inner: RefCell::new(inner.unwrap_or_else(|| Box::new(PassthroughFilter))),
        }
    }
}

impl GuestFilterInstance for FilterInstanceProxy {
    fn filter(&self, ctx: CodecContext, packet: RawPacket) -> FilterOutput {
        let mut inner = self.inner.borrow_mut();
        let mut packet = packet_from_wit(packet);
        let mut injections = Injections::default();
        let verdict = inner.filter(&ctx, &mut packet, &mut injections);
        build_filter_output(verdict, packet, injections)
    }
    fn on_state_change(&self, new_state: ConnectionState) {
        self.inner.borrow_mut().on_state_change(new_state);
    }
    fn on_compression_change(&self, threshold: i32) {
        self.inner.borrow_mut().on_compression_change(threshold);
    }
    fn on_encryption_enabled(&self) {
        self.inner.borrow_mut().on_encryption_enabled();
    }
    fn on_close(&self) {
        self.inner.borrow_mut().on_close();
    }
}

struct PassthroughFilter;

impl CodecFilter for PassthroughFilter {
    fn filter(&mut self, _ctx: &CodecContext, _packet: &mut Packet, _out: &mut Injections) -> Verdict {
        Verdict::Pass
    }
}

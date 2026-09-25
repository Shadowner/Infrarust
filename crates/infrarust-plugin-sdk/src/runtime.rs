//! Dispatch glue and the per-instance registries the macro-generated `Guest`
//! impl delegates to. The guest is single-threaded, so `thread_local!` +
//! `RefCell` is the cheapest correct storage.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::thread::LocalKey;

use crate::bindings::codec_filter::{
    CodecSessionInit, ConnectionState, FilterOutput, GuestFilterInstance,
};
use crate::bindings::guest::{Event, EventOutcome};
use crate::codec::{
    CodecContext, CodecFilter, CodecRegistrar, FilterConstructor, Injections, Packet, Verdict,
    build_filter_output,
};
use crate::context::{CommandInvocation, Context};
use crate::event::{EventPriority, GuestEvent};
use crate::limbo::{HandlerOutcome, LimboHandler, LimboRegistrar, LimboSession};
use crate::plugin::Plugin;
use crate::registry::Registry;

type EventEntry = RefCell<dyn FnMut(Event) -> EventOutcome>;
type CommandClosure = Box<dyn FnMut(CommandInvocation)>;
type CompletionClosure = Box<dyn Fn(&[String], u32) -> Vec<String>>;
type OnceTask = Box<dyn FnOnce()>;
type RepeatingTask = Box<dyn FnMut()>;
type CodecConstructor = dyn Fn(&CodecSessionInit) -> Box<dyn CodecFilter>;

struct CommandEntry {
    name: String,
    handler: RefCell<CommandClosure>,
    completer: Option<CompletionClosure>,
}

enum Task {
    Once(Cell<Option<OnceTask>>),
    Repeating(RefCell<RepeatingTask>),
}

struct TaskEntry {
    host_handle: u64,
    task: Task,
}

thread_local! {
    static EVENTS: Registry<EventEntry> = const { Registry::new() };
    static COMMANDS: Registry<CommandEntry> = const { Registry::new() };
    static TASKS: Registry<TaskEntry> = const { Registry::new() };
    static LIMBO_HANDLERS: Registry<dyn LimboHandler> = const { Registry::new() };
    static CODEC_FACTORIES: Registry<CodecConstructor> = const { Registry::new() };
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
    static NEXT_CODEC_FACTORY: Cell<u64> = const { Cell::new(0) };
    static PLUGIN: RefCell<Option<Box<dyn Plugin>>> = const { RefCell::new(None) };
    static CODEC_DECLARED: Cell<bool> = const { Cell::new(false) };
    static LIMBO_DECLARED: Cell<bool> = const { Cell::new(false) };
}

fn take_id(counter: &'static LocalKey<Cell<u64>>) -> u64 {
    counter.with(|c| {
        let id = c.get();
        c.set(id + 1);
        id
    })
}

fn next_id() -> u64 {
    take_id(&NEXT_ID)
}

/// Subscribe a typed handler for `E`, returning its `listener_id`. Each call adds
/// an independent native listener, so multiple handlers may share a kind and the
/// host routes every fire to the exact closure by its id.
pub fn register_event<E: GuestEvent>(
    priority: EventPriority,
    mut handler: impl FnMut(&mut E) + 'static,
) -> u64 {
    let entry: Rc<EventEntry> = Rc::new(RefCell::new(move |ev: Event| match E::from_event(ev) {
        Some(mut typed) => {
            handler(&mut typed);
            typed.into_outcome()
        }
        None => EventOutcome::None,
    }));
    let listener = crate::host::subscribe(E::KIND, priority.value());
    EVENTS.with(|events| events.insert(listener, entry));
    listener
}

/// Drop a single event handler by its `listener_id` (see [`register_event`]).
pub fn unsubscribe_event(listener: u64) {
    let removed = EVENTS.with(|events| events.remove(listener));
    crate::host::unsubscribe(listener);
    drop(removed);
}

pub fn handle_event(listener: u64, ev: Event) -> EventOutcome {
    let Some(entry) = EVENTS.with(|events| events.get(listener)) else {
        return EventOutcome::None;
    };
    let Ok(mut handler) = entry.try_borrow_mut() else {
        return EventOutcome::None;
    };
    handler(ev)
}

pub fn register_command(
    name: &str,
    aliases: &[String],
    description: &str,
    handler: CommandClosure,
    completer: Option<CompletionClosure>,
) {
    let key = name.to_lowercase();
    let id = next_id();
    let replaced = COMMANDS.with(|commands| {
        let previous = commands.find(|entry| entry.name == key);
        commands.insert(
            id,
            Rc::new(CommandEntry {
                name: key,
                handler: RefCell::new(handler),
                completer,
            }),
        );
        previous.and_then(|previous| commands.remove(previous))
    });
    crate::host::register_command(name, aliases, description, id);
    drop(replaced);
}

pub fn unregister_command(name: &str) -> bool {
    let key = name.to_lowercase();
    let removed = COMMANDS.with(|commands| {
        commands
            .find(|entry| entry.name == key)
            .and_then(|id| commands.remove(id))
    });
    let Some(removed) = removed else {
        return false;
    };
    crate::host::unregister_command(&key);
    drop(removed);
    true
}

pub fn handle_command(callback_id: u64, args: Vec<String>, player: Option<u64>) {
    let Some(entry) = COMMANDS.with(|commands| commands.get(callback_id)) else {
        return;
    };
    if let Ok(mut handler) = entry.handler.try_borrow_mut() {
        handler(CommandInvocation { args, player });
    }
}

pub fn tab_complete(callback_id: u64, partial: Vec<String>, cursor: u32) -> Vec<String> {
    let Some(entry) = COMMANDS.with(|commands| commands.get(callback_id)) else {
        return Vec::new();
    };
    entry
        .completer
        .as_ref()
        .map_or_else(Vec::new, |complete| complete(&partial, cursor))
}

pub fn schedule_delay(after_ms: u64, task: OnceTask) -> u64 {
    schedule(Task::Once(Cell::new(Some(task))), |id| {
        crate::host::delay(after_ms, id)
    })
}

pub fn schedule_interval(period_ms: u64, task: RepeatingTask) -> u64 {
    schedule(Task::Repeating(RefCell::new(task)), |id| {
        crate::host::interval(period_ms, id)
    })
}

fn schedule(task: Task, start_on_host: impl FnOnce(u64) -> u64) -> u64 {
    let id = next_id();
    let host_handle = start_on_host(id);
    TASKS.with(|tasks| tasks.insert(id, Rc::new(TaskEntry { host_handle, task })));
    id
}

pub fn cancel_task(id: u64) {
    let Some(removed) = TASKS.with(|tasks| tasks.remove(id)) else {
        return;
    };
    crate::host::cancel(removed.host_handle);
}

pub fn on_scheduled_task(callback_id: u64) {
    let Some(entry) = TASKS.with(|tasks| tasks.get(callback_id)) else {
        return;
    };
    match &entry.task {
        Task::Once(slot) => {
            TASKS.with(|tasks| tasks.remove(callback_id));
            if let Some(task) = slot.take() {
                task();
            }
        }
        Task::Repeating(task) => {
            if let Ok(mut task) = task.try_borrow_mut() {
                task();
            }
        }
    }
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
    let id = take_id(&NEXT_CODEC_FACTORY);
    CODEC_FACTORIES.with(|factories| factories.insert(id, Rc::from(constructor)));
    if notify {
        crate::host::register_codec_filter(&metadata, id);
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
    LIMBO_HANDLERS.with(|handlers| handlers.insert(id, Rc::from(handler)));
    crate::host::register_limbo_handler(name, id);
}

fn with_limbo_handler<R>(
    handler: u64,
    missing: impl FnOnce() -> R,
    call: impl FnOnce(&dyn LimboHandler) -> R,
) -> R {
    let Some(entry) = LIMBO_HANDLERS.with(|handlers| handlers.get(handler)) else {
        return missing();
    };
    call(&*entry)
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
    with_limbo_handler(
        handler,
        || HandlerOutcome::Accept.into_wit(),
        |hdlr| hdlr.on_player_enter(&LimboSession::new(session)).into_wit(),
    )
}

pub fn limbo_on_command(
    handler: u64,
    session: &crate::bindings::guest::LimboSession,
    command: String,
    args: Vec<String>,
) {
    with_limbo_handler(
        handler,
        || (),
        |hdlr| hdlr.on_command(&LimboSession::new(session), &command, &args),
    );
}

pub fn limbo_on_chat(
    handler: u64,
    session: &crate::bindings::guest::LimboSession,
    message: String,
) {
    with_limbo_handler(
        handler,
        || (),
        |hdlr| hdlr.on_chat(&LimboSession::new(session), &message),
    );
}

pub fn limbo_on_disconnect(handler: u64, player: u64) {
    with_limbo_handler(handler, || (), |hdlr| hdlr.on_disconnect(player));
}

pub fn limbo_on_session_end(
    handler: u64,
    player: u64,
    reason: crate::bindings::guest::SessionEndReason,
) {
    with_limbo_handler(
        handler,
        || (),
        |hdlr| hdlr.on_session_end(player, crate::limbo::SessionEndReason::from_wit(reason)),
    );
}
pub fn create_codec_filter<P: Plugin>(factory: u64, init: CodecSessionInit) -> FilterInstanceProxy {
    declare_codec_filters::<P>(false);
    let inner = CODEC_FACTORIES
        .with(|factories| factories.get(factory))
        .map(|construct| construct(&init));
    FilterInstanceProxy::new(inner, CodecContext::from_init(&init))
}

pub struct FilterInstanceProxy {
    inner: RefCell<Box<dyn CodecFilter>>,
    ctx: RefCell<CodecContext>,
}

impl FilterInstanceProxy {
    fn new(inner: Option<Box<dyn CodecFilter>>, ctx: CodecContext) -> Self {
        Self {
            inner: RefCell::new(inner.unwrap_or_else(|| Box::new(PassthroughFilter))),
            ctx: RefCell::new(ctx),
        }
    }
}

impl GuestFilterInstance for FilterInstanceProxy {
    fn filter(&self, packet_id: i32, data: Vec<u8>) -> FilterOutput {
        let mut inner = self.inner.borrow_mut();
        let mut packet = Packet::from_parts(packet_id, data);
        let mut injections = Injections::default();
        let verdict = inner.filter(&self.ctx.borrow(), &mut packet, &mut injections);
        build_filter_output(verdict, packet, injections)
    }
    fn on_state_change(&self, new_state: ConnectionState) {
        self.ctx.borrow_mut().state = new_state;
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
    fn filter(
        &mut self,
        _ctx: &CodecContext,
        _packet: &mut Packet,
        _out: &mut Injections,
    ) -> Verdict {
        Verdict::Pass
    }
}

#[cfg(test)]
mod tests;

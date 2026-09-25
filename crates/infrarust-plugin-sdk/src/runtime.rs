use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::thread::LocalKey;

use crate::bindings::codec_filter::{
    CodecSessionInit as WitSessionInit, ConnectionState, FilterOutput, GuestFilterInstance,
};
use crate::bindings::codec_registry::CodecFilterMetadata;
use crate::bindings::command_manager::CommandSpec;
use crate::bindings::events::{Event, EventOutcome};
use crate::bindings::guest as wg;
use crate::codec::{
    CodecContext, CodecFilter, CodecRegistrar, CodecSessionInit, FilterConstructor, Injections,
    Packet, Verdict, build_filter_output,
};
use crate::command::{
    CommandClosure, CommandInvocation, CommandRegistration, CommandSender, Completion,
    CompletionClosure,
};
use crate::context::{Context, DisableReason, EnableReason};
use crate::error::Error;
use crate::event::{EventPriority, GuestEvent};
use crate::limbo::{HandlerOutcome, LimboHandler, LimboRegistrar, LimboSession, SessionEndReason};
use crate::plugin::Plugin;
use crate::registry::Registry;
use crate::types::PlayerId;

type EventEntry = RefCell<dyn FnMut(Event) -> EventOutcome>;
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

pub fn register_event<E: GuestEvent>(
    priority: EventPriority,
    mut handler: impl FnMut(&mut E) + 'static,
) -> Result<u64, Error> {
    let listener = crate::host::subscribe(E::KIND, priority.value())?;
    let entry: Rc<EventEntry> = Rc::new(RefCell::new(move |ev: Event| match E::from_event(ev) {
        Some(mut typed) => {
            handler(&mut typed);
            typed.into_outcome()
        }
        None => EventOutcome::Unchanged,
    }));
    EVENTS.with(|events| events.insert(listener, entry));
    Ok(listener)
}

pub fn unsubscribe_event(listener: u64) {
    let removed = EVENTS.with(|events| events.remove(listener));
    let _ = crate::host::unsubscribe(listener);
    drop(removed);
}

pub fn handle_event(listener: u64, ev: Event) -> EventOutcome {
    let Some(entry) = EVENTS.with(|events| events.get(listener)) else {
        return EventOutcome::Unchanged;
    };
    let Ok(mut handler) = entry.try_borrow_mut() else {
        return EventOutcome::Unchanged;
    };
    handler(ev)
}

pub(crate) fn register_command(
    spec: CommandSpec,
    handler: CommandClosure,
    completer: Option<CompletionClosure>,
) -> Result<CommandRegistration, Error> {
    let key = spec.name.to_lowercase();
    let id = next_id();
    let registration = crate::host::register_command(&spec, id)?;
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
    drop(replaced);
    Ok(CommandRegistration::from_wit(registration))
}

pub(crate) fn unregister_command(name: &str) -> Result<bool, Error> {
    let key = name.to_lowercase();
    let removed = COMMANDS.with(|commands| {
        commands
            .find(|entry| entry.name == key)
            .and_then(|id| commands.remove(id))
    });
    let Some(removed) = removed else {
        return Ok(false);
    };
    let answer = crate::host::unregister_command(&key);
    drop(removed);
    answer?;
    Ok(true)
}

pub fn handle_command(handler: u64, invocation: wg::CommandInvocation) {
    let Some(entry) = COMMANDS.with(|commands| commands.get(handler)) else {
        return;
    };
    if let Ok(mut run) = entry.handler.try_borrow_mut() {
        run(CommandInvocation::from_wit(invocation));
    }
}

pub fn tab_complete(
    handler: u64,
    sender: wg::CommandSender,
    args: Vec<String>,
    cursor: u32,
) -> Vec<wg::Suggestion> {
    let Some(entry) = COMMANDS.with(|commands| commands.get(handler)) else {
        return Vec::new();
    };
    let Some(complete) = entry.completer.as_ref() else {
        return Vec::new();
    };
    let completion = Completion {
        sender: CommandSender::from_wit(sender),
        args,
        cursor,
    };
    complete(&completion)
        .iter()
        .map(crate::command::Suggestion::to_wit)
        .collect()
}

pub(crate) fn schedule_delay(after_ms: u64, task: OnceTask) -> Result<u64, Error> {
    schedule(Task::Once(Cell::new(Some(task))), |id| {
        crate::host::delay(after_ms, id)
    })
}

pub(crate) fn schedule_interval(
    period_ms: u64,
    initial_delay_ms: Option<u64>,
    task: RepeatingTask,
) -> Result<u64, Error> {
    schedule(Task::Repeating(RefCell::new(task)), |id| {
        crate::host::interval(period_ms, initial_delay_ms, id)
    })
}

fn schedule(
    task: Task,
    start_on_host: impl FnOnce(u64) -> Result<u64, crate::bindings::types::HostError>,
) -> Result<u64, Error> {
    let id = next_id();
    let host_handle = start_on_host(id)?;
    TASKS.with(|tasks| tasks.insert(id, Rc::new(TaskEntry { host_handle, task })));
    Ok(id)
}

pub(crate) fn cancel_task(id: u64) {
    let Some(removed) = TASKS.with(|tasks| tasks.remove(id)) else {
        return;
    };
    let _ = crate::host::cancel(removed.host_handle);
}

pub fn on_scheduled_task(handler: u64) {
    let Some(entry) = TASKS.with(|tasks| tasks.get(handler)) else {
        return;
    };
    match &entry.task {
        Task::Once(slot) => {
            TASKS.with(|tasks| tasks.remove(handler));
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

pub fn on_enable<P: Plugin + Default>(reason: wg::EnableReason) -> Result<(), String> {
    let plugin = P::default();
    let result = plugin.on_enable(&Context::enabling(EnableReason::from_wit(reason)));
    if result.is_ok() {
        declare_codec_filters::<P>(true);
        declare_limbo_handlers::<P>();
    }
    PLUGIN.with(|p| *p.borrow_mut() = Some(Box::new(plugin)));
    result.map_err(String::from)
}

pub fn on_disable(reason: wg::DisableReason) -> Result<(), String> {
    let plugin = PLUGIN.with(|p| p.borrow_mut().take());
    match plugin {
        Some(plugin) => plugin
            .on_disable(&Context::disabling(DisableReason::from_wit(reason)))
            .map_err(String::from),
        None => Ok(()),
    }
}

pub(crate) fn register_codec_factory(
    notify: bool,
    metadata: CodecFilterMetadata,
    constructor: FilterConstructor,
) {
    let id = take_id(&NEXT_CODEC_FACTORY);
    CODEC_FACTORIES.with(|factories| factories.insert(id, Rc::from(constructor)));
    if notify {
        let _ = crate::host::register_codec_filter(&metadata, id);
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

pub(crate) fn register_limbo_handler(name: &str, handler: Box<dyn LimboHandler>) {
    let id = next_id();
    LIMBO_HANDLERS.with(|handlers| handlers.insert(id, Rc::from(handler)));
    if crate::host::register_limbo_handler(name, id).is_err() {
        let refused = LIMBO_HANDLERS.with(|handlers| handlers.remove(id));
        drop(refused);
    }
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

pub fn limbo_on_player_enter(handler: u64, session: &wg::LimboSession) -> wg::HandlerResult {
    with_limbo_handler(
        handler,
        || HandlerOutcome::Accept.into_wit(),
        |hdlr| hdlr.on_player_enter(&LimboSession::new(session)).into_wit(),
    )
}

pub fn limbo_on_command(
    handler: u64,
    session: &wg::LimboSession,
    command: String,
    args: Vec<String>,
) {
    with_limbo_handler(
        handler,
        || (),
        |hdlr| hdlr.on_command(&LimboSession::new(session), &command, &args),
    );
}

pub fn limbo_on_chat(handler: u64, session: &wg::LimboSession, message: String) {
    with_limbo_handler(
        handler,
        || (),
        |hdlr| hdlr.on_chat(&LimboSession::new(session), &message),
    );
}

pub fn limbo_on_disconnect(handler: u64, player: u64) {
    with_limbo_handler(
        handler,
        || (),
        |hdlr| hdlr.on_disconnect(PlayerId::new(player)),
    );
}

pub fn limbo_on_session_end(handler: u64, player: u64, reason: wg::SessionEndReason) {
    with_limbo_handler(
        handler,
        || (),
        |hdlr| hdlr.on_session_end(PlayerId::new(player), SessionEndReason::from_wit(reason)),
    );
}

pub fn create_codec_filter<P: Plugin>(factory: u64, init: WitSessionInit) -> FilterInstanceProxy {
    declare_codec_filters::<P>(false);
    let init = CodecSessionInit::from_wit(init);
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

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::thread::LocalKey;

use crate::ban_provider::{BanProvider, BanQuery, LoginAttempt, UnbanRequest};
use crate::bindings::ban_service as wb;
use crate::bindings::codec_filter::{
    CodecSessionInit as WitSessionInit, ConnectionState, FilterOutput, GuestFilterInstance,
};
use crate::bindings::codec_registry::CodecFilterMetadata;
use crate::bindings::command_manager::CommandSpec;
use crate::bindings::events::{Event, EventOutcome};
use crate::bindings::guest as wg;
use crate::bindings::permissions as wp;
use crate::codec::{
    CodecContext, CodecFilter, CodecRegistrar, CodecSessionInit, FilterConstructor, Injections,
    Packet, Verdict, build_filter_output,
};
use crate::command::{
    CommandClosure, CommandInvocation, CommandRegistration, CommandSender, Completion,
    CompletionClosure,
};
use crate::context::{Context, DisableReason, EnableReason};
use crate::error::{Error, PluginError};
use crate::event::{BanSource, EventPriority, GuestEvent};
use crate::limbo::{HandlerOutcome, LimboHandler, LimboRegistrar, LimboSession, SessionEndReason};
use crate::permissions::{PermissionProvider, PermissionSnapshot, PermissionSubject};
use crate::plugin::Plugin;
use crate::registry::Registry;
use crate::services::{BanRequest, BanTarget};
use crate::types::PlayerId;

pub(crate) const NO_BAN_PROVIDER: &str = "this plugin provides no bans";

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
    static BAN_PROVIDER: RefCell<Option<Rc<dyn BanProvider>>> = const { RefCell::new(None) };
    static PERMISSION_PROVIDER: RefCell<Option<Rc<dyn PermissionProvider>>> =
        const { RefCell::new(None) };
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
    handler: impl FnMut(&mut E) + 'static,
) -> Result<u64, Error> {
    register_listener(
        |priority| crate::host::subscribe(E::KIND, priority),
        priority,
        handler,
    )
}

pub(crate) fn register_listener<E: GuestEvent>(
    subscribe: impl FnOnce(u8) -> Result<u64, crate::bindings::types::HostError>,
    priority: EventPriority,
    mut handler: impl FnMut(&mut E) + 'static,
) -> Result<u64, Error> {
    let listener = subscribe(priority.value())?;
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
    if notify && crate::host::register_codec_filter(&metadata, id).is_err() {
        let refused = CODEC_FACTORIES.with(|factories| factories.remove(id));
        drop(refused);
    }
}

pub(crate) fn unregister_codec_filter(id: &str) -> Result<(), Error> {
    crate::host::unregister_codec_filter(id)?;
    Ok(())
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

pub(crate) fn provide_bans(provider: Rc<dyn BanProvider>) -> Result<(), Error> {
    crate::host::register_ban_provider(&provider.features().to_wit())?;
    let replaced = BAN_PROVIDER.with(|slot| slot.replace(Some(provider)));
    drop(replaced);
    Ok(())
}

pub(crate) fn provide_permissions(provider: Rc<dyn PermissionProvider>) -> Result<(), Error> {
    crate::host::register_permission_provider()?;
    let replaced = PERMISSION_PROVIDER.with(|slot| slot.replace(Some(provider)));
    drop(replaced);
    Ok(())
}

fn with_bans<T>(ask: impl FnOnce(&dyn BanProvider) -> Result<T, PluginError>) -> Result<T, String> {
    let provider = BAN_PROVIDER.with(|slot| slot.borrow().clone());
    match provider {
        Some(provider) => ask(&*provider).map_err(String::from),
        None => Err(NO_BAN_PROVIDER.to_owned()),
    }
}

pub fn ban_provider_check(attempt: wb::LoginAttempt) -> Result<Option<wb::BanVerdict>, String> {
    let attempt = LoginAttempt::from_wit(attempt);
    with_bans(|provider| {
        provider.check(&attempt).map(|verdict| {
            verdict
                .as_ref()
                .map(crate::ban_provider::BanVerdict::to_wit)
        })
    })
}

pub fn ban_provider_ban(
    request: wb::BanRequest,
    source: wb::BanSource,
) -> Result<wb::BanRecord, String> {
    let request = BanRequest::from_wit(request);
    let source = BanSource::from_wit(source);
    with_bans(|provider| provider.ban(request, source).map(|record| record.to_wit()))
}

pub fn ban_provider_unban(request: wb::UnbanRequest) -> Result<Option<wb::BanRecord>, String> {
    let request = UnbanRequest::from_wit(request);
    with_bans(|provider| {
        provider
            .unban(request)
            .map(|record| record.as_ref().map(crate::ban_provider::BanRecord::to_wit))
    })
}

pub fn ban_provider_get(target: wb::BanTarget) -> Result<Option<wb::BanRecord>, String> {
    let target = BanTarget::from_wit(target);
    with_bans(|provider| {
        provider
            .get(&target)
            .map(|record| record.as_ref().map(crate::ban_provider::BanRecord::to_wit))
    })
}

pub fn ban_provider_list(query: wb::BanQuery) -> Result<wb::BanRecordPage, String> {
    let query = BanQuery::from_wit(query);
    with_bans(|provider| provider.list(&query).map(|page| page.to_wit()))
}

pub fn permission_snapshot_for(subject: wp::PermissionSubject) -> wp::PermissionSnapshot {
    let provider = PERMISSION_PROVIDER.with(|slot| slot.borrow().clone());
    provider
        .map_or_else(PermissionSnapshot::new, |provider| {
            provider.snapshot_for(&PermissionSubject::from_wit(subject))
        })
        .to_wit()
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

//! Dispatch glue and the per-instance registries the macro-generated `Guest`
//! impl delegates to. The guest is single-threaded, so `thread_local!` +
//! `RefCell` is the cheapest correct storage.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use crate::bindings::codec_filter::{
    CodecContext, CodecVerdict, ConnectionState, FilterOutput, GuestFilterInstance, RawPacket,
};
use crate::bindings::guest::{Event, EventOutcome};
use crate::context::{CommandInvocation, Context};
use crate::event::{EventPriority, GuestEvent};
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

pub struct NoopFilterInstance;

impl GuestFilterInstance for NoopFilterInstance {
    fn filter(&self, _ctx: CodecContext, _packet: RawPacket) -> FilterOutput {
        FilterOutput {
            verdict: CodecVerdict::Pass,
            packet: None,
            inject_before: Vec::new(),
            inject_after: Vec::new(),
        }
    }
    fn on_state_change(&self, _new_state: ConnectionState) {}
    fn on_compression_change(&self, _threshold: i32) {}
    fn on_encryption_enabled(&self) {}
    fn on_close(&self) {}
}

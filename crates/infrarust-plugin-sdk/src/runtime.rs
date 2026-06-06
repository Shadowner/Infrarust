//! Dispatch glue and the per-instance registries the macro-generated `Guest`
//! impl delegates to. The guest is single-threaded, so `thread_local!` +
//! `RefCell` is the cheapest correct storage.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use crate::bindings::guest::{Event, EventOutcome};
use crate::context::{CommandInvocation, Context};
use crate::event::{EventPriority, GuestEvent, kind_of};
use crate::plugin::Plugin;

const SLOTS: usize = 16;

type EventClosure = Box<dyn FnMut(Event) -> EventOutcome>;
type CommandClosure = Box<dyn FnMut(CommandInvocation)>;
type TaskClosure = Box<dyn FnMut()>;

struct EventSlot {
    handle: u64,
    closure: EventClosure,
}

thread_local! {
    static EVENT_HANDLERS: RefCell<[Option<EventSlot>; SLOTS]> =
        RefCell::new(std::array::from_fn(|_| None));
    static COMMANDS: RefCell<HashMap<u64, CommandClosure>> = RefCell::new(HashMap::new());
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

/// Subscribe a typed handler for `E`. At most one handler per kind: a second
/// registration swaps the closure without adding a native listener (which would
/// double-fire, since `handle-event` carries no listener id).
pub fn register_event<E: GuestEvent>(
    priority: EventPriority,
    mut handler: impl FnMut(&mut E) + 'static,
) {
    let closure: EventClosure = Box::new(move |ev| match E::from_event(ev) {
        Some(mut typed) => {
            handler(&mut typed);
            typed.into_outcome()
        }
        None => EventOutcome::None,
    });

    let idx = E::KIND as usize;
    let subscribe = EVENT_HANDLERS.with(|hs| {
        let mut hs = hs.borrow_mut();
        match &mut hs[idx] {
            Some(slot) => {
                slot.closure = closure;
                false
            }
            None => {
                hs[idx] = Some(EventSlot { handle: 0, closure });
                true
            }
        }
    });

    if subscribe {
        let handle = crate::bindings::event_bus::subscribe(E::KIND, priority);
        EVENT_HANDLERS.with(|hs| {
            if let Some(slot) = &mut hs.borrow_mut()[idx] {
                slot.handle = handle;
            }
        });
    } else {
        crate::bindings::log::warn(&format!(
            "infrarust-plugin-sdk: handler for {:?} replaced (one handler per kind in v0.1)",
            E::KIND
        ));
    }
}

pub fn register_command(
    name: &str,
    aliases: &[String],
    description: &str,
    handler: CommandClosure,
) {
    let id = next_id();
    COMMANDS.with(|c| c.borrow_mut().insert(id, handler));
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

pub fn handle_event(ev: Event) -> EventOutcome {
    let idx = kind_of(&ev) as usize;
    let slot = EVENT_HANDLERS.with(|hs| hs.borrow_mut()[idx].take());
    match slot {
        Some(mut slot) => {
            let outcome = (slot.closure)(ev);
            EVENT_HANDLERS.with(|hs| {
                let mut hs = hs.borrow_mut();
                if hs[idx].is_none() {
                    hs[idx] = Some(slot);
                }
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

pub fn tab_complete(_callback_id: u64, _partial: Vec<String>) -> Vec<String> {
    Vec::new()
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

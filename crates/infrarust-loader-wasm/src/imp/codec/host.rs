use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use wasmtime::StoreContextMut;
use wasmtime::component::{ComponentType, LinkerInstance, Lower, Resource, WasmList};

use super::store_state::{CodecStoreState, GuestLevel};

const WRITE_BUDGET: u64 = 64 * 1024;

type Store<'a> = StoreContextMut<'a, CodecStoreState>;

#[derive(ComponentType, Lower)]
#[component(record)]
struct Datetime {
    seconds: u64,
    nanoseconds: u32,
}

#[derive(ComponentType, Lower)]
#[component(variant)]
enum StreamError {
    #[allow(dead_code)]
    #[component(name = "last-operation-failed")]
    LastOperationFailed(Resource<()>),
    #[component(name = "closed")]
    Closed,
}

fn wall_now() -> Datetime {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Datetime {
        seconds: since_epoch.as_secs(),
        nanoseconds: since_epoch.subsec_nanos(),
    }
}

fn monotonic_now() -> u64 {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    u64::try_from(ORIGIN.get_or_init(Instant::now).elapsed().as_nanos()).unwrap_or(u64::MAX)
}

fn discard() -> Resource<()> {
    Resource::new_own(0)
}

fn guest_level(function: &str) -> Option<GuestLevel> {
    match function {
        "trace" => Some(GuestLevel::Trace),
        "debug" => Some(GuestLevel::Debug),
        "info" => Some(GuestLevel::Info),
        "warn" => Some(GuestLevel::Warn),
        "error" => Some(GuestLevel::Error),
        _ => None,
    }
}

pub(super) fn define(
    slot: &mut LinkerInstance<'_, CodecStoreState>,
    interface: &str,
    function: &str,
) -> wasmtime::Result<bool> {
    match (interface, function) {
        ("infrarust:plugin/log", level) => {
            let Some(level) = guest_level(level) else {
                return Ok(false);
            };
            slot.func_wrap(function, move |store: Store<'_>, (message,): (String,)| {
                store.data().log(level, &message);
                Ok(())
            })?;
        }
        ("wasi:clocks/wall-clock", "now") => {
            slot.func_wrap(function, |_: Store<'_>, (): ()| Ok((wall_now(),)))?;
        }
        ("wasi:clocks/wall-clock", "resolution") => {
            slot.func_wrap(function, |_: Store<'_>, (): ()| {
                Ok((Datetime {
                    seconds: 0,
                    nanoseconds: 1,
                },))
            })?;
        }
        ("wasi:clocks/monotonic-clock", "now") => {
            slot.func_wrap(function, |_: Store<'_>, (): ()| Ok((monotonic_now(),)))?;
        }
        ("wasi:clocks/monotonic-clock", "resolution") => {
            slot.func_wrap(function, |_: Store<'_>, (): ()| Ok((1_u64,)))?;
        }
        ("wasi:random/random", "get-random-bytes")
        | ("wasi:random/insecure", "get-insecure-random-bytes") => {
            slot.func_wrap(function, |mut store: Store<'_>, (len,): (u64,)| {
                Ok((store.data_mut().random_bytes(len)?,))
            })?;
        }
        ("wasi:random/random", "get-random-u64")
        | ("wasi:random/insecure", "get-insecure-random-u64") => {
            slot.func_wrap(function, |mut store: Store<'_>, (): ()| {
                Ok((store.data_mut().random_u64(),))
            })?;
        }
        ("wasi:random/insecure-seed", "insecure-seed") => {
            slot.func_wrap(function, |mut store: Store<'_>, (): ()| {
                let state = store.data_mut();
                Ok(((state.random_u64(), state.random_u64()),))
            })?;
        }
        ("wasi:cli/environment", "get-environment") => {
            slot.func_wrap(function, |_: Store<'_>, (): ()| {
                Ok((Vec::<(String, String)>::new(),))
            })?;
        }
        ("wasi:cli/environment", "get-arguments") => {
            slot.func_wrap(function, |_: Store<'_>, (): ()| Ok((Vec::<String>::new(),)))?;
        }
        ("wasi:cli/environment", "initial-cwd") => {
            slot.func_wrap(function, |_: Store<'_>, (): ()| Ok((None::<String>,)))?;
        }
        ("wasi:cli/stdin", "get-stdin")
        | ("wasi:cli/stdout", "get-stdout")
        | ("wasi:cli/stderr", "get-stderr") => {
            slot.func_wrap(function, |_: Store<'_>, (): ()| Ok((discard(),)))?;
        }
        ("wasi:cli/terminal-stdin", "get-terminal-stdin")
        | ("wasi:cli/terminal-stdout", "get-terminal-stdout")
        | ("wasi:cli/terminal-stderr", "get-terminal-stderr") => {
            slot.func_wrap(function, |_: Store<'_>, (): ()| Ok((None::<Resource<()>>,)))?;
        }
        ("wasi:io/poll", "poll") => {
            slot.func_wrap(
                function,
                |_: Store<'_>, (pollables,): (Vec<Resource<()>>,)| {
                    let ready = u32::try_from(pollables.len())?;
                    Ok(((0..ready).collect::<Vec<u32>>(),))
                },
            )?;
        }
        ("wasi:io/poll", "[method]pollable.ready") => {
            slot.func_wrap(function, |_: Store<'_>, (_,): (Resource<()>,)| Ok((true,)))?;
        }
        ("wasi:io/poll", "[method]pollable.block") => {
            slot.func_wrap(function, |_: Store<'_>, (_,): (Resource<()>,)| Ok(()))?;
        }
        ("wasi:io/streams", "[method]output-stream.check-write") => {
            slot.func_wrap(function, |_: Store<'_>, (_,): (Resource<()>,)| {
                Ok((Ok::<u64, StreamError>(WRITE_BUDGET),))
            })?;
        }
        (
            "wasi:io/streams",
            "[method]output-stream.write" | "[method]output-stream.blocking-write-and-flush",
        ) => {
            slot.func_wrap(
                function,
                |_: Store<'_>, (_, _): (Resource<()>, WasmList<u8>)| {
                    Ok((Ok::<(), StreamError>(()),))
                },
            )?;
        }
        (
            "wasi:io/streams",
            "[method]output-stream.flush" | "[method]output-stream.blocking-flush",
        ) => {
            slot.func_wrap(function, |_: Store<'_>, (_,): (Resource<()>,)| {
                Ok((Ok::<(), StreamError>(()),))
            })?;
        }
        (
            "wasi:io/streams",
            "[method]output-stream.write-zeroes"
            | "[method]output-stream.blocking-write-zeroes-and-flush",
        ) => {
            slot.func_wrap(function, |_: Store<'_>, (_, _): (Resource<()>, u64)| {
                Ok((Ok::<(), StreamError>(()),))
            })?;
        }
        (
            "wasi:io/streams",
            "[method]output-stream.subscribe" | "[method]input-stream.subscribe",
        ) => {
            slot.func_wrap(function, |_: Store<'_>, (_,): (Resource<()>,)| {
                Ok((discard(),))
            })?;
        }
        ("wasi:io/streams", "[method]input-stream.read" | "[method]input-stream.blocking-read") => {
            slot.func_wrap(function, |_: Store<'_>, (_, _): (Resource<()>, u64)| {
                Ok((Err::<Vec<u8>, StreamError>(StreamError::Closed),))
            })?;
        }
        ("wasi:io/streams", "[method]input-stream.skip" | "[method]input-stream.blocking-skip") => {
            slot.func_wrap(function, |_: Store<'_>, (_, _): (Resource<()>, u64)| {
                Ok((Err::<u64, StreamError>(StreamError::Closed),))
            })?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

use std::cell::RefCell;
use std::time::{Duration, Instant};

use infrarust_plugin_sdk::prelude::*;

#[derive(Default, Clone, Copy)]
pub struct Knobs {
    chat_trap_every: u64,
    chat_busy_ms: u64,
    login_spin_every: u64,
    login_grow_every: u64,
    login_sleep_ms: u64,
    preconnect_sleep_ms: u64,
    preconnect_busy_ms: u64,
    disconnect_trap_every: u64,
    tick_ms: u64,
    tick_trap_every: u64,
    command_trap_every: u64,
    leak_kb_per_event: u64,
}

impl Knobs {
    pub fn load() -> Self {
        let text = std::fs::read_to_string("/soak.txt").unwrap_or_default();
        let mut knobs = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let Ok(value) = value.trim().parse::<u64>() else {
                continue;
            };
            let slot = match key.trim() {
                "chat_trap_every" => &mut knobs.chat_trap_every,
                "chat_busy_ms" => &mut knobs.chat_busy_ms,
                "login_spin_every" => &mut knobs.login_spin_every,
                "login_grow_every" => &mut knobs.login_grow_every,
                "login_sleep_ms" => &mut knobs.login_sleep_ms,
                "preconnect_sleep_ms" => &mut knobs.preconnect_sleep_ms,
                "preconnect_busy_ms" => &mut knobs.preconnect_busy_ms,
                "disconnect_trap_every" => &mut knobs.disconnect_trap_every,
                "tick_ms" => &mut knobs.tick_ms,
                "tick_trap_every" => &mut knobs.tick_trap_every,
                "command_trap_every" => &mut knobs.command_trap_every,
                "leak_kb_per_event" => &mut knobs.leak_kb_per_event,
                _ => continue,
            };
            *slot = value;
        }
        knobs
    }
}

#[derive(Default)]
struct Counters {
    chat: u64,
    login: u64,
    preconnect: u64,
    disconnect: u64,
    tick: u64,
    command: u64,
}

thread_local! {
    static COUNTERS: RefCell<Counters> = RefCell::new(Counters::default());
    static HOARD: RefCell<Vec<Vec<u8>>> = const { RefCell::new(Vec::new()) };
    static RNG: RefCell<u64> = const { RefCell::new(0x9e37_79b9_7f4a_7c15) };
}

fn seed() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |d| d.as_nanos());
    let folded = u64::try_from(nanos % u128::from(u64::MAX)).unwrap_or(1) | 1;
    RNG.with(|rng| *rng.borrow_mut() = folded);
}

fn roll(every: u64) -> bool {
    if every == 0 {
        return false;
    }
    RNG.with(|rng| {
        let mut x = *rng.borrow();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *rng.borrow_mut() = x;
        x % every == 0
    })
}

fn bump(select: impl FnOnce(&mut Counters) -> &mut u64) -> u64 {
    COUNTERS.with(|counters| {
        let mut counters = counters.borrow_mut();
        let slot = select(&mut counters);
        *slot += 1;
        *slot
    })
}

fn spin_forever() -> ! {
    loop {
        std::hint::black_box(0u64);
    }
}

fn grow_forever() -> ! {
    loop {
        HOARD.with(|hoard| hoard.borrow_mut().push(vec![7u8; 1 << 20]));
    }
}

fn busy(ms: u64) {
    if ms == 0 {
        return;
    }
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(ms) {
        std::hint::black_box(0u64);
    }
}

fn nap(ms: u64) {
    if ms > 0 {
        std::thread::sleep(Duration::from_millis(ms));
    }
}

fn hoard(kb: u64) {
    if kb > 0 {
        let bytes = usize::try_from(kb * 1024).unwrap_or(1024);
        HOARD.with(|hoard| hoard.borrow_mut().push(vec![3u8; bytes]));
    }
}

fn record_enable(ctx: &Context, id: &str) {
    if let Some(EnableReason::Recovered(info)) = ctx.enable_reason() {
        let line = format!("{id} attempt={} cause={}\n", info.attempt, info.cause);
        let mut previous = std::fs::read_to_string("/recoveries.txt").unwrap_or_default();
        if previous.len() > 64 * 1024 {
            previous.clear();
        }
        previous.push_str(&line);
        let _ = std::fs::write("/recoveries.txt", previous);
    }
}

pub fn install(ctx: &Context, id: &str, command: &str) -> Result<(), PluginError> {
    let knobs = Knobs::load();
    seed();
    record_enable(ctx, id);
    let owner = id.to_string();
    ctx.on::<ChatMessageEvent>(EventPriority::Late, move |_| {
        let n = bump(|c| &mut c.chat);
        hoard(knobs.leak_kb_per_event);
        busy(knobs.chat_busy_ms);
        if roll(knobs.chat_trap_every) {
            panic!("{owner}: chat trap at {n}");
        }
    })?;
    ctx.on::<PostLoginEvent>(EventPriority::Normal, move |_| {
        bump(|c| &mut c.login);
        nap(knobs.login_sleep_ms);
        if roll(knobs.login_spin_every) {
            spin_forever();
        }
        if roll(knobs.login_grow_every) {
            grow_forever();
        }
    })?;
    ctx.on::<ServerPreConnectEvent>(EventPriority::Normal, move |_| {
        bump(|c| &mut c.preconnect);
        nap(knobs.preconnect_sleep_ms);
        busy(knobs.preconnect_busy_ms);
    })?;
    let owner = id.to_string();
    ctx.on::<DisconnectEvent>(EventPriority::Normal, move |_| {
        let n = bump(|c| &mut c.disconnect);
        if roll(knobs.disconnect_trap_every) {
            panic!("{owner}: disconnect trap at {n}");
        }
    })?;
    if knobs.tick_ms > 0 {
        let owner = id.to_string();
        ctx.interval(Duration::from_millis(knobs.tick_ms), move || {
            let n = bump(|c| &mut c.tick);
            if roll(knobs.tick_trap_every) {
                panic!("{owner}: tick trap at {n}");
            }
        })?;
    }
    let owner = id.to_string();
    let reply_label = format!("{command}-ok");
    ctx.command(command)
        .description("Soak misbehaving command")
        .handler(move |invocation| {
            let n = bump(|c| &mut c.command);
            if roll(knobs.command_trap_every) {
                panic!("{owner}: command trap at {n}");
            }
            let token = invocation.args.first().cloned().unwrap_or_default();
            let _ = invocation.reply(Component::text(format!("{reply_label} {token}")));
        })
        .register()?;
    Ok(())
}

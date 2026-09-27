use std::hint::black_box;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use infrarust_plugin_sdk::prelude::*;

use crate::faults::{self, Mode};

fn guest_path(name: &str) -> PathBuf {
    Path::new("/").join(name)
}

fn faults_text() -> String {
    std::fs::read_to_string(guest_path(faults::FAULTS_FILE)).unwrap_or_default()
}

pub fn log(line: &str) {
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(guest_path(faults::LOG_FILE))
    {
        let _ = writeln!(file, "{line}");
    }
}

#[inline(never)]
fn recurse(depth: u64, parent: &mut [u8; 512]) -> u64 {
    let mut frame = [0u8; 512];
    frame[0] = parent[1].wrapping_add(1);
    black_box(&mut frame);
    if black_box(depth) == u64::MAX {
        return u64::from(frame[3]);
    }
    recurse(depth + 1, &mut frame) + u64::from(frame[2])
}

pub fn fault(mode: Mode, site: &str) -> Result<(), PluginError> {
    match mode {
        Mode::Panic => panic!("fault-lab: {site} panics on purpose"),
        Mode::Unreachable => core::arch::wasm32::unreachable(),
        Mode::Stack => {
            let mut root = [0u8; 512];
            black_box(recurse(0, &mut root));
            Ok(())
        }
        Mode::Grow => {
            let mut sink: Vec<Vec<u8>> = Vec::new();
            loop {
                sink.push(black_box(vec![1u8; 1 << 20]));
            }
        }
        Mode::Huge => {
            let huge: Vec<u8> = Vec::with_capacity(black_box(1usize << 30));
            black_box(&huge);
            Ok(())
        }
        Mode::Spin => loop {
            black_box(0u64);
        },
        Mode::Sleep => {
            std::thread::sleep(Duration::from_secs(3600));
            Ok(())
        }
        Mode::Refuse => Err(PluginError::from(format!("fault-lab: {site} refuses"))),
        Mode::Linger => {
            std::thread::sleep(Duration::from_millis(400));
            Ok(())
        }
    }
}

pub fn strike(site: &str) -> Result<(), PluginError> {
    let mode = faults::mode_at(&faults_text(), site);
    log(site);
    match mode {
        Some(mode) => fault(mode, site),
        None => Ok(()),
    }
}

fn strike_event(name: &str) {
    let site = format!("event:{name}");
    let text = faults_text();
    let mode = faults::mode_at(&text, &site).or_else(|| faults::mode_at(&text, "event"));
    log(&site);
    if let Some(mode) = mode {
        let _ = fault(mode, &site);
    }
}

struct LabProvider;

impl BanProvider for LabProvider {
    fn check(&self, _attempt: &LoginAttempt) -> Result<Option<BanVerdict>, PluginError> {
        strike("ban-check")?;
        Ok(None)
    }

    fn ban(&self, request: BanRequest, source: BanSource) -> Result<BanRecord, PluginError> {
        strike("ban-ban")?;
        Ok(BanRecord::new("lab-1", request.target, source))
    }

    fn unban(&self, _request: UnbanRequest) -> Result<Option<BanRecord>, PluginError> {
        strike("ban-unban")?;
        Ok(None)
    }

    fn get(&self, _target: &BanTarget) -> Result<Option<BanRecord>, PluginError> {
        strike("ban-get")?;
        Ok(None)
    }

    fn list(&self, _query: &BanQuery) -> Result<BanRecordPage, PluginError> {
        strike("ban-list")?;
        Ok(BanRecordPage::new(Vec::new(), None))
    }
}

impl PermissionProvider for LabProvider {
    fn snapshot_for(&self, _subject: &PermissionSubject) -> PermissionSnapshot {
        let _ = strike("permission");
        PermissionSnapshot::new().grant("fault-lab.answered")
    }
}

struct LabLimbo;

impl LimboHandler for LabLimbo {
    fn on_player_enter(&self, session: &LimboSession) -> HandlerOutcome {
        match strike("limbo-enter") {
            Ok(()) => {
                log(&format!("held {}", session.player_id().as_u64()));
                HandlerOutcome::Hold
            }
            Err(_) => HandlerOutcome::Accept,
        }
    }

    fn on_command(&self, session: &LimboSession, command: &str, _args: &[String]) {
        let _ = strike("limbo-command");
        if command == "accept" {
            let _ = session.complete(HandlerOutcome::Accept);
        }
    }

    fn on_chat(&self, _session: &LimboSession, _message: &str) {
        let _ = strike("limbo-chat");
    }

    fn on_disconnect(&self, _player: PlayerId) {
        let _ = strike("limbo-disconnect");
    }

    fn on_session_end(&self, _player: PlayerId, _reason: SessionEndReason) {
        let _ = strike("limbo-session-end");
    }
}

struct LabCodec;

impl CodecFilter for LabCodec {
    fn filter(
        &mut self,
        _ctx: &CodecContext,
        packet: &mut Packet,
        _out: &mut Injections,
    ) -> Verdict {
        let id = packet.id();
        if let Some(mode) = u64::try_from(id - faults::CODEC_FAULT_PACKET_BASE)
            .ok()
            .and_then(Mode::from_code)
        {
            let _ = fault(mode, "codec-filter");
        }
        if id == faults::CODEC_MARK_PACKET {
            packet.set_data(b"fault-lab".to_vec());
        }
        Verdict::Pass
    }
}

pub fn codec_filters(reg: &mut CodecRegistrar) {
    reg.add(faults::CODEC_FILTER_ID, FilterPriority::Normal, |init| {
        if let Some(mode) = init
            .connection_id
            .checked_sub(faults::CODEC_FAULT_CONNECTION_BASE)
            .and_then(Mode::from_code)
        {
            let _ = fault(mode, "codec-create");
        }
        Box::new(LabCodec)
    });
}

pub fn limbo_handlers(reg: &mut LimboRegistrar) {
    let wanted = faults::directives(&faults_text())
        .iter()
        .any(|words| words[0] == "limbo");
    if wanted {
        reg.add(faults::LIMBO_HANDLER, LabLimbo);
    }
}

fn subscribe_events(ctx: &Context) -> Result<(), PluginError> {
    ctx.on::<PreLoginEvent>(EventPriority::Normal, |e| {
        strike_event("pre-login");
        e.deny(Component::text("fault-lab"));
    })?;
    ctx.on::<PostLoginEvent>(EventPriority::Normal, |_| strike_event("post-login"))?;
    ctx.on::<ChatMessageEvent>(EventPriority::Normal, |e| {
        strike_event("chat-message");
        e.modify("fault-lab");
    })?;
    ctx.on::<ServerPreConnectEvent>(EventPriority::Normal, |e| {
        strike_event("server-pre-connect");
        e.redirect_to("fault-lab");
    })?;
    ctx.on::<ProxyPingEvent>(EventPriority::Normal, |e| {
        strike_event("proxy-ping");
        e.response_mut().description = Component::text("fault-lab");
    })?;
    ctx.on::<DisconnectEvent>(EventPriority::Normal, |_| strike_event("disconnect"))?;
    Ok(())
}

fn fire(name: &str) {
    let line = match Context::new().fire_named_text(name, "fault-lab") {
        Ok(outcome) => format!(
            "fired {name} {}",
            outcome
                .response
                .as_ref()
                .and_then(NamedResponse::text)
                .unwrap_or("-")
        ),
        Err(error) => format!("fire {name} failed {}", error.kind().as_str()),
    };
    log(&line);
}

fn command_prefix() -> String {
    faults::directives(&faults_text())
        .iter()
        .find(|words| words[0] == "prefix")
        .and_then(|words| words.get(1).cloned())
        .unwrap_or_default()
}

fn register_commands(ctx: &Context) -> Result<(), PluginError> {
    let prefix = command_prefix();
    ctx.command(format!("{prefix}lab"))
        .handler(|invocation| {
            let _ = strike("command");
            log(&format!("command {}", invocation.args.join(",")));
        })
        .completer(|_| {
            let _ = strike("complete");
            vec!["lab-suggestion"]
        })
        .register()?;
    ctx.command(format!("{prefix}labfire"))
        .handler(|invocation| {
            let _ = strike("fire");
            if let Some(name) = invocation.args.first() {
                fire(name);
            }
        })
        .register()?;
    let late = format!("{prefix}lab-late");
    ctx.command(format!("{prefix}labreg"))
        .handler(move |_| {
            let registered = Context::new()
                .command(late.clone())
                .handler(|_| log("command late"))
                .register();
            log(&format!("labreg {}", registered.is_ok()));
        })
        .register()?;
    Ok(())
}

fn apply(ctx: &Context, words: &[String]) -> Result<(), PluginError> {
    let word = |at: usize| words.get(at).map_or("", String::as_str);
    let millis = |at: usize| Duration::from_millis(word(at).parse().unwrap_or(1000));
    match word(0) {
        "interval" => {
            ctx.interval(millis(1), || {
                let _ = strike("task");
            })?;
        }
        "delay" => {
            ctx.delay(millis(1), || {
                let _ = strike("task");
            })?;
        }
        "listen" => {
            let name = word(1).to_owned();
            let site = format!("named:{name}");
            ctx.on_named(name, EventPriority::Normal, move |e| {
                let _ = strike(&site);
                e.respond_text("fault-lab");
            })?;
        }
        "relay" => {
            let heard = word(1).to_owned();
            let next = word(2).to_owned();
            let site = format!("named:{heard}");
            ctx.on_named(heard, EventPriority::Normal, move |_| {
                let _ = strike(&site);
                fire(&next);
            })?;
        }
        "enable-fire" => {
            if matches!(ctx.enable_reason(), Some(EnableReason::Recovered(_))) {
                fire(word(1));
            }
        }
        "bans" => ctx.provide_bans(LabProvider)?,
        "permissions" => ctx.provide_permissions(LabProvider)?,
        "channel" => Messaging::register(&ChannelId::modern(word(1)))?,
        _ => {}
    }
    Ok(())
}

pub fn enable(ctx: &Context) -> Result<(), PluginError> {
    let reason = match ctx.enable_reason() {
        Some(EnableReason::Recovered(info)) => {
            format!("enable recovered {} {}", info.attempt, info.cause)
        }
        _ => "enable initial".to_owned(),
    };
    log(&reason);
    subscribe_events(ctx)?;
    register_commands(ctx)?;
    for words in faults::directives(&faults_text()) {
        apply(ctx, &words)?;
    }
    strike("enable")
}

pub fn disable() -> Result<(), PluginError> {
    strike("disable")
}

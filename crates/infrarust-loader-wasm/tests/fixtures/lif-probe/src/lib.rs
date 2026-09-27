use std::io::Write;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use infrarust_plugin_sdk::prelude::*;

const MARKER: &[u8; 16] = b"LIFPROBE-BLOB-V1";
const BLOB_LEN: usize = 8192;
const END: u8 = 0xFF;

static BLOB: [u8; BLOB_LEN] = blob();

const fn blob() -> [u8; BLOB_LEN] {
    let mut bytes = [END; BLOB_LEN];
    let mut at = 0;
    while at < MARKER.len() {
        bytes[at] = MARKER[at];
        at += 1;
    }
    bytes
}

fn settings_text() -> String {
    let base = BLOB.as_ptr();
    let mut out = Vec::new();
    for at in MARKER.len()..BLOB_LEN {
        let byte = unsafe { std::ptr::read_volatile(base.add(at)) };
        if byte == END {
            break;
        }
        out.push(byte);
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn settings() -> Vec<(String, String)> {
    settings_text()
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}

fn setting(key: &str) -> Option<String> {
    settings()
        .into_iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
}

fn every(key: &str) -> Vec<String> {
    settings()
        .into_iter()
        .filter(|(k, _)| k == key)
        .map(|(_, v)| v)
        .collect()
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos())
}

fn log(line: &str) {
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/log.txt")
    {
        let _ = writeln!(file, "{line} {}", now_nanos());
    }
}

fn act(action: &str) -> Result<(), PluginError> {
    let (verb, arg) = action.split_once(':').unwrap_or((action, ""));
    match verb {
        "fail" => Err(PluginError::from("lif-probe refused on purpose")),
        "panic" => panic!("lif-probe panic on purpose"),
        "spin" => loop {
            std::hint::black_box(0u64);
        },
        "sleep" => {
            let millis = arg.parse().unwrap_or(0);
            std::thread::sleep(Duration::from_millis(millis));
            Ok(())
        }
        _ => Ok(()),
    }
}

#[derive(Default)]
struct LifProbe;

#[plugin]
impl Plugin for LifProbe {
    fn metadata(&self) -> PluginMetadata {
        if let Some(action) = setting("meta") {
            let _ = act(&action);
        }
        let mut metadata = PluginMetadata::new(
            setting("id").unwrap_or_else(|| "lif-probe".to_owned()),
            setting("name").unwrap_or_else(|| "LIF Probe".to_owned()),
            setting("version").unwrap_or_else(|| "0.1.0".to_owned()),
        );
        for dependency in every("dep") {
            metadata = metadata.depends_on(dependency);
        }
        for dependency in every("softdep") {
            metadata = metadata.optional_dependency(dependency);
        }
        metadata
    }

    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
        log(&format!("enable {:?}", ctx.enable_reason()));
        for spec in every("cmd") {
            let (name, action) = spec.split_once(':').unwrap_or((spec.as_str(), "ok"));
            let label = name.to_owned();
            let action = action.to_owned();
            ctx.command(name)
                .handler(move |_| {
                    log(&format!("cmd {label} start"));
                    let _ = act(&action);
                    log(&format!("cmd {label} end"));
                })
                .register()?;
        }
        if let Some(action) = setting("post-login") {
            ctx.on::<PostLoginEvent>(EventPriority::Normal, move |_| {
                log("post-login start");
                let _ = act(&action);
                log("post-login end");
            })?;
        }
        let outcome = act(&setting("enable").unwrap_or_default());
        log("enable-end");
        outcome
    }

    fn on_disable(&self, ctx: &Context) -> Result<(), PluginError> {
        log(&format!("disable {:?}", ctx.disable_reason()));
        let outcome = act(&setting("disable").unwrap_or_default());
        log("disable-end");
        outcome
    }
}

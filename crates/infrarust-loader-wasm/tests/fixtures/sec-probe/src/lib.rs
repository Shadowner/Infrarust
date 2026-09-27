use std::fs::OpenOptions;
use std::io::Write;
use std::time::Duration;

use infrarust_plugin_sdk::bindings::events::EventKind;
use infrarust_plugin_sdk::bindings::types::{ChannelId as WitChannelId, HostError};
use infrarust_plugin_sdk::bindings::{
    codec_registry, command_manager, config_service, event_bus, limbo, messaging, scheduler,
};
use infrarust_plugin_sdk::prelude::*;

const LOG: &str = "sec.log";

#[derive(Default)]
struct SecProbe;

#[plugin(id = "sec-probe", name = "Security Probe Fixture")]
impl Plugin for SecProbe {
    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
        ctx.command("sec")
            .description("Runs one host-abuse probe and appends the outcome to sec.log")
            .handler(|invocation| {
                let args = invocation.args;
                let Some((tag, rest)) = args.split_first() else {
                    return;
                };
                record(tag, run(rest));
            })
            .register()?;
        Ok(())
    }
}

fn record(tag: &str, outcome: Result<String, String>) {
    let line = match outcome {
        Ok(value) => format!("{tag} ok {value}\n"),
        Err(error) => format!("{tag} err {error}\n"),
    };
    if let Ok(mut log) = OpenOptions::new().create(true).append(true).open(LOG) {
        let _ = log.write_all(line.as_bytes());
    }
}

fn run(args: &[String]) -> Result<String, String> {
    let arg = |index: usize| {
        args.get(index)
            .map(String::as_str)
            .ok_or_else(|| format!("missing argument {index}"))
    };
    let count = |index: usize| arg(index)?.parse::<u32>().map_err(|e| e.to_string());
    match arg(0)? {
        "commands" => Ok(flood_commands(count(1)?).to_string()),
        "channels" => flood_channels(count(1)?),
        "tasks" => Ok(flood_tasks(count(1)?).to_string()),
        "listeners" => Ok(flood_listeners(count(1)?).to_string()),
        "quota" => quota(arg(1)?, count(2)?),
        "json-nest" => parse_nested(count(1)?),
        "json-huge" => parse_huge(count(1)?),
        "forge-get" => Ok(match Players::get(PlayerId::new(u64::MAX)) {
            Some(_) => "present".to_owned(),
            None => "absent".to_owned(),
        }),
        "forge-unsub" => forge_unsub(arg(1)?.parse::<u64>().map_err(|e| e.to_string())?),
        "forge-cancel" => forge_cancel(arg(1)?.parse::<u64>().map_err(|e| e.to_string())?),
        "forge-unregister" => forge_unregister(arg(1)?),
        "forge-unreg-codec" => forge_unreg_codec(arg(1)?),
        "forge-unreg-channel" => forge_unreg_channel(arg(1)?),
        "config-dump" => config_dump(),
        "config-effective" => config_effective(),
        "config-get" => config_get(arg(1)?),
        "config-write" => config_write(arg(1)?),
        "config-edit" => config_edit(arg(1)?, arg(2)?),
        "caps" => Ok(Proxy::granted_capabilities()
            .into_iter()
            .map(Capability::to_kebab)
            .collect::<Vec<_>>()
            .join(",")),
        other => Err(format!("unknown probe {other}")),
    }
}

fn flood_commands(n: u32) -> u32 {
    let ctx = Context::new();
    let mut registered = 0;
    for index in 0..n {
        let ok = ctx
            .command(format!("flood-{index}"))
            .description("flood")
            .handler(|_| {})
            .register()
            .is_ok();
        if ok {
            registered += 1;
        }
    }
    registered
}

fn flood_channels(n: u32) -> Result<String, String> {
    let mut registered = 0;
    for index in 0..n {
        if Messaging::register(&ChannelId::modern(format!("sec:flood{index}"))).is_ok() {
            registered += 1;
        }
    }
    let held = Messaging::channels().map(|list| list.len()).unwrap_or(0);
    Ok(format!("{registered} {held}"))
}

fn kind(error: &HostError) -> String {
    format!("{:?}", error.kind)
}

fn flood_listeners(n: u32) -> u32 {
    let mut registered = 0;
    for _ in 0..n {
        if event_bus::subscribe(EventKind::PostLogin, 0).is_ok() {
            registered += 1;
        }
    }
    registered
}

fn register_one(family: &str, index: u32) -> Result<Result<u64, HostError>, String> {
    let name = format!("quota{index}");
    let handler = 900_000 + u64::from(index);
    Ok(match family {
        "listeners" => event_bus::subscribe(EventKind::PostLogin, 0),
        "commands" => command_manager::register(
            &command_manager::CommandSpec {
                name,
                aliases: Vec::new(),
                description: String::new(),
                usage: None,
                permission: None,
                hidden: false,
            },
            handler,
        )
        .map(|_| u64::from(index)),
        "tasks" => scheduler::delay(u64::MAX, handler),
        "channels" => messaging::register_channel(&quota_channel(index)).map(|()| u64::from(index)),
        "codecs" => codec_registry::register_codec_filter(
            &codec_registry::CodecFilterMetadata {
                id: name,
                priority: codec_registry::FilterPriority::Normal,
                after: Vec::new(),
                before: Vec::new(),
                required: false,
            },
            handler,
        )
        .map(|()| u64::from(index)),
        "limbo" => limbo::register_limbo_handler(&name, handler).map(|()| u64::from(index)),
        other => return Err(format!("unknown quota kind {other}")),
    })
}

fn release_one(family: &str, handle: u64) -> Result<(), String> {
    let index = u32::try_from(handle).unwrap_or(u32::MAX);
    let released = match family {
        "listeners" => event_bus::unsubscribe(handle).map(|_| ()),
        "commands" => command_manager::unregister(&format!("quota{index}")),
        "tasks" => scheduler::cancel(handle),
        "channels" => messaging::unregister_channel(&quota_channel(index)).map(|_| ()),
        "codecs" => codec_registry::unregister_codec_filter(&format!("quota{index}")),
        other => return Err(format!("{other} has no release")),
    };
    released.map_err(|error| kind(&error))
}

fn quota_channel(index: u32) -> WitChannelId {
    WitChannelId {
        modern: Some(format!("sec:quota{index}")),
        legacy: None,
    }
}

fn quota(family: &str, n: u32) -> Result<String, String> {
    let mut handles = Vec::new();
    let mut refused = String::from("none");
    for index in 0..n {
        match register_one(family, index)? {
            Ok(handle) => handles.push(handle),
            Err(error) if refused == "none" => refused = kind(&error),
            Err(_) => {}
        }
    }
    let retry = match handles.first() {
        Some(&handle) if family != "limbo" => {
            release_one(family, handle)?;
            match register_one(family, n)? {
                Ok(_) => "ok".to_owned(),
                Err(error) => kind(&error),
            }
        }
        _ => "none".to_owned(),
    };
    Ok(format!("{} {refused} retry={retry}", handles.len()))
}

fn forge_unsub(handle: u64) -> Result<String, String> {
    event_bus::unsubscribe(handle)
        .map(|removed| removed.to_string())
        .map_err(|error| kind(&error))
}

fn forge_cancel(handle: u64) -> Result<String, String> {
    scheduler::cancel(handle)
        .map(|()| "cancelled".to_owned())
        .map_err(|error| kind(&error))
}

fn forge_unregister(name: &str) -> Result<String, String> {
    command_manager::unregister(name)
        .map(|()| "unregistered".to_owned())
        .map_err(|error| kind(&error))
}

fn forge_unreg_codec(id: &str) -> Result<String, String> {
    codec_registry::unregister_codec_filter(id)
        .map(|()| "unregistered".to_owned())
        .map_err(|error| kind(&error))
}

fn forge_unreg_channel(name: &str) -> Result<String, String> {
    let channel = WitChannelId {
        modern: Some(name.to_owned()),
        legacy: None,
    };
    messaging::unregister_channel(&channel)
        .map(|removed| removed.to_string())
        .map_err(|error| kind(&error))
}

fn config_dump() -> Result<String, String> {
    config_service::get_proxy_config_document()
        .map(|document| one_line(&document))
        .map_err(|error| kind(&error))
}

fn config_effective() -> Result<String, String> {
    config_service::get_effective_proxy_config_document()
        .map(|document| one_line(&document))
        .map_err(|error| kind(&error))
}

fn one_line(document: &str) -> String {
    document.replace('\n', "\\n")
}

fn config_get(key: &str) -> Result<String, String> {
    config_service::get_value(key)
        .map(|value| value.unwrap_or_else(|| "none".to_owned()))
        .map_err(|error| kind(&error))
}

fn config_write(document: &str) -> Result<String, String> {
    config_service::write_proxy_config_document(&document.replace("\\n", "\n"))
        .map(|()| "written".to_owned())
        .map_err(|error| kind(&error))
}

fn config_edit(from: &str, to: &str) -> Result<String, String> {
    let document = config_service::get_proxy_config_document().map_err(|error| kind(&error))?;
    config_write(&document.replace(from, to))
}

fn flood_tasks(n: u32) -> u32 {
    let ctx = Context::new();
    let mut scheduled = 0;
    for _ in 0..n {
        if ctx.delay(Duration::from_millis(u64::MAX), || {}).is_ok() {
            scheduled += 1;
        }
    }
    scheduled
}

fn parse_nested(depth: u32) -> Result<String, String> {
    let mut json = String::from("{\"text\":\"x\"}");
    for _ in 0..depth {
        json = format!("{{\"extra\":[{json}]}}");
    }
    Component::from_json(&json)
        .map(|component| format!("plain-len {}", component.to_plain().len()))
        .map_err(|error| format!("{:?}", error.kind()))
}

fn parse_huge(count: u32) -> Result<String, String> {
    let mut json = String::from("[\"root\"");
    for index in 0..count {
        json.push_str(&format!(",{{\"text\":\"{index}\"}}"));
    }
    json.push(']');
    Component::from_json(&json)
        .map(|component| format!("plain-len {}", component.to_plain().len()))
        .map_err(|error| format!("{:?}", error.kind()))
}

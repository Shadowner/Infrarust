use std::hint::black_box;

use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct PerfProbe;

const MODIFY_HOST: &str = "modify.perf";

fn repeat(event: &NamedEvent) -> u32 {
    event
        .text()
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(1)
}

fn answer(event: &mut NamedEvent, total: u64) {
    event.respond_text(total.to_string());
}

fn on_named_loop(
    ctx: &Context,
    name: &str,
    mut body: impl FnMut(u32) -> u64 + 'static,
) -> Result<(), PluginError> {
    ctx.on_named(name, EventPriority::Normal, move |event| {
        let times = repeat(event);
        let mut total = 0u64;
        for i in 0..times {
            total = total.wrapping_add(body(i));
        }
        answer(event, total);
    })?;
    Ok(())
}

struct PassFilter;

const TRACE_PACKET: i32 = 0x20;

impl CodecFilter for PassFilter {
    fn filter(&mut self, _ctx: &CodecContext, packet: &mut Packet, _out: &mut Injections) -> Verdict {
        if packet.id() == TRACE_PACKET {
            trace!("perf probe codec trace for packet {}", packet.id());
        }
        Verdict::Pass
    }
}

#[plugin(id = "perf-probe", name = "Perf Probe Fixture")]
impl Plugin for PerfProbe {
    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
        ctx.on::<PlayerClientBrandEvent>(EventPriority::Normal, |event| {
            black_box(&event.brand);
        })?;
        ctx.on::<ProxyPingEvent>(EventPriority::Normal, |event| {
            if event.virtual_host.as_deref() == Some(MODIFY_HOST) {
                event.response_mut().max_players += 1;
            }
        })?;
        ctx.on::<GameProfileRequestEvent>(EventPriority::Normal, |event| {
            if event.virtual_host.as_deref() == Some(MODIFY_HOST) {
                event.profile_mut().username.push('_');
            }
        })?;
        ctx.on_named("perf.noop", EventPriority::Normal, |_| {})?;
        ctx.on_named("perf.trap", EventPriority::Normal, |_| {
            panic!("perf probe trap on demand");
        })?;
        on_named_loop(ctx, "perf.players.count", |_| u64::from(Players::count()))?;
        on_named_loop(ctx, "perf.players.list", |_| Players::list().len() as u64)?;
        on_named_loop(ctx, "perf.config.get", |_| {
            Config::get("perf.key").ok().flatten().map_or(0, |value| value.len() as u64)
        })?;
        on_named_loop(ctx, "perf.log.trace", |i| {
            trace!("perf probe trace line {i}");
            1
        })?;
        on_named_loop(ctx, "perf.log.info", |i| {
            info!("perf probe info line {i}");
            1
        })?;
        on_named_loop(ctx, "perf.spin", |i| {
            let mut x = u64::from(i);
            for step in 0..1_000u64 {
                x = black_box(x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(step));
            }
            x & 1
        })?;
        Ok(())
    }

    fn register_codec_filters(reg: &mut CodecRegistrar) {
        reg.add("perf-pass", FilterPriority::Normal, |_init| Box::new(PassFilter));
    }
}

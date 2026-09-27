use std::cell::RefCell;
use std::time::Duration;

use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct Counts {
    post_login: u64,
    disconnect: u64,
    chat: u64,
    pre_connect: u64,
    connected: u64,
    wping: u64,
    wswitch: u64,
    wswitch_failed: u64,
    ticks: u64,
    recovered_attempt: u32,
}

thread_local! {
    static COUNTS: RefCell<Counts> = RefCell::new(Counts::default());
}

fn with_counts(update: impl FnOnce(&mut Counts)) {
    COUNTS.with(|counts| update(&mut counts.borrow_mut()));
}

fn dump() {
    let text = COUNTS.with(|counts| {
        let c = counts.borrow();
        format!(
            "post_login={}\ndisconnect={}\nchat={}\npre_connect={}\nconnected={}\nwping={}\nwswitch={}\nwswitch_failed={}\nticks={}\nrecovered_attempt={}\n",
            c.post_login,
            c.disconnect,
            c.chat,
            c.pre_connect,
            c.connected,
            c.wping,
            c.wswitch,
            c.wswitch_failed,
            c.ticks,
            c.recovered_attempt
        )
    });
    let _ = std::fs::write("/counts.tmp", text);
    let _ = std::fs::rename("/counts.tmp", "/counts.txt");
}

#[derive(Default)]
struct SoakWitness;

#[plugin(id = "soak-witness", name = "Soak Witness")]
impl Plugin for SoakWitness {
    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
        if let Some(EnableReason::Recovered(info)) = ctx.enable_reason() {
            let attempt = info.attempt;
            with_counts(|c| c.recovered_attempt = attempt);
        }
        ctx.on::<PostLoginEvent>(EventPriority::Normal, |_| with_counts(|c| c.post_login += 1))?;
        ctx.on::<DisconnectEvent>(EventPriority::Normal, |_| with_counts(|c| c.disconnect += 1))?;
        ctx.on::<ChatMessageEvent>(EventPriority::Normal, |_| with_counts(|c| c.chat += 1))?;
        ctx.on::<ServerPreConnectEvent>(EventPriority::Normal, |_| {
            with_counts(|c| c.pre_connect += 1);
        })?;
        ctx.on::<ServerConnectedEvent>(EventPriority::Normal, |_| {
            with_counts(|c| c.connected += 1);
        })?;
        ctx.command("wping")
            .description("Soak witness round trip")
            .handler(|invocation| {
                with_counts(|c| c.wping += 1);
                let token = invocation.args.first().cloned().unwrap_or_default();
                let _ = invocation.reply(Component::text(format!("wpong {token}")));
            })
            .register()?;
        ctx.command("wswitch")
            .description("Soak witness server switch through the host")
            .handler(|invocation| {
                with_counts(|c| c.wswitch += 1);
                let target = invocation.args.first().cloned().unwrap_or_default();
                let token = invocation.args.get(1).cloned().unwrap_or_default();
                let Some(player) = invocation.player() else {
                    return;
                };
                let outcome = player.handle().switch_server(target.as_str());
                if outcome.is_err() {
                    with_counts(|c| c.wswitch_failed += 1);
                }
                let label = match outcome {
                    Ok(()) => "ok".to_string(),
                    Err(e) => format!("err {e}"),
                };
                let _ = invocation.reply(Component::text(format!("wswitched {token} {label}")));
            })
            .register()?;
        ctx.interval(Duration::from_secs(2), || {
            with_counts(|c| c.ticks += 1);
            dump();
        })?;
        dump();
        Ok(())
    }

    fn on_disable(&self, _ctx: &Context) -> Result<(), PluginError> {
        dump();
        Ok(())
    }
}

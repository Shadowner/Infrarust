use std::fs::OpenOptions;
use std::io::Write;

use infrarust_plugin_sdk::prelude::*;

const LOG: &str = "claims.log";

#[derive(Default)]
struct ClaimProbe;

#[plugin(id = "claim-probe", name = "Connection Claim Probe Fixture")]
impl Plugin for ClaimProbe {
    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
        let recovered = matches!(ctx.enable_reason(), Some(EnableReason::Recovered(_)));
        let mut lines = vec![format!(
            "enable {}",
            if recovered { "recovered" } else { "initial" }
        )];
        let prefixes: &[&[u8]] = if recovered {
            &[b"AMOK"]
        } else {
            &[b"AMMH", b"AMOK", &[0x10, 0x00]]
        };
        for prefix in prefixes {
            let answer = ConnectionClaims::register_prefix(prefix);
            lines.push(format!("register {prefix:?} {}", answer_text(answer)));
        }
        lines.push(format!(
            "unregister-unknown {}",
            answer_text(ConnectionClaims::unregister_prefix(b"NOPE"))
        ));
        lines.push(format!(
            "subscribe raw-connection {}",
            answer_text(ctx.on::<RawConnectionEvent>(EventPriority::NORMAL, raw_connection))
        ));
        lines.push(format!(
            "subscribe login-claim {}",
            answer_text(ctx.on::<LoginClaimEvent>(EventPriority::NORMAL, login_claim))
        ));
        lines.push(format!(
            "subscribe login-plugin-request {}",
            answer_text(
                ctx.on::<LoginPluginRequestEvent>(EventPriority::NORMAL, login_plugin_request)
            )
        ));
        lines.push(format!(
            "subscribe login-plugin-response {}",
            answer_text(
                ctx.on::<LoginPluginResponseEvent>(EventPriority::NORMAL, login_plugin_response)
            )
        ));
        lines.push(format!(
            "subscribe connection-handshake {}",
            answer_text(ctx.on::<ConnectionHandshakeEvent>(EventPriority::NORMAL, handshake))
        ));
        ctx.command("claimtrap")
            .description("Traps on purpose so the host recovers the plugin")
            .handler(|_| panic!("claim-probe traps on purpose"))
            .register()?;
        append(&lines)?;
        Ok(())
    }
}

fn raw_connection(event: &mut RawConnectionEvent) {
    match event.data.get(event.prefix.len()).copied() {
        Some(b'P') => event.pass(),
        Some(b'N') => event.need_more(32),
        Some(b'R') => event.reject(),
        Some(b'H') => event.handoff(
            Handoff::to(HandoffTarget::server_port("modded", 30037))
                .with_prefix(b"PRE".to_vec())
                .with_reply(b"OK".to_vec())
                .without_replay()
                .with_proxy_protocol(true),
        ),
        Some(b'A') => event.handoff(Handoff::to(HandoffTarget::address("files.example", 443))),
        Some(b'B') => event.handoff(Handoff::to(HandoffTarget::address("blocked.example", 443))),
        _ => {}
    }
}

fn login_claim(event: &mut LoginClaimEvent) {
    match event.username.as_str() {
        "pass" => event.pass(),
        "route" => event.handoff(Handoff::route()),
        "server" => event.handoff(
            Handoff::to(HandoffTarget::server("modded"))
                .with_reply(event.login_start.clone())
                .with_proxy_protocol(false),
        ),
        "mismatch" if !event.layout_matches => {
            event.handoff(Handoff::to(HandoffTarget::server("lenient")));
        }
        _ => {}
    }
}

fn login_plugin_request(event: &mut LoginPluginRequestEvent) {
    match event.channel.as_str() {
        "probe:forward" => event.forward(),
        "probe:replace" => {
            let reversed: Vec<u8> = event.data.iter().rev().copied().collect();
            event.replace(reversed);
        }
        "probe:respond" if event.client_reachable => event.respond(b"answer".to_vec()),
        "probe:respond" => event.respond(b"unreachable".to_vec()),
        "probe:none" => event.respond_not_understood(),
        _ => {}
    }
}

fn login_plugin_response(event: &mut LoginPluginResponseEvent) {
    if event.channel == "probe:replace" {
        let mut data = event.message_id.to_be_bytes().to_vec();
        data.extend_from_slice(&event.data);
        event.replace(!event.successful, data);
    }
}

fn handshake(event: &mut ConnectionHandshakeEvent) {
    if event.raw_host == "handoff.test" {
        event.handoff(Handoff::to(HandoffTarget::server("modded")).with_prefix(b"HS".to_vec()));
    }
}

fn answer_text<T>(answer: Result<T, Error>) -> String {
    match answer {
        Ok(_) => "ok".to_owned(),
        Err(error) => error.kind().to_string(),
    }
}

fn append(lines: &[String]) -> std::io::Result<()> {
    let mut log = OpenOptions::new().create(true).append(true).open(LOG)?;
    for line in lines {
        writeln!(log, "{line}")?;
    }
    Ok(())
}

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use infrarust_api::limbo::{HandlerResult, LimboEntryContext, SessionEndReason};
use infrarust_api::player::Player;
use infrarust_api::services::ban_service::{BanEntry, BanTarget, IpNet};
use infrarust_api::services::config_service::{ProxyMode, ServerConfig};
use infrarust_api::services::server_manager::ServerState;
use infrarust_api::types::{Component, GameProfile, RawPacket, ServerAddress, ServerId, TitleData};
use infrarust_plugin_wit::arena::ArenaError;

use crate::bindings::infrarust::plugin::ban_service as wb;
use crate::bindings::infrarust::plugin::config_service as wc;
use crate::bindings::infrarust::plugin::limbo as wl;
use crate::bindings::infrarust::plugin::types as wt;
use crate::component;
use crate::host_error::{HostResult, host_error};

pub(crate) fn system_time_to_millis(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

pub(crate) fn uuid_to_wit(uuid: uuid::Uuid) -> wt::Uuid {
    let (hi, lo) = uuid.as_u64_pair();
    wt::Uuid { hi, lo }
}

pub(crate) fn uuid_from_wit(uuid: wt::Uuid) -> uuid::Uuid {
    uuid::Uuid::from_u64_pair(uuid.hi, uuid.lo)
}

pub(crate) fn ip_to_wit(ip: IpAddr) -> wt::IpAddress {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, c, d] = v4.octets();
            wt::IpAddress::Ipv4((a, b, c, d))
        }
        IpAddr::V6(v6) => {
            let [a, b, c, d, e, f, g, h] = v6.segments();
            wt::IpAddress::Ipv6((a, b, c, d, e, f, g, h))
        }
    }
}

pub(crate) fn ip_from_wit(ip: wt::IpAddress) -> IpAddr {
    match ip {
        wt::IpAddress::Ipv4((a, b, c, d)) => IpAddr::V4(Ipv4Addr::new(a, b, c, d)),
        wt::IpAddress::Ipv6((a, b, c, d, e, f, g, h)) => {
            IpAddr::V6(Ipv6Addr::new(a, b, c, d, e, f, g, h))
        }
    }
}

pub(crate) fn socket_to_wit(addr: SocketAddr) -> wt::SocketAddress {
    wt::SocketAddress {
        ip: ip_to_wit(addr.ip()),
        port: addr.port(),
    }
}

pub(crate) fn game_profile_to_wit(p: &GameProfile) -> wt::GameProfile {
    wt::GameProfile {
        uuid: uuid_to_wit(p.uuid),
        username: p.username.clone(),
        properties: p
            .properties
            .iter()
            .map(|pp| wt::ProfileProperty {
                name: pp.name.clone(),
                value: pp.value.clone(),
                signature: pp.signature.clone(),
            })
            .collect(),
    }
}

pub(crate) fn player_ref(player: &dyn Player) -> wt::PlayerRef {
    let profile = player.profile();
    wt::PlayerRef {
        id: player.id().as_u64(),
        uuid: uuid_to_wit(profile.uuid),
        username: profile.username.clone(),
    }
}

pub(crate) fn server_ids(servers: &[ServerId]) -> Vec<String> {
    servers.iter().map(|s| s.as_str().to_owned()).collect()
}

pub(crate) fn server_address_to_wit(address: &ServerAddress) -> wt::ServerAddress {
    wt::ServerAddress {
        host: address.host.clone(),
        port: address.port,
    }
}

pub(crate) fn server_state_to_wit(s: ServerState) -> wt::ServerState {
    match s {
        ServerState::Online => wt::ServerState::Online,
        ServerState::Offline => wt::ServerState::Offline,
        ServerState::Starting => wt::ServerState::Starting,
        ServerState::Stopping => wt::ServerState::Stopping,
        ServerState::Sleeping => wt::ServerState::Sleeping,
        ServerState::Crashed => wt::ServerState::Crashed,
        _ => wt::ServerState::Offline,
    }
}

pub(crate) fn proxy_mode_to_wit(m: ProxyMode) -> wt::ProxyMode {
    match m {
        ProxyMode::Passthrough => wt::ProxyMode::Passthrough,
        ProxyMode::ZeroCopy => wt::ProxyMode::ZeroCopy,
        ProxyMode::ClientOnly => wt::ProxyMode::ClientOnly,
        ProxyMode::Offline => wt::ProxyMode::Offline,
        ProxyMode::ServerOnly => wt::ProxyMode::ServerOnly,
        _ => wt::ProxyMode::Passthrough,
    }
}

pub(crate) fn server_config_to_wit(c: &ServerConfig) -> wc::ServerConfig {
    wc::ServerConfig {
        id: c.id.as_str().to_owned(),
        network: c.network.clone(),
        addresses: c.addresses.iter().map(server_address_to_wit).collect(),
        domains: c.domains.clone(),
        proxy_mode: proxy_mode_to_wit(c.proxy_mode),
        limbo_handlers: c.limbo_handlers.clone(),
        max_players: c.max_players,
        disconnect_message: c.disconnect_message.clone(),
        send_proxy_protocol: c.send_proxy_protocol,
        has_server_manager: c.has_server_manager,
    }
}

pub(crate) fn ban_target_from_wit(target: wb::BanTarget) -> HostResult<BanTarget> {
    Ok(match target {
        wb::BanTarget::Ip(ip) => BanTarget::Ip(ip_from_wit(ip)),
        wb::BanTarget::IpRange(range) => {
            BanTarget::IpRange(range.parse::<IpNet>().map_err(|e| {
                host_error(
                    wt::ErrorKind::InvalidArgument,
                    format!("invalid ip range {range:?}: {e}"),
                )
            })?)
        }
        wb::BanTarget::Username(name) => BanTarget::Username(name),
        wb::BanTarget::Uuid(uuid) => BanTarget::Uuid(uuid_from_wit(uuid)),
    })
}

pub(crate) fn ban_target_to_wit(t: &BanTarget) -> wb::BanTarget {
    match t {
        BanTarget::Ip(ip) => wb::BanTarget::Ip(ip_to_wit(*ip)),
        BanTarget::IpRange(net) => wb::BanTarget::IpRange(net.to_string()),
        BanTarget::Username(u) => wb::BanTarget::Username(u.clone()),
        BanTarget::Uuid(u) => wb::BanTarget::Uuid(uuid_to_wit(*u)),
        other => wb::BanTarget::Username(other.to_string()),
    }
}

pub(crate) fn ban_entry_to_wit(e: &BanEntry) -> wb::BanEntry {
    wb::BanEntry {
        id: e.id.clone(),
        target: ban_target_to_wit(&e.target),
        reason: e.reason.clone(),
        source: e.source.to_string(),
        created_at: system_time_to_millis(e.created_at),
        expires_at: e.expires_at.map(system_time_to_millis),
    }
}

pub(crate) fn raw_packet_from_wit(p: wt::RawPacket) -> RawPacket {
    RawPacket::new(p.packet_id, bytes::Bytes::from(p.data))
}

pub(crate) fn title_data_from_wit(t: &wt::TitleData) -> Result<TitleData, ArenaError> {
    Ok(TitleData {
        title: component::from_wit(&t.title)?,
        subtitle: component::from_wit(&t.subtitle)?,
        fade_in_ticks: t.fade_in_ticks,
        stay_ticks: t.stay_ticks,
        fade_out_ticks: t.fade_out_ticks,
    })
}

pub(crate) type TextConverter<'a> =
    &'a mut dyn FnMut(&wt::Component) -> Result<Component, ArenaError>;

pub(crate) fn handler_result_with(
    r: &wl::HandlerResult,
    text: TextConverter<'_>,
) -> Result<HandlerResult, ArenaError> {
    Ok(match r {
        wl::HandlerResult::Accept => HandlerResult::Accept,
        wl::HandlerResult::Deny(c) => HandlerResult::Deny(text(c)?),
        wl::HandlerResult::Hold => HandlerResult::Hold,
        wl::HandlerResult::HoldWithTimeout(t) => HandlerResult::HoldWithTimeout {
            after: Duration::from_millis(t.after_ms),
            on_timeout: Box::new(timeout_outcome_with(&t.on_timeout, text)?),
        },
        wl::HandlerResult::Redirect(s) => HandlerResult::Redirect(ServerId::from(s.as_str())),
        wl::HandlerResult::SendToLimbo(v) => HandlerResult::SendToLimbo(v.clone()),
    })
}

pub(crate) fn handler_result_from_wit(r: &wl::HandlerResult) -> Result<HandlerResult, ArenaError> {
    handler_result_with(r, &mut component::from_wit)
}

pub(crate) fn timeout_outcome_with(
    t: &wl::TimeoutOutcome,
    text: TextConverter<'_>,
) -> Result<HandlerResult, ArenaError> {
    Ok(match t {
        wl::TimeoutOutcome::Accept => HandlerResult::Accept,
        wl::TimeoutOutcome::Deny(c) => HandlerResult::Deny(text(c)?),
        wl::TimeoutOutcome::Redirect(s) => HandlerResult::Redirect(ServerId::from(s.as_str())),
        wl::TimeoutOutcome::SendToLimbo(v) => HandlerResult::SendToLimbo(v.clone()),
    })
}

pub(crate) fn complete_result_from_wit(r: &wl::HandlerResult) -> Result<HandlerResult, ArenaError> {
    Ok(match handler_result_from_wit(r)? {
        HandlerResult::Hold | HandlerResult::HoldWithTimeout { .. } => HandlerResult::Accept,
        other => other,
    })
}

pub(crate) fn session_end_reason_to_wit(r: SessionEndReason) -> wl::SessionEndReason {
    match r {
        SessionEndReason::Disconnected => wl::SessionEndReason::Disconnected,
        SessionEndReason::Released => wl::SessionEndReason::Released,
        SessionEndReason::Kicked => wl::SessionEndReason::Kicked,
        SessionEndReason::Redirected => wl::SessionEndReason::Redirected,
        SessionEndReason::TimedOut => wl::SessionEndReason::TimedOut,
        SessionEndReason::Shutdown => wl::SessionEndReason::Shutdown,
        _ => wl::SessionEndReason::Disconnected,
    }
}

pub(crate) fn limbo_entry_context_to_wit(c: &LimboEntryContext) -> wl::LimboEntryContext {
    match c {
        LimboEntryContext::InitialConnection { target_server } => {
            wl::LimboEntryContext::InitialConnection(target_server.as_str().to_owned())
        }
        LimboEntryContext::KickedFromServer { server, reason } => {
            wl::LimboEntryContext::KickedFromServer((
                server.as_str().to_owned(),
                component::to_wit(reason),
            ))
        }
        LimboEntryContext::PluginRedirect { from_server } => wl::LimboEntryContext::PluginRedirect(
            from_server.as_ref().map(|s| s.as_str().to_owned()),
        ),
        _ => wl::LimboEntryContext::PluginRedirect(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timeout_outcome_from_wit(t: &wl::TimeoutOutcome) -> Result<HandlerResult, ArenaError> {
        timeout_outcome_with(t, &mut component::from_wit)
    }

    fn text(message: &str) -> wt::Component {
        component::to_wit(&Component::text(message))
    }

    #[test]
    fn uuids_and_addresses_round_trip() {
        let uuid: uuid::Uuid = "0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0".parse().unwrap();
        assert_eq!(uuid_from_wit(uuid_to_wit(uuid)), uuid);
        for ip in ["203.0.113.7", "2001:db8::7"] {
            let ip: IpAddr = ip.parse().unwrap();
            assert_eq!(ip_from_wit(ip_to_wit(ip)), ip);
        }
    }

    #[test]
    fn a_ban_range_is_parsed_and_a_bad_one_is_an_argument_error() {
        assert_eq!(
            ban_target_from_wit(wb::BanTarget::IpRange("10.0.0.0/8".into())).unwrap(),
            BanTarget::IpRange("10.0.0.0/8".parse().unwrap())
        );
        let err = ban_target_from_wit(wb::BanTarget::IpRange("nope".into())).unwrap_err();
        assert_eq!(err.kind, wt::ErrorKind::InvalidArgument);
    }

    #[test]
    fn hold_with_timeout_maps_to_native_with_terminal_outcome() {
        let wit = wl::HandlerResult::HoldWithTimeout(wl::HoldTimeout {
            after_ms: 1500,
            on_timeout: wl::TimeoutOutcome::Deny(text("bye")),
        });
        match handler_result_from_wit(&wit).unwrap() {
            HandlerResult::HoldWithTimeout { after, on_timeout } => {
                assert_eq!(after, Duration::from_millis(1500));
                assert!(matches!(*on_timeout, HandlerResult::Deny(_)));
            }
            other => panic!("expected HoldWithTimeout, got {other:?}"),
        }
    }

    #[test]
    fn timeout_outcome_never_yields_a_hold() {
        assert!(matches!(
            timeout_outcome_from_wit(&wl::TimeoutOutcome::Accept),
            Ok(HandlerResult::Accept)
        ));
        assert!(matches!(
            timeout_outcome_from_wit(&wl::TimeoutOutcome::SendToLimbo(vec!["a".to_owned()])),
            Ok(HandlerResult::SendToLimbo(_))
        ));
    }

    #[test]
    fn complete_coerces_holds_to_accept() {
        assert!(matches!(
            complete_result_from_wit(&wl::HandlerResult::Hold),
            Ok(HandlerResult::Accept)
        ));
        let hwt = wl::HandlerResult::HoldWithTimeout(wl::HoldTimeout {
            after_ms: 1,
            on_timeout: wl::TimeoutOutcome::Accept,
        });
        assert!(matches!(
            complete_result_from_wit(&hwt),
            Ok(HandlerResult::Accept)
        ));
        assert!(matches!(
            complete_result_from_wit(&wl::HandlerResult::Redirect("lobby".to_owned())),
            Ok(HandlerResult::Redirect(_))
        ));
    }

    #[test]
    fn a_limbo_deny_with_an_invalid_component_is_refused() {
        let invalid = wl::HandlerResult::Deny(wt::Component { nodes: Vec::new() });
        assert!(matches!(
            handler_result_from_wit(&invalid),
            Err(ArenaError::Empty)
        ));
    }

    #[test]
    fn session_end_reason_maps_each_variant() {
        assert!(matches!(
            session_end_reason_to_wit(SessionEndReason::Released),
            wl::SessionEndReason::Released
        ));
        assert!(matches!(
            session_end_reason_to_wit(SessionEndReason::TimedOut),
            wl::SessionEndReason::TimedOut
        ));
        assert!(matches!(
            session_end_reason_to_wit(SessionEndReason::Disconnected),
            wl::SessionEndReason::Disconnected
        ));
    }
}

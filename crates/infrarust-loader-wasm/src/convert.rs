use std::time::{SystemTime, UNIX_EPOCH};

use infrarust_api::error::{PlayerError, ServiceError};
use infrarust_api::limbo::{HandlerResult, LimboEntryContext};
use infrarust_api::permissions::PermissionLevel;
use infrarust_api::services::ban_service::{BanEntry, BanTarget};
use infrarust_api::services::config_service::{ProxyMode, ServerConfig};
use infrarust_api::services::server_manager::ServerState;
use infrarust_api::types::{Component, GameProfile, RawPacket, ServerId, TitleData};

use crate::bindings::infrarust::plugin::limbo as wl;
use crate::bindings::infrarust::plugin::types as wit;

pub(crate) fn system_time_to_millis(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

pub(crate) fn component_from_wit(s: &str) -> Component {
    Component::from_json(s).unwrap_or_else(|_| Component::text(s))
}

pub(crate) fn component_to_wit(c: &Component) -> String {
    c.to_json()
}

pub(crate) fn game_profile_to_wit(p: &GameProfile) -> wit::GameProfile {
    wit::GameProfile {
        uuid: p.uuid.to_string(),
        username: p.username.clone(),
        properties: p
            .properties
            .iter()
            .map(|pp| wit::ProfileProperty {
                name: pp.name.clone(),
                value: pp.value.clone(),
                signature: pp.signature.clone(),
            })
            .collect(),
    }
}

pub(crate) fn server_state_to_wit(s: ServerState) -> wit::ServerState {
    match s {
        ServerState::Online => wit::ServerState::Online,
        ServerState::Offline => wit::ServerState::Offline,
        ServerState::Starting => wit::ServerState::Starting,
        ServerState::Stopping => wit::ServerState::Stopping,
        ServerState::Sleeping => wit::ServerState::Sleeping,
        ServerState::Crashed => wit::ServerState::Crashed,
        _ => wit::ServerState::Offline,
    }
}

pub(crate) fn proxy_mode_to_wit(m: ProxyMode) -> wit::ProxyMode {
    match m {
        ProxyMode::Passthrough => wit::ProxyMode::Passthrough,
        ProxyMode::ZeroCopy => wit::ProxyMode::ZeroCopy,
        ProxyMode::ClientOnly => wit::ProxyMode::ClientOnly,
        ProxyMode::Offline => wit::ProxyMode::Offline,
        ProxyMode::ServerOnly => wit::ProxyMode::ServerOnly,
        _ => wit::ProxyMode::Passthrough,
    }
}

pub(crate) fn permission_level_to_wit(l: PermissionLevel) -> wit::PermissionLevel {
    match l {
        PermissionLevel::Player => wit::PermissionLevel::Player,
        PermissionLevel::Admin => wit::PermissionLevel::Admin,
    }
}

pub(crate) fn server_config_to_wit(c: &ServerConfig) -> wit::ServerConfig {
    wit::ServerConfig {
        id: c.id.as_str().to_string(),
        network: c.network.clone(),
        addresses: c
            .addresses
            .iter()
            .map(|a| wit::ServerAddress {
                host: a.host.clone(),
                port: a.port,
            })
            .collect(),
        domains: c.domains.clone(),
        proxy_mode: proxy_mode_to_wit(c.proxy_mode),
        limbo_handlers: c.limbo_handlers.clone(),
        max_players: c.max_players,
        disconnect_message: c.disconnect_message.clone(),
        send_proxy_protocol: c.send_proxy_protocol,
        has_server_manager: c.has_server_manager,
    }
}

pub(crate) fn ban_target_from_wit(t: &wit::BanTarget) -> Option<BanTarget> {
    match t {
        wit::BanTarget::Ip(s) => s.parse().ok().map(BanTarget::Ip),
        wit::BanTarget::Username(s) => Some(BanTarget::Username(s.clone())),
        wit::BanTarget::Uuid(s) => uuid::Uuid::parse_str(s).ok().map(BanTarget::Uuid),
    }
}

pub(crate) fn ban_target_to_wit(t: &BanTarget) -> wit::BanTarget {
    match t {
        BanTarget::Ip(ip) => wit::BanTarget::Ip(ip.to_string()),
        BanTarget::Username(u) => wit::BanTarget::Username(u.clone()),
        BanTarget::Uuid(u) => wit::BanTarget::Uuid(u.to_string()),
        _ => wit::BanTarget::Username(String::new()),
    }
}

pub(crate) fn ban_entry_to_wit(e: &BanEntry) -> wit::BanEntry {
    wit::BanEntry {
        target: ban_target_to_wit(&e.target),
        reason: e.reason.clone(),
        expires_at: e.expires_at.map(system_time_to_millis),
        created_at: system_time_to_millis(e.created_at),
        source: e.source.clone(),
    }
}

pub(crate) fn player_error_to_wit(e: PlayerError) -> wit::PlayerError {
    match e {
        PlayerError::NotActive => wit::PlayerError::NotActive,
        PlayerError::Disconnected => wit::PlayerError::Disconnected,
        PlayerError::SendFailed(s) => wit::PlayerError::SendFailed(s),
        PlayerError::ServerNotFound(s) => wit::PlayerError::ServerNotFound(s),
        PlayerError::SwitchFailed(s) => wit::PlayerError::SwitchFailed(s),
        _ => wit::PlayerError::SendFailed("unknown error".to_string()),
    }
}

pub(crate) fn service_error_to_wit(e: ServiceError) -> wit::ServiceError {
    match e {
        ServiceError::NotFound(s) => wit::ServiceError::NotFound(s),
        ServiceError::OperationFailed(s) => wit::ServiceError::OperationFailed(s),
        ServiceError::Unavailable(s) => wit::ServiceError::Unavailable(s),
        _ => wit::ServiceError::OperationFailed("unknown error".to_string()),
    }
}

pub(crate) fn raw_packet_from_wit(p: wit::RawPacket) -> RawPacket {
    RawPacket::new(p.packet_id, bytes::Bytes::from(p.data))
}

pub(crate) fn raw_packet_to_wit(p: &RawPacket) -> wit::RawPacket {
    wit::RawPacket {
        packet_id: p.packet_id,
        data: p.data.to_vec(),
    }
}

pub(crate) fn title_data_from_wit(t: wit::TitleData) -> TitleData {
    TitleData {
        title: component_from_wit(&t.title),
        subtitle: component_from_wit(&t.subtitle),
        fade_in_ticks: t.fade_in_ticks,
        stay_ticks: t.stay_ticks,
        fade_out_ticks: t.fade_out_ticks,
    }
}

pub(crate) fn handler_result_from_wit(r: wl::HandlerResult) -> HandlerResult {
    match r {
        wl::HandlerResult::Accept => HandlerResult::Accept,
        wl::HandlerResult::Deny(c) => HandlerResult::Deny(component_from_wit(&c)),
        wl::HandlerResult::Hold => HandlerResult::Hold,
        wl::HandlerResult::Redirect(s) => HandlerResult::Redirect(ServerId::from(s)),
        wl::HandlerResult::SendToLimbo(v) => HandlerResult::SendToLimbo(v),
    }
}

pub(crate) fn limbo_entry_context_to_wit(c: &LimboEntryContext) -> wl::LimboEntryContext {
    match c {
        LimboEntryContext::InitialConnection { target_server } => {
            wl::LimboEntryContext::InitialConnection(target_server.as_str().to_string())
        }
        LimboEntryContext::KickedFromServer { server, reason } => {
            wl::LimboEntryContext::KickedFromServer((
                server.as_str().to_string(),
                component_to_wit(reason),
            ))
        }
        LimboEntryContext::PluginRedirect { from_server } => wl::LimboEntryContext::PluginRedirect(
            from_server.as_ref().map(|s| s.as_str().to_string()),
        ),
        _ => wl::LimboEntryContext::PluginRedirect(None),
    }
}

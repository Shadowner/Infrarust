use infrarust_api::permissions::Capability;
use infrarust_api::services::proxy_info::{ProxyInfo, UnknownDomainBehavior};

use crate::bindings::infrarust::plugin::proxy_info as wi;
use crate::bindings::infrarust::plugin::types as wt;
use crate::convert;
use crate::store_state::PluginStoreState;

fn millis(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

pub(crate) fn details(info: &ProxyInfo) -> wi::ProxyDetails {
    wi::ProxyDetails {
        version: info.version.clone(),
        bind: convert::socket_to_wit(info.bind),
        max_connections: info.max_connections,
        connect_timeout_ms: millis(info.connect_timeout),
        receive_proxy_protocol: info.receive_proxy_protocol,
        worker_threads: u32::try_from(info.worker_threads).unwrap_or(u32::MAX),
        so_reuseport: info.so_reuseport,
        rate_limit: wi::RateLimitInfo {
            max_connections: info.rate_limit.max_connections,
            window_ms: millis(info.rate_limit.window),
            status_max: info.rate_limit.status_max,
            status_window_ms: millis(info.rate_limit.status_window),
        },
        status_cache: wi::StatusCacheInfo {
            ttl_ms: millis(info.status_cache.ttl),
            max_entries: u64::try_from(info.status_cache.max_entries).unwrap_or(u64::MAX),
        },
        keepalive: wi::KeepaliveInfo {
            time_ms: millis(info.keepalive.time),
            interval_ms: millis(info.keepalive.interval),
            retries: info.keepalive.retries,
        },
        telemetry_enabled: info.telemetry_enabled,
        docker_enabled: info.docker_enabled,
        web_api_enabled: info.web_api_enabled,
        web_ui_enabled: info.web_ui_enabled,
        unknown_domain_behavior: match info.unknown_domain_behavior {
            UnknownDomainBehavior::Drop => wi::UnknownDomainBehavior::Drop,
            _ => wi::UnknownDomainBehavior::DefaultMotd,
        },
    }
}

pub(crate) const fn capability_to_wit(capability: Capability) -> Option<wt::Capability> {
    Some(match capability {
        Capability::EventBus => wt::Capability::EventBus,
        Capability::PlayerRead => wt::Capability::PlayerRead,
        Capability::PlayerWrite => wt::Capability::PlayerWrite,
        Capability::RawPacket => wt::Capability::RawPacket,
        Capability::ServerManage => wt::Capability::ServerManage,
        Capability::Ban => wt::Capability::Ban,
        Capability::Command => wt::Capability::Command,
        Capability::Scheduler => wt::Capability::Scheduler,
        Capability::ConfigRead => wt::Capability::ConfigRead,
        Capability::ConfigWrite => wt::Capability::ConfigWrite,
        Capability::CodecFilter => wt::Capability::CodecFilter,
        Capability::TransportFilter => wt::Capability::TransportFilter,
        Capability::Limbo => wt::Capability::Limbo,
        Capability::VirtualBackend => wt::Capability::VirtualBackend,
        Capability::PermissionProvider => wt::Capability::PermissionProvider,
        Capability::FilesystemExtended => wt::Capability::FilesystemExtended,
        Capability::Network => wt::Capability::Network,
        Capability::ChatIntercept => wt::Capability::ChatIntercept,
        Capability::BanProvider => wt::Capability::BanProvider,
        Capability::PluginMessaging => wt::Capability::PluginMessaging,
        _ => return None,
    })
}

impl wi::Host for PluginStoreState {
    async fn details(&mut self) -> wasmtime::Result<wi::ProxyDetails> {
        Ok(self.ctx().map_or_else(
            || details(&ProxyInfo::default()),
            |ctx| details(ctx.proxy_info()),
        ))
    }

    async fn granted_capabilities(&mut self) -> wasmtime::Result<Vec<wt::Capability>> {
        Ok(Capability::ALL
            .into_iter()
            .filter(|capability| self.capabilities().has(*capability))
            .filter_map(capability_to_wit)
            .collect())
    }
}

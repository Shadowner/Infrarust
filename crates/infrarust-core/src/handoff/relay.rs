use std::sync::Arc;
use std::time::Duration;

use infrarust_api::handoff::{Handoff, HandoffTarget};
use infrarust_api::types::ServerId;
use infrarust_config::{ServerAddress, ServerConfig};
use infrarust_transport::{
    BackendConnection, BackendConnector, CopyForwarder, ForwardResult, Forwarder, TransportError,
};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio_util::sync::CancellationToken;

use crate::error::CoreError;
use crate::loadbalancer::{PendingTicket, select_backend_addresses};
use crate::middleware::backend_selection::BackendTargets;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::types::{HandshakeData, RoutingData};
use crate::services::ProxyServices;
use crate::util::domain_rewrite::handoff_handshake;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum HandoffLevel {
    Raw,
    Handshake,
    Login,
}

impl HandoffLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Handshake => "handshake",
            Self::Login => "login",
        }
    }
}

pub const MAX_HANDOFF_BYTES: usize = 64 * 1024;

const ADDRESS_LABEL: &str = "handoff:address";

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HandoffError {
    #[error("the handoff {what} holds {len} bytes, more than the {MAX_HANDOFF_BYTES} allowed")]
    TooLarge { what: &'static str, len: usize },
    #[error("the connection has no route yet, so it cannot be handed off to it")]
    NoRoute,
    #[error("no configured server is named `{0}`")]
    UnknownServer(ServerId),
    #[error("server `{0}` has no backend address")]
    NoAddress(String),
    #[error("replaying the opening needs the parsed handshake")]
    MissingHandshake,
    #[error("could not rebuild the handshake for the handoff target: {0}")]
    Handshake(#[source] Box<CoreError>),
    #[error("the proxy is shutting down")]
    Shutdown,
    #[error("could not reach the handoff target: {0}")]
    Connect(#[from] TransportError),
    #[error("handoff i/o failed: {0}")]
    Io(#[from] std::io::Error),
}

struct Resolved {
    label: String,
    addresses: Vec<ServerAddress>,
    timeout: Duration,
    proxy_protocol: bool,
    server: Option<Arc<ServerConfig>>,
    balanced: bool,
    ticket: Option<PendingTicket>,
}

pub struct HandoffRelay {
    services: ProxyServices,
    connector: Arc<BackendConnector>,
    zero_copy: bool,
    #[cfg(feature = "telemetry")]
    metrics: Option<Arc<crate::telemetry::ProxyMetrics>>,
}

impl HandoffRelay {
    pub fn new(services: ProxyServices, connector: Arc<BackendConnector>) -> Self {
        Self {
            services,
            connector,
            zero_copy: cfg!(target_os = "linux"),
            #[cfg(feature = "telemetry")]
            metrics: None,
        }
    }

    #[must_use]
    pub const fn with_zero_copy(mut self, zero_copy: bool) -> Self {
        self.zero_copy = zero_copy && cfg!(target_os = "linux");
        self
    }

    #[cfg(feature = "telemetry")]
    #[must_use]
    pub fn with_metrics(mut self, metrics: Arc<crate::telemetry::ProxyMetrics>) -> Self {
        self.metrics = Some(metrics);
        self
    }

    pub async fn relay(
        &self,
        mut ctx: ConnectionContext,
        handoff: Handoff,
        level: HandoffLevel,
        shutdown: CancellationToken,
    ) -> Result<ForwardResult, HandoffError> {
        bounded("prefix", &handoff.prefix)?;
        bounded("reply", &handoff.reply)?;
        let resolved = self.resolve(&ctx, &handoff)?;
        let replay = if handoff.replay {
            replay_bytes(&ctx, level, resolved.server.as_deref())?
        } else {
            Vec::new()
        };
        let backend = tokio::select! {
            biased;
            () = shutdown.cancelled() => return Err(HandoffError::Shutdown),
            backend = self.connect(&ctx, &resolved, &handoff) => backend?,
        };
        let Resolved { label, ticket, .. } = resolved;
        drop(ticket);
        let address = backend.server_address().clone();
        let mut target = backend.into_stream();
        let opening = [&handoff.prefix[..], &replay[..]];
        tokio::select! {
            biased;
            () = shutdown.cancelled() => return Err(HandoffError::Shutdown),
            written = write_all(&mut target, &opening) => written?,
        }
        let reply = [&handoff.reply[..]];
        tokio::select! {
            biased;
            () = shutdown.cancelled() => return Err(HandoffError::Shutdown),
            written = write_all(ctx.stream_mut(), &reply) => written?,
        }
        tracing::debug!(
            client = %ctx.client_addr(),
            target = %handoff.target,
            server = %label,
            address = %address,
            level = level.as_str(),
            replayed = replay.len(),
            "connection handed off"
        );

        #[cfg(feature = "telemetry")]
        if let Some(metrics) = &self.metrics {
            metrics.record_connection_start(&label, "handoff");
        }

        let client = ctx.take_stream();
        let result = self.forwarder().forward(client, target, shutdown).await;

        #[cfg(feature = "telemetry")]
        if let Some(metrics) = &self.metrics {
            metrics.record_connection_end(
                ctx.connection_duration().as_secs_f64(),
                &label,
                "handoff",
            );
        }

        tracing::debug!(
            client = %ctx.client_addr(),
            server = %label,
            address = %address,
            level = level.as_str(),
            c2b = result.client_to_backend,
            b2c = result.backend_to_client,
            reason = ?result.reason,
            "handed-off connection ended"
        );
        drop(ctx);
        Ok(result)
    }

    fn forwarder(&self) -> Box<dyn Forwarder> {
        #[cfg(target_os = "linux")]
        if self.zero_copy {
            return Box::new(infrarust_transport::SpliceForwarder::new());
        }
        Box::new(CopyForwarder)
    }

    fn resolve(
        &self,
        ctx: &ConnectionContext,
        handoff: &Handoff,
    ) -> Result<Resolved, HandoffError> {
        let explicit = handoff.proxy_protocol;
        match &handoff.target {
            HandoffTarget::Route => {
                let routing = ctx
                    .extensions
                    .get::<RoutingData>()
                    .cloned()
                    .ok_or(HandoffError::NoRoute)?;
                let chosen = ctx
                    .extensions
                    .get::<BackendTargets>()
                    .map(|targets| targets.addresses.to_vec());
                self.balanced(&routing, chosen, explicit)
            }
            HandoffTarget::Server(server) => {
                let routing = self.route_of(server)?;
                self.balanced(&routing, None, explicit)
            }
            HandoffTarget::ServerPort { server, port } => {
                let routing = self.route_of(server)?;
                let mut resolved = self.balanced(&routing, None, explicit)?;
                drop(resolved.ticket.take());
                for address in &mut resolved.addresses {
                    address.port = *port;
                }
                resolved.balanced = false;
                Ok(resolved)
            }
            HandoffTarget::Address { host, port } => Ok(Resolved {
                label: ADDRESS_LABEL.to_owned(),
                addresses: vec![ServerAddress {
                    host: host.clone(),
                    port: *port,
                }],
                timeout: self.connector.default_timeout,
                proxy_protocol: explicit.unwrap_or(false),
                server: None,
                balanced: false,
                ticket: None,
            }),
            _ => Err(HandoffError::NoRoute),
        }
    }

    fn route_of(&self, server: &ServerId) -> Result<RoutingData, HandoffError> {
        let (server_config, load_balancer) = self
            .services
            .domain_router
            .find_route_by_server_id(server.as_str())
            .ok_or_else(|| HandoffError::UnknownServer(server.clone()))?;
        Ok(RoutingData {
            server_config,
            config_id: server.as_str().to_owned(),
            load_balancer,
        })
    }

    fn balanced(
        &self,
        routing: &RoutingData,
        chosen: Option<Vec<ServerAddress>>,
        explicit: Option<bool>,
    ) -> Result<Resolved, HandoffError> {
        let services = &self.services;
        let config = &routing.server_config;
        let (addresses, ticket) = match chosen {
            Some(addresses) => (addresses, None),
            None => {
                let addresses = select_backend_addresses(
                    config,
                    routing.load_balancer.as_ref(),
                    services.pending_backends.as_ref(),
                    services.backend_health.as_ref(),
                )
                .to_vec();
                let ticket = addresses
                    .first()
                    .map(|first| services.pending_backends.reserve(first));
                (addresses, ticket)
            }
        };
        if addresses.is_empty() {
            return Err(HandoffError::NoAddress(routing.config_id.clone()));
        }
        Ok(Resolved {
            label: routing.config_id.clone(),
            addresses,
            timeout: config
                .timeouts
                .as_ref()
                .map_or(self.connector.default_timeout, |t| t.connect),
            proxy_protocol: explicit.unwrap_or(config.send_proxy_protocol),
            server: Some(Arc::clone(config)),
            balanced: true,
            ticket,
        })
    }

    async fn connect(
        &self,
        ctx: &ConnectionContext,
        resolved: &Resolved,
        handoff: &Handoff,
    ) -> Result<BackendConnection, HandoffError> {
        let info = ctx.connection_info();
        if resolved.balanced {
            return Ok(self
                .connector
                .connect(
                    &resolved.label,
                    &resolved.addresses,
                    Some(resolved.timeout),
                    resolved.proxy_protocol,
                    &info,
                )
                .await?);
        }
        let mut last = None;
        for address in &resolved.addresses {
            match self
                .connector
                .connect_one(address, resolved.timeout, resolved.proxy_protocol, &info)
                .await
            {
                Ok(backend) => return Ok(backend),
                Err(error) => {
                    tracing::debug!(
                        target = %handoff.target,
                        address = %address,
                        error = %error,
                        "handoff target unreachable"
                    );
                    last = Some(error);
                }
            }
        }
        Err(last
            .unwrap_or_else(|| TransportError::AllBackendsFailed {
                server_id: resolved.label.clone(),
            })
            .into())
    }
}

const fn bounded(what: &'static str, bytes: &[u8]) -> Result<(), HandoffError> {
    if bytes.len() > MAX_HANDOFF_BYTES {
        return Err(HandoffError::TooLarge {
            what,
            len: bytes.len(),
        });
    }
    Ok(())
}

fn replay_bytes(
    ctx: &ConnectionContext,
    level: HandoffLevel,
    server: Option<&ServerConfig>,
) -> Result<Vec<u8>, HandoffError> {
    if level == HandoffLevel::Raw {
        return Ok(ctx.buffered_data.to_vec());
    }
    let handshake = ctx
        .extensions
        .get::<HandshakeData>()
        .ok_or(HandoffError::MissingHandshake)?;
    let mut replay = handoff_handshake(handshake, server)
        .map_err(|error| HandoffError::Handshake(Box::new(error)))?;
    for packet in handshake.raw_packets.iter().skip(1) {
        replay.extend_from_slice(packet);
    }
    replay.extend_from_slice(&ctx.buffered_data);
    Ok(replay)
}

async fn write_all(stream: &mut TcpStream, parts: &[&[u8]]) -> std::io::Result<()> {
    if parts.iter().all(|part| part.is_empty()) {
        return Ok(());
    }
    for part in parts {
        stream.write_all(part).await?;
    }
    stream.flush().await
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use tokio::net::TcpListener;

    use super::*;
    use crate::limbo::test_helpers::test_proxy_services;
    use crate::provider::ProviderId;

    #[tokio::test]
    async fn metric_labels_never_carry_a_plugin_chosen_address_or_port() {
        let services = test_proxy_services();
        services.domain_router.add(
            ProviderId::file("modded"),
            toml::from_str(
                "name = \"modded\"\ndomains = [\"modded.test\"]\naddresses = [\"127.0.0.1:25565\"]\n",
            )
            .unwrap(),
        );
        let relay = HandoffRelay::new(
            services.clone(),
            Arc::new(BackendConnector::new(
                Duration::from_secs(2),
                infrarust_config::KeepaliveConfig::default(),
            )),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local = listener.local_addr().unwrap();
        let _peer = TcpStream::connect(local).await.unwrap();
        let (stream, remote) = listener.accept().await.unwrap();
        let mut ctx = ConnectionContext::new_for_test(stream, remote, remote.ip(), local);
        let (server_config, load_balancer) = services
            .domain_router
            .find_route_by_server_id("modded")
            .unwrap();
        ctx.extensions.insert(RoutingData {
            server_config,
            config_id: "modded".into(),
            load_balancer,
        });

        for (target, label) in [
            (HandoffTarget::Route, "modded"),
            (HandoffTarget::server("modded"), "modded"),
            (HandoffTarget::server_port("modded", 30037), "modded"),
            (
                HandoffTarget::address("files.example", 30037),
                "handoff:address",
            ),
            (
                HandoffTarget::address("198.51.100.7", 40001),
                "handoff:address",
            ),
        ] {
            let resolved = relay.resolve(&ctx, &Handoff::to(target.clone())).unwrap();
            assert_eq!(resolved.label, label, "{target}");
        }
    }
}

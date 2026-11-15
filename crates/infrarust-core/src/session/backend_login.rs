use infrarust_config::{ServerAddress, ServerConfig};
use infrarust_protocol::packets::login::SLoginAcknowledged;
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};

use crate::error::CoreError;
use crate::forwarding::domain_rewrite::rewrite_handshake;
use crate::forwarding::velocity::VelocityForwarding;
use crate::forwarding::{ForwardingHandler, build_handshake_for_backend};
use crate::session::backend_bridge::BackendBridge;
use crate::session::context::{SessionContext, SessionIo};
use crate::session::kick::BackendKick;
use crate::session::login_bridge::bridge_login;

#[derive(Debug, thiserror::Error)]
pub(crate) enum ConnectError {
    #[error("backend kicked the player: {0}")]
    Kicked(Box<BackendKick>),

    #[error("the client left during the login")]
    ClientLeft,

    #[error("the session ended during the backend login")]
    Cancelled,

    #[error("transport error: {0}")]
    Unreachable(#[from] infrarust_transport::TransportError),

    #[error(transparent)]
    Failed(#[from] CoreError),
}

impl From<infrarust_protocol::ProtocolError> for ConnectError {
    fn from(error: infrarust_protocol::ProtocolError) -> Self {
        Self::Failed(error.into())
    }
}

pub(crate) enum Login<'a> {
    Relay,
    Proxied,
    Bridged(&'a mut SessionIo),
}

pub(crate) async fn connect_backend(
    ctx: &SessionContext<'_>,
    server: &str,
    config: &ServerConfig,
    addresses: &[ServerAddress],
    login: Login<'_>,
) -> Result<BackendBridge, ConnectError> {
    let version = ctx.version();
    let connection = ctx
        .backend_connector
        .connect(
            server,
            addresses,
            config.timeouts.as_ref().map(|t| t.connect),
            config.send_proxy_protocol,
            &ctx.connection_info,
        )
        .await?;
    let address = connection.server_address().clone();
    let mut backend =
        BackendBridge::new(connection.into_stream(), version).with_server_address(address);
    backend.set_cancel(ctx.token.clone());

    match login {
        Login::Relay => {
            backend.send_initial_packets(&ctx.handshake, config).await?;
        }
        Login::Proxied => log_in(ctx, None, &mut backend, server, config).await?,
        Login::Bridged(io) => log_in(ctx, Some(io), &mut backend, server, config).await?,
    }

    Ok(backend)
}

async fn log_in(
    ctx: &SessionContext<'_>,
    io: Option<&mut SessionIo>,
    backend: &mut BackendBridge,
    server: &str,
    config: &ServerConfig,
) -> Result<(), ConnectError> {
    let handler = start_login(ctx, config, backend).await?;
    let data = ctx.forwarding_data();
    let velocity = velocity_secret(ctx, &handler).map(|secret| VelocityForwarding {
        data: &data,
        secret,
    });
    bridge_login(ctx, io, backend, server, config, velocity).await
}

async fn start_login(
    ctx: &SessionContext<'_>,
    config: &ServerConfig,
    backend: &mut BackendBridge,
) -> Result<ForwardingHandler, CoreError> {
    let handler = ctx.services.resolve_forwarding_handler(config)?;
    if handler.modifies_handshake() {
        let mut handshake = build_handshake_for_backend(&ctx.handshake, config);
        handler.apply_handshake(&mut handshake, &ctx.forwarding_data());
        backend.send_handshake(&handshake).await?;
    } else {
        backend
            .write_unframed(&rewrite_handshake(&ctx.handshake, config)?)
            .await?;
    }
    backend
        .send_login_start(ctx.username(), ctx.registry())
        .await?;
    Ok(handler)
}

fn velocity_secret<'a>(
    ctx: &'a SessionContext<'_>,
    handler: &ForwardingHandler,
) -> Option<&'a [u8]> {
    handler
        .is_velocity()
        .then(|| ctx.services.forwarding_secret())
        .flatten()
}

pub(crate) async fn acknowledge_login(
    ctx: &SessionContext<'_>,
    backend: &mut BackendBridge,
) -> Result<(), CoreError> {
    if ctx.version().no_less_than(ProtocolVersion::V1_20_2) {
        backend
            .send_packet(&SLoginAcknowledged, ctx.registry())
            .await?;
        backend.set_state(ConnectionState::Config);
        tracing::debug!("backend LoginAcknowledged -> Config");
    }
    Ok(())
}

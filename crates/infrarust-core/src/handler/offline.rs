//! Offline proxy mode handler.
//!
//! No authentication — packets are parsed and forwarded transparently.
//! The proxy activates compression and manages state transitions
//! (Login → Config → Play) but does not verify player identity.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use infrarust_protocol::registry::PacketRegistry;
use infrarust_transport::BackendConnector;

use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::types::{HandshakeData, LoginData, RoutingData};
use crate::registry::{ConnectionRegistry, SessionEntry};
use crate::session::backend_bridge::BackendBridge;
use crate::session::client_bridge::ClientBridge;
use crate::session::proxy_loop::{ProxyLoopOutcome, proxy_loop};

/// Handles connections in Offline proxy mode.
///
/// Flow: create bridges → send initial packets → proxy_loop → cleanup.
pub struct OfflineHandler {
    backend_connector: Arc<BackendConnector>,
    registry: Arc<PacketRegistry>,
    connection_registry: Arc<ConnectionRegistry>,
}

impl OfflineHandler {
    /// Creates a new offline handler.
    pub fn new(
        backend_connector: Arc<BackendConnector>,
        registry: Arc<PacketRegistry>,
        connection_registry: Arc<ConnectionRegistry>,
    ) -> Self {
        Self {
            backend_connector,
            registry,
            connection_registry,
        }
    }

    /// Handles an offline-mode connection.
    pub async fn handle(
        &self,
        mut ctx: ConnectionContext,
        shutdown: CancellationToken,
    ) -> Result<(), CoreError> {
        let routing = ctx.require_extension::<RoutingData>("RoutingData")?.clone();
        let handshake = ctx
            .require_extension::<HandshakeData>("HandshakeData")?
            .clone();
        let login_data = ctx.extensions.get::<LoginData>().cloned();

        let server_config = &routing.server_config;

        // 1. Create client bridge
        let mut client = ClientBridge::new(
            ctx.take_stream(),
            ctx.buffered_data.split(),
            handshake.protocol_version,
        );

        // 2. Connect to backend
        let backend_conn = match self
            .backend_connector
            .connect(
                &routing.config_id,
                &server_config.addresses,
                server_config.timeouts.as_ref().map(|t| t.connect),
                server_config.send_proxy_protocol,
                &ctx.connection_info(),
            )
            .await
        {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(
                    server = %routing.config_id,
                    error = %e,
                    "backend unreachable, sending disconnect to client"
                );
                let msg = server_config.effective_disconnect_message();
                client.disconnect(msg, &self.registry).await.ok();
                return Ok(());
            }
        };

        // 3. Create backend bridge
        let mut backend = BackendBridge::new(backend_conn.into_stream());

        // 4. Send initial packets (handshake + login start with domain rewrite)
        backend
            .send_initial_packets(&handshake, server_config)
            .await?;

        // 5. Register session
        let session_token = CancellationToken::new();
        let session_id = Uuid::new_v4();
        self.connection_registry.register(SessionEntry {
            session_id,
            username: login_data.as_ref().map(|d| d.username.clone()),
            player_uuid: login_data.as_ref().and_then(|d| d.player_uuid),
            client_ip: ctx.client_ip,
            server_id: routing.config_id.clone(),
            proxy_mode: server_config.proxy_mode,
            connected_at: ctx.connected_at,
            shutdown_token: session_token.clone(),
        });

        tracing::info!(
            session = %session_id,
            server = %routing.config_id,
            username = ?login_data.as_ref().map(|d| &d.username),
            mode = "offline",
            "session started"
        );

        // 6. Combine shutdown tokens
        let combined_shutdown = CancellationToken::new();
        let combined = combined_shutdown.clone();
        let global = shutdown.clone();
        let session = session_token.clone();
        tokio::spawn(async move {
            tokio::select! {
                () = global.cancelled() => combined.cancel(),
                () = session.cancelled() => combined.cancel(),
            }
        });

        // 7. Proxy loop (Login → Config → Play)
        let outcome =
            proxy_loop(&mut client, &mut backend, &self.registry, combined_shutdown).await;

        // 8. Cleanup
        self.connection_registry.unregister(&session_id);

        match &outcome {
            ProxyLoopOutcome::ClientDisconnected => {
                tracing::info!(session = %session_id, "client disconnected");
            }
            ProxyLoopOutcome::BackendDisconnected { reason } => {
                tracing::info!(session = %session_id, ?reason, "backend disconnected");
            }
            ProxyLoopOutcome::Shutdown => {
                tracing::debug!(session = %session_id, "shutdown");
            }
            ProxyLoopOutcome::Error(e) => {
                tracing::warn!(session = %session_id, error = %e, "session error");
            }
        }

        Ok(())
    }
}

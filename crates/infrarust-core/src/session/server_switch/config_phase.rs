//! Configuration phase handling for server switch (1.20.2+).
//!
//! When switching servers on 1.20.2+, the proxy tells the client to re-enter
//! the configuration phase, forwards config packets from the new backend
//! opaquely, then transitions back to Play.

use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::config::{
    CConfigDisconnect, CFinishConfig, SAcknowledgeFinishConfig,
};
use infrarust_protocol::packets::play::disconnect::CDisconnect;
use infrarust_protocol::packets::play::start_configuration::{
    CStartConfiguration, SAcknowledgeConfiguration,
};
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};

use crate::error::CoreError;
use crate::plugin_messaging::channels::MessageIds;
use crate::plugin_messaging::router::{self, Scope};
use crate::session::backend_bridge::BackendBridge;
use crate::session::client_bridge::ClientBridge;
use crate::session::kick::BackendKick;
use crate::session::presentation::PresentationIds;

pub(super) enum PhaseError {
    Backend(CoreError),
    Client(CoreError),
}

fn backend_frame(
    read: Result<Option<PacketFrame>, CoreError>,
    kick_id: Option<i32>,
    state: ConnectionState,
    version: ProtocolVersion,
) -> Result<PacketFrame, PhaseError> {
    let frame = read
        .map_err(PhaseError::Backend)?
        .ok_or(PhaseError::Backend(CoreError::ConnectionClosed))?;
    if Some(frame.id) == kick_id {
        return Err(PhaseError::Backend(CoreError::BackendKick(Box::new(
            BackendKick::new(frame, state, version),
        ))));
    }
    Ok(frame)
}

async fn client_frame(client: &mut ClientBridge) -> Result<PacketFrame, PhaseError> {
    client
        .read_frame()
        .await
        .map_err(PhaseError::Client)?
        .ok_or(PhaseError::Client(CoreError::ConnectionClosed))
}

async fn from_client(
    scope: &Scope<'_>,
    messages: &MessageIds,
    presentation: &PresentationIds,
    frame: PacketFrame,
) -> Option<PacketFrame> {
    let state = ConnectionState::Config;
    if presentation.client_reply(scope.session, &scope.services.event_bus, &frame, state) {
        return None;
    }
    if messages.is_information(&frame, state) {
        router::observe_information(
            scope.session,
            &scope.services.event_bus,
            &frame,
            state,
            scope.version,
        );
        return Some(frame);
    }
    if messages.is_serverbound(&frame, state) {
        return router::from_client(scope, frame, state).await;
    }
    Some(frame)
}

/// Handles the configuration phase during a server switch for 1.20.2+.
///
/// Returns the JoinGame frame read from the backend after config phase completes.
pub(super) async fn handle_config_phase_switch(
    client: &mut ClientBridge,
    backend: &mut BackendBridge,
    scope: &Scope<'_>,
    stranded: &mut bool,
) -> Result<PacketFrame, PhaseError> {
    let registry = scope.services.packet_registry.as_ref();
    let version = scope.version;
    let messages = MessageIds::resolve(registry, version);
    let presentation = PresentationIds::resolve(registry, version);
    let config_kick = registry.get_packet_id::<CConfigDisconnect>(version);
    let finish_config_id = registry.get_packet_id::<CFinishConfig>(version);
    let ack_finish_id = registry.get_packet_id::<SAcknowledgeFinishConfig>(version);

    let mut pending = None;
    if client.state() == ConnectionState::Play {
        let first = backend_frame(
            backend.read_frame().await,
            config_kick,
            ConnectionState::Config,
            version,
        )?;
        *stranded = true;
        client
            .send_packet(&CStartConfiguration, registry)
            .await
            .map_err(PhaseError::Client)?;
        let ack_id = registry.get_packet_id::<SAcknowledgeConfiguration>(version);
        loop {
            let frame = client_frame(client).await?;
            if Some(frame.id) == ack_id {
                break;
            }
            presentation.client_reply(
                scope.session,
                &scope.services.event_bus,
                &frame,
                ConnectionState::Play,
            );
            tracing::trace!(
                id = frame.id,
                "absorbing client packet during config transition"
            );
        }
        pending = Some(first);
    }
    *stranded = true;

    client.set_state(ConnectionState::Config);
    backend.set_state(ConnectionState::Config);

    loop {
        let frame = match pending.take() {
            Some(frame) => frame,
            None => tokio::select! {
                read = backend.read_frame() => {
                    backend_frame(read, config_kick, ConnectionState::Config, version)?
                }
                read = client.read_frame() => {
                    let frame = read
                        .map_err(PhaseError::Client)?
                        .ok_or(PhaseError::Client(CoreError::ConnectionClosed))?;
                    if let Some(frame) = from_client(scope, &messages, &presentation, frame).await {
                        backend.write_frame(&frame).await.map_err(PhaseError::Backend)?;
                    }
                    continue;
                }
            },
        };
        presentation.observe_backend(scope.session, &frame, ConnectionState::Config);
        let frame = if presentation.is_transfer(&frame, ConnectionState::Config) {
            match presentation
                .transfer_from_backend(
                    scope.session,
                    &scope.services.event_bus,
                    frame,
                    ConnectionState::Config,
                )
                .await
            {
                Some(frame) => frame,
                None => continue,
            }
        } else {
            frame
        };
        let frame = if messages.is_clientbound(&frame, ConnectionState::Config) {
            let routed = router::from_backend(scope, frame, backend, ConnectionState::Config)
                .await
                .map_err(PhaseError::Backend)?;
            backend.flush().await.map_err(PhaseError::Backend)?;
            match routed {
                Some(frame) => frame,
                None => continue,
            }
        } else {
            frame
        };
        client
            .write_frame(&frame)
            .await
            .map_err(PhaseError::Client)?;
        if Some(frame.id) == finish_config_id {
            break;
        }
    }

    loop {
        let frame = client_frame(client).await?;
        if Some(frame.id) == ack_finish_id {
            backend
                .write_frame(&frame)
                .await
                .map_err(PhaseError::Backend)?;
            break;
        }
        presentation.client_reply(
            scope.session,
            &scope.services.event_bus,
            &frame,
            ConnectionState::Config,
        );
        tracing::trace!(
            id = frame.id,
            "absorbing client packet waiting for finish ack"
        );
    }

    client.set_state(ConnectionState::Play);
    backend.set_state(ConnectionState::Play);

    let play_kick = registry.get_packet_id::<CDisconnect>(version);
    backend_frame(
        backend.read_frame().await,
        play_kick,
        ConnectionState::Play,
        version,
    )
}

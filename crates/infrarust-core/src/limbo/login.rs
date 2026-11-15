use std::time::Duration;

use infrarust_protocol::packets::config::{CFinishConfig, SAcknowledgeFinishConfig, SKnownPacks};
use infrarust_protocol::packets::play::start_configuration::SAcknowledgeConfiguration;
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};

use crate::error::CoreError;
use crate::registry_data::embedded;
use crate::session::client_bridge::ClientBridge;
use crate::session::frame_chain::FrameChain;
use crate::util::packets::encode_packet;

const LIMBO_CONFIG_PHASE_TIMEOUT_SECS: u64 = 10;

pub(crate) async fn complete_config_for_limbo(
    client: &mut ClientBridge,
    version: ProtocolVersion,
    registry: &PacketRegistry,
    observer: Option<&FrameChain<'_>>,
) -> Result<(), CoreError> {
    if client.awaits_config_ack() {
        absorb_until(
            client,
            registry.get_packet_id::<SAcknowledgeConfiguration>(version),
            "limbo login: client did not acknowledge the configuration phase in time",
            None,
        )
        .await?;
        client.reconfiguration_acknowledged();
    }

    let embedded = embedded::frames(version);
    if let Ok(Some(kp_frame)) = embedded.as_ref().map(|frames| &frames.known_packs) {
        client.write_frame(kp_frame).await?;

        let skp_id = registry.get_packet_id::<SKnownPacks>(version);

        absorb_until(
            client,
            skp_id,
            "limbo login: client did not send KnownPacks in time",
            observer,
        )
        .await?;
    }

    for frame in &embedded?.registry {
        client.write_frame(frame).await?;
    }

    let finish_frame = encode_packet(&CFinishConfig, version, registry)?;
    client.write_frame(&finish_frame).await?;

    let ack_id = registry.get_packet_id::<SAcknowledgeFinishConfig>(version);

    absorb_until(
        client,
        ack_id,
        "limbo login: client did not acknowledge finish config in time",
        observer,
    )
    .await?;

    client.set_state(ConnectionState::Play);

    Ok(())
}

async fn absorb_until(
    client: &mut ClientBridge,
    target_id: Option<i32>,
    timeout_message: &'static str,
    observer: Option<&FrameChain<'_>>,
) -> Result<(), CoreError> {
    tokio::time::timeout(
        Duration::from_secs(LIMBO_CONFIG_PHASE_TIMEOUT_SECS),
        async {
            loop {
                let frame = client
                    .read_frame()
                    .await?
                    .ok_or(CoreError::ConnectionClosed)?;

                if Some(frame.id) == target_id {
                    break;
                }
                if let Some(observer) = observer {
                    observer.observe(&frame, ConnectionState::Config);
                }
                tracing::trace!(
                    id = frame.id,
                    "absorbing client config packet during limbo login"
                );
            }
            Ok::<(), CoreError>(())
        },
    )
    .await
    .map_err(|_| CoreError::Timeout(timeout_message.into()))?
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use bytes::BytesMut;
    use tokio::net::{TcpListener, TcpStream};

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn absorb_until_times_out_when_client_is_silent() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let _peer = TcpStream::connect(addr).await.unwrap(); // stays silent
        let (server_side, _) = listener.accept().await.unwrap();

        let mut client = ClientBridge::new(server_side, BytesMut::new(), ProtocolVersion::V1_21);

        let result = absorb_until(&mut client, Some(0x42), "test timeout", None).await;
        assert!(
            matches!(result, Err(CoreError::Timeout(_))),
            "expected a timeout, got {result:?}"
        );
    }
}

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use bytes::{Bytes, BytesMut};
use infrarust_core::auth::game_profile::GameProfile;
use infrarust_core::auth::mojang::{AuthLimits, MojangAuth};
use infrarust_core::error::CoreError;
use infrarust_core::test_support::ClientBridge;
use infrarust_protocol::Packet;
use infrarust_protocol::io::{PacketDecoder, PacketEncoder, PacketFrame};
use infrarust_protocol::packets::login::{
    CEncryptionRequest, EncryptionProof, SEncryptionResponse,
};
use infrarust_protocol::registry::{PacketRegistry, build_default_registry};
use infrarust_protocol::version::ProtocolVersion;
use rsa::pkcs8::DecodePublicKey;
use rsa::rand_core::OsRng;
use rsa::{Pkcs1v15Encrypt, RsaPublicKey};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

const V: ProtocolVersion = ProtocolVersion::V1_21;
const GUARD: Duration = Duration::from_secs(15);

async fn bridge_and_peer(buffered: BytesMut) -> (ClientBridge, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (peer, accepted) = tokio::join!(TcpStream::connect(addr), listener.accept());
    let (stream, _) = accepted.unwrap();
    (ClientBridge::new(stream, buffered, V), peer.unwrap())
}

async fn session_server() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    (
        listener,
        format!("http://{addr}/session/minecraft/hasJoined"),
    )
}

async fn answer_encryption(peer: &mut TcpStream, registry: &PacketRegistry, trailing: &[u8]) {
    let mut decoder = PacketDecoder::new();
    let frame = loop {
        if let Some(frame) = decoder.try_next_frame().unwrap() {
            break frame;
        }
        let n = peer.read_buf(decoder.read_buf_mut()).await.unwrap();
        assert_ne!(n, 0, "the proxy closed before asking for encryption");
    };
    let request = CEncryptionRequest::decode(&mut frame.payload.as_ref(), V).unwrap();

    let public_key = RsaPublicKey::from_public_key_der(&request.public_key).unwrap();
    let encrypt = |data: &[u8]| {
        public_key
            .encrypt(&mut OsRng, Pkcs1v15Encrypt, data)
            .unwrap()
    };
    let response = SEncryptionResponse {
        shared_secret: encrypt(&[0x42; 16]),
        proof: EncryptionProof::VerifyToken(encrypt(&request.verify_token)),
    };

    let mut payload = Vec::new();
    response.encode(&mut payload, V).unwrap();
    let mut encoder = PacketEncoder::new();
    encoder
        .append_raw(
            registry.get_packet_id::<SEncryptionResponse>(V).unwrap(),
            &payload,
        )
        .unwrap();
    let mut wire = encoder.take();
    wire.extend_from_slice(trailing);
    peer.write_all(&wire).await.unwrap();
}

async fn authenticate_against(
    auth: &MojangAuth,
    registry: &PacketRegistry,
    trailing: &[u8],
) -> Result<GameProfile, CoreError> {
    let (mut bridge, mut peer) = bridge_and_peer(BytesMut::new()).await;
    let exchange = async {
        tokio::join!(
            auth.authenticate(&mut bridge, "Steve", None, registry),
            answer_encryption(&mut peer, registry, trailing),
        )
    };
    let (result, ()) = timeout(GUARD, exchange)
        .await
        .expect("the authentication outlived every limit");
    result
}

#[tokio::test]
async fn a_silent_client_expires() {
    let (_server, url) = session_server().await;
    let auth = MojangAuth::with_limits(
        url,
        AuthLimits {
            authenticate_timeout: Duration::from_millis(100),
            ..AuthLimits::default()
        },
    )
    .unwrap();
    let registry = build_default_registry();
    let (mut bridge, _peer) = bridge_and_peer(BytesMut::new()).await;

    let result = timeout(
        GUARD,
        auth.authenticate(&mut bridge, "Steve", None, &registry),
    )
    .await
    .expect("the authentication outlived its deadline");

    assert!(matches!(result, Err(CoreError::Timeout(_))), "{result:?}");
}

#[tokio::test]
async fn a_silent_session_server_fails_the_request() {
    let (_server, url) = session_server().await;
    let auth = MojangAuth::with_limits(
        url,
        AuthLimits {
            request_timeout: Duration::from_millis(200),
            ..AuthLimits::default()
        },
    )
    .unwrap();
    let registry = build_default_registry();

    let result = authenticate_against(&auth, &registry, &[]).await;

    let Err(CoreError::Auth(message)) = &result else {
        panic!("expected an auth error, got {result:?}");
    };
    assert!(
        message.contains("session server request failed"),
        "{message}"
    );
}

#[tokio::test]
async fn the_deadline_covers_the_session_server() {
    let (_server, url) = session_server().await;
    let auth = MojangAuth::with_limits(
        url,
        AuthLimits {
            authenticate_timeout: Duration::from_secs(1),
            request_timeout: Duration::from_secs(60),
            ..AuthLimits::default()
        },
    )
    .unwrap();
    let registry = build_default_registry();

    let result = authenticate_against(&auth, &registry, &[]).await;

    assert!(matches!(result, Err(CoreError::Timeout(_))), "{result:?}");
}

#[tokio::test]
async fn no_request_leaves_without_a_session_permit() {
    let (server, url) = session_server().await;
    let auth = MojangAuth::with_limits(
        url,
        AuthLimits {
            authenticate_timeout: Duration::from_secs(1),
            session_permits: 0,
            ..AuthLimits::default()
        },
    )
    .unwrap();
    let registry = build_default_registry();

    let result = authenticate_against(&auth, &registry, &[]).await;

    assert!(matches!(result, Err(CoreError::Timeout(_))), "{result:?}");
    assert!(
        timeout(Duration::from_millis(100), server.accept())
            .await
            .is_err(),
        "a request reached the session server without a permit"
    );
}

fn answer_every_request_with_a_profile(listener: TcpListener) {
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut head = Vec::new();
            while !head.windows(4).any(|window| window == b"\r\n\r\n") {
                if stream.read_buf(&mut head).await.unwrap() == 0 {
                    break;
                }
            }
            let body =
                r#"{"id":"069a79f444e94726a5befca90e38aaf5","name":"Steve","properties":[]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        }
    });
}

#[tokio::test]
async fn a_clean_exchange_authenticates() {
    let (server, url) = session_server().await;
    answer_every_request_with_a_profile(server);
    let auth = MojangAuth::with_session_url(url).unwrap();
    let registry = build_default_registry();

    let profile = authenticate_against(&auth, &registry, &[]).await.unwrap();

    assert_eq!(profile.name, "Steve");
}

#[tokio::test]
async fn bytes_sent_ahead_of_the_encryption_switch_fail_the_login() {
    let (server, url) = session_server().await;
    answer_every_request_with_a_profile(server);
    let auth = MojangAuth::with_session_url(url).unwrap();
    let registry = build_default_registry();

    let result = authenticate_against(&auth, &registry, &[0xAB; 8]).await;

    let Err(CoreError::Auth(message)) = &result else {
        panic!("expected an auth error, got {result:?}");
    };
    assert!(message.contains("ahead of the encryption"), "{message}");
}

#[tokio::test]
async fn enable_encryption_accepts_an_empty_decoder() {
    let (mut bridge, _peer) = bridge_and_peer(BytesMut::new()).await;

    bridge.enable_encryption(&[0x42; 16]).unwrap();
}

#[tokio::test]
async fn a_refused_switch_leaves_the_bridge_in_cleartext() {
    let (mut bridge, mut peer) = bridge_and_peer(BytesMut::from(&[0xAB; 8][..])).await;

    let result = bridge.enable_encryption(&[0x42; 16]);
    assert!(matches!(result, Err(CoreError::Auth(_))), "{result:?}");

    bridge
        .write_frame(&PacketFrame::new(0x00, Bytes::from_static(b"x")))
        .await
        .unwrap();

    let mut encoder = PacketEncoder::new();
    encoder.append_raw(0x00, b"x").unwrap();
    let expected = encoder.take();
    let mut received = vec![0u8; expected.len()];
    peer.read_exact(&mut received).await.unwrap();
    assert_eq!(received, expected.as_ref());
}

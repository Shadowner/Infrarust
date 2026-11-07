#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use bytes::{Bytes, BytesMut};
use infrarust_core::error::CoreError;
use infrarust_core::test_support::ClientBridge;
use infrarust_protocol::crypto::DecryptCipher;
use infrarust_protocol::io::{PacketDecoder, PacketFrame};
use infrarust_protocol::version::ProtocolVersion;
use tokio::io::AsyncReadExt;
use tokio::net::{TcpSocket, TcpStream};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

const V: ProtocolVersion = ProtocolVersion::V1_21;
const FRAME_SIZE: usize = 256 * 1024;
const FRAMES: usize = 8;
const SOCKET_BUFFER: u32 = 4096;
const KEY: [u8; 16] = [7; 16];

async fn bridge_and_peer() -> (ClientBridge, TcpStream) {
    let server = TcpSocket::new_v4().unwrap();
    server.set_send_buffer_size(SOCKET_BUFFER).unwrap();
    server.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let addr = server.local_addr().unwrap();
    let listener = server.listen(1).unwrap();

    let client = TcpSocket::new_v4().unwrap();
    client.set_recv_buffer_size(SOCKET_BUFFER).unwrap();
    let (peer, accepted) = tokio::join!(client.connect(addr), listener.accept());
    let (stream, _) = accepted.unwrap();
    (ClientBridge::new(stream, BytesMut::new(), V), peer.unwrap())
}

fn frame(index: usize) -> PacketFrame {
    PacketFrame::new(0x01, Bytes::from(vec![index as u8; FRAME_SIZE]))
}

fn queue_more_than_the_sockets_hold(client: &mut ClientBridge) {
    for index in 0..FRAMES {
        client.queue_frame(&frame(index)).unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn a_stalled_write_gives_up_at_the_limit_and_stays_broken() {
    let (mut client, _peer) = bridge_and_peer().await;
    queue_more_than_the_sockets_hold(&mut client);

    let started = Instant::now();
    let result = client.flush().await;
    let waited = started.elapsed();

    assert!(matches!(result, Err(CoreError::Timeout(_))), "{result:?}");
    assert_eq!(waited, Duration::from_secs(30));

    let again = client.flush().await;
    assert!(
        matches!(again, Err(CoreError::ConnectionClosed)),
        "{again:?}"
    );
    let next = client.write_frame(&frame(0)).await;
    assert!(matches!(next, Err(CoreError::ConnectionClosed)), "{next:?}");
    assert_eq!(started.elapsed(), waited, "a broken write must not wait");
}

#[tokio::test(start_paused = true)]
async fn cancelling_a_stalled_write_leaves_it_the_grace_period() {
    let (mut client, _peer) = bridge_and_peer().await;
    let token = CancellationToken::new();
    client.set_cancel(token.clone());
    queue_more_than_the_sockets_hold(&mut client);

    let started = Instant::now();
    let (result, ()) = tokio::join!(client.flush(), async {
        tokio::time::sleep(Duration::from_secs(1)).await;
        token.cancel();
    });

    assert!(matches!(result, Err(CoreError::Timeout(_))), "{result:?}");
    assert_eq!(started.elapsed(), Duration::from_secs(6));
}

#[tokio::test(start_paused = true)]
async fn a_dropped_write_leaves_the_bridge_broken() {
    let (mut client, _peer) = bridge_and_peer().await;
    queue_more_than_the_sockets_hold(&mut client);

    tokio::time::timeout(Duration::from_secs(1), client.flush())
        .await
        .expect_err("the peer reads nothing, so the write cannot finish");

    let next = client.write_frame(&frame(0)).await;
    assert!(matches!(next, Err(CoreError::ConnectionClosed)), "{next:?}");
    let empty = client.flush().await;
    assert!(
        matches!(empty, Err(CoreError::ConnectionClosed)),
        "{empty:?}"
    );
}

#[tokio::test]
async fn a_slow_reader_gets_every_byte_and_the_bridge_stays_usable() {
    let (mut client, mut peer) = bridge_and_peer().await;
    client.enable_encryption(&KEY).unwrap();
    queue_more_than_the_sockets_hold(&mut client);

    let reader = async move {
        let mut cipher = DecryptCipher::new(&KEY);
        let mut decoder = PacketDecoder::new();
        for index in 0..=FRAMES {
            let received = loop {
                if let Some(received) = decoder.try_next_frame().unwrap() {
                    break received;
                }
                let mut chunk = BytesMut::with_capacity(4096);
                assert_ne!(peer.read_buf(&mut chunk).await.unwrap(), 0, "peer closed");
                cipher.decrypt(&mut chunk);
                decoder.queue_bytes(&chunk);
            };
            assert_eq!(received.payload, frame(index).payload, "frame {index}");
        }
    };
    let writer = async {
        client.flush().await.unwrap();
        client.write_frame(&frame(FRAMES)).await.unwrap();
    };

    tokio::time::timeout(Duration::from_secs(30), async {
        tokio::join!(reader, writer);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn a_cancelled_token_does_not_stop_a_write_that_has_room() {
    let (mut client, mut peer) = bridge_and_peer().await;
    let token = CancellationToken::new();
    token.cancel();
    client.set_cancel(token);

    let small = PacketFrame::new(0x01, Bytes::from_static(b"goodbye"));
    client.write_frame(&small).await.unwrap();

    let mut received = BytesMut::new();
    assert_ne!(peer.read_buf(&mut received).await.unwrap(), 0);
}

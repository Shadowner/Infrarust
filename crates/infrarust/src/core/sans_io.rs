use std::net::Ipv4Addr;
use std::borrow::Cow;

use bytes::BytesMut;
use valence_protocol::{packets::handshaking::HandshakeC2s, Packet, PacketDecoder};

#[derive(Debug, Clone)]
pub enum Decision {
    ConnectToBackend(Ipv4Addr, u16, BytesMut), 
    PassThrough(BytesMut), 
}

pub fn decision(data: &BytesMut) -> Decision {
    let mut decoder = PacketDecoder::new();
    decoder.queue_bytes(data.clone());

    let decode_result = decoder.try_next_packet();
    if decode_result.is_err() {
        println!("Error decoding packet: {:?}", decode_result.err());
        return Decision::PassThrough(data.clone());
    }

    let frame_option = decode_result.ok();
    if frame_option.is_none() {
        println!("No packet found in the data");
        return Decision::PassThrough(data.clone());
    }

    let maybe_packet = frame_option.unwrap();
    if maybe_packet.is_none() {
        println!("Empty packet received");
        return Decision::PassThrough(data.clone());
    }

    let packet_frame = maybe_packet.unwrap();
    if packet_frame.id != 0x00 {
        println!("Packet ID is not HandshakeC2s: {}", packet_frame.id);
        return Decision::PassThrough(data.clone());
    }
    let handshake_result = packet_frame.decode::<HandshakeC2s>();
    match handshake_result {
        Ok(handshake_ref) => {
            Decision::ConnectToBackend(Ipv4Addr::new(192,168,1,235), 25571, data.clone())
        },
        Err(e) => {
            println!("Error decoding HandshakeC2s packet: {:?}", e);
            Decision::PassThrough(data.clone())
        }
    }
}
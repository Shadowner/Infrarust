use bytes::BytesMut;

use crate::network::packet::Packet;

#[derive(Debug, Clone)]
pub enum GatewayMessage {
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum MinecraftCommunication<T> {
    RawData(BytesMut),
    Packet(Packet),
    Shutdown,
    CustomData(T),
}

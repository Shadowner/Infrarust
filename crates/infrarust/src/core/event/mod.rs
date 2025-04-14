use crate::network::packet::Packet;

#[derive(Debug, Clone)]
pub enum GatewayMessage {
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum MinecraftCommunication<T> {
    RawData(Vec<u8>),
    Packet(Packet),
    Shutdown,
    CustomData(T),
}

use bytes::Bytes;

#[derive(Debug, Clone)]
pub struct OpaquePacket {
    pub id: i32,
    pub payload: Bytes,
}

use std::net::SocketAddr;

use infrarust_api::types::{GameProfile, ProfileProperty};
use infrarust_config::ProxyMode;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::HandoverError;
use super::resume::ResumeRecord;
use crate::binary::HANDOVER_ACCEPTS;

pub const MAGIC: [u8; 8] = *b"IRHANDOV";
const HEADER_LEN: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HandleId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoverState {
    pub protocol: u32,
    pub from_version: String,
    pub created_at_ms: u64,
    pub next_player_id: u64,
    pub listener: Option<ListenerRecord>,
    pub sessions: Vec<SessionRecord>,
    #[serde(default)]
    pub resumes: Vec<ResumeRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListenerRecord {
    pub handle: HandleId,
    pub local_addr: SocketAddr,
    pub bind: SocketAddr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PropertyRecord {
    pub name: String,
    pub value: String,
    pub signature: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileRecord {
    pub uuid: Uuid,
    pub username: String,
    pub properties: Vec<PropertyRecord>,
}

impl From<&GameProfile> for ProfileRecord {
    fn from(profile: &GameProfile) -> Self {
        Self {
            uuid: profile.uuid,
            username: profile.username.clone(),
            properties: profile
                .properties
                .iter()
                .map(|property| PropertyRecord {
                    name: property.name.clone(),
                    value: property.value.clone(),
                    signature: property.signature.clone(),
                })
                .collect(),
        }
    }
}

impl From<ProfileRecord> for GameProfile {
    fn from(record: ProfileRecord) -> Self {
        Self {
            uuid: record.uuid,
            username: record.username,
            properties: record
                .properties
                .into_iter()
                .map(|property| ProfileProperty {
                    name: property.name,
                    value: property.value,
                    signature: property.signature,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMeta {
    pub player_id: u64,
    pub profile: ProfileRecord,
    pub protocol_version: i32,
    pub mode: ProxyMode,
    pub client_addr: SocketAddr,
    pub server: String,
    pub connected_address: Option<String>,
    pub virtual_host: Option<String>,
    pub connected_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SocketRecord {
    pub handle: HandleId,
    pub local: SocketAddr,
    pub peer: SocketAddr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TailRef {
    pub offset: u64,
    pub len: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectionRecord {
    pub tail: TailRef,
    pub source_eof: bool,
    pub copied: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SideRecord {
    Client,
    Backend,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub meta: SessionMeta,
    pub client: SocketRecord,
    pub backend: SocketRecord,
    pub client_to_backend: DirectionRecord,
    pub backend_to_client: DirectionRecord,
    pub first_closed: Option<SideRecord>,
}

#[derive(Debug, Default)]
pub struct TailBlob(Vec<u8>);

impl TailBlob {
    pub fn push(&mut self, bytes: &[u8]) -> Result<TailRef, HandoverError> {
        let len = u32::try_from(bytes.len())
            .map_err(|_| HandoverError::Malformed("a buffered tail exceeds 4 GiB".into()))?;
        let offset = self.0.len() as u64;
        self.0.extend_from_slice(bytes);
        Ok(TailRef { offset, len })
    }

    pub fn into_inner(self) -> Vec<u8> {
        self.0
    }
}

pub fn tail(tails: &[u8], at: TailRef) -> Result<&[u8], HandoverError> {
    let start = usize::try_from(at.offset)
        .map_err(|_| HandoverError::Malformed("tail offset out of range".into()))?;
    let end = start
        .checked_add(at.len as usize)
        .filter(|end| *end <= tails.len())
        .ok_or_else(|| HandoverError::Malformed("tail out of range".into()))?;
    Ok(&tails[start..end])
}

pub fn encode(state: &HandoverState, tails: &[u8]) -> Result<Vec<u8>, HandoverError> {
    let meta = serde_json::to_vec(state).map_err(|e| HandoverError::Malformed(e.to_string()))?;
    let meta_len = u32::try_from(meta.len())
        .map_err(|_| HandoverError::Malformed("handover metadata exceeds 4 GiB".into()))?;
    let mut out = Vec::with_capacity(HEADER_LEN + meta.len() + tails.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&state.protocol.to_le_bytes());
    out.extend_from_slice(&meta_len.to_le_bytes());
    out.extend_from_slice(&meta);
    out.extend_from_slice(tails);
    Ok(out)
}

pub fn decode(bytes: &[u8]) -> Result<(HandoverState, Vec<u8>), HandoverError> {
    let header = bytes
        .get(..HEADER_LEN)
        .ok_or_else(|| HandoverError::Malformed("truncated header".into()))?;
    if header[..8] != MAGIC {
        return Err(HandoverError::Malformed("bad magic".into()));
    }
    let protocol = u32::from_le_bytes([header[8], header[9], header[10], header[11]]);
    if !HANDOVER_ACCEPTS.contains(&protocol) {
        return Err(HandoverError::Protocol {
            found: protocol,
            accepted: HANDOVER_ACCEPTS,
        });
    }
    let meta_len = u32::from_le_bytes([header[12], header[13], header[14], header[15]]) as usize;
    let rest = &bytes[HEADER_LEN..];
    let meta = rest
        .get(..meta_len)
        .ok_or_else(|| HandoverError::Malformed("truncated metadata".into()))?;
    let tails = rest[meta_len..].to_vec();
    let state: HandoverState =
        serde_json::from_slice(meta).map_err(|e| HandoverError::Malformed(e.to_string()))?;
    if state.protocol != protocol {
        return Err(HandoverError::Malformed(format!(
            "header protocol {protocol} differs from metadata protocol {}",
            state.protocol
        )));
    }
    validate(&state, &tails)?;
    Ok((state, tails))
}

fn validate(state: &HandoverState, tails: &[u8]) -> Result<(), HandoverError> {
    let mut handles: Vec<HandleId> = state
        .listener
        .iter()
        .map(|listener| listener.handle)
        .chain(
            state
                .sessions
                .iter()
                .flat_map(|session| [session.client.handle, session.backend.handle]),
        )
        .collect();
    let total = handles.len();
    handles.sort_unstable();
    handles.dedup();
    if handles.len() != total {
        return Err(HandoverError::Malformed(
            "a handle is referenced twice".into(),
        ));
    }
    for session in &state.sessions {
        tail(tails, session.client_to_backend.tail)?;
        tail(tails, session.backend_to_client.tail)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::binary::HANDOVER_PROTOCOL;

    fn session(client: u32, backend: u32, c2b: TailRef, b2c: TailRef) -> SessionRecord {
        let addr = |port| SocketAddr::from(([127, 0, 0, 1], port));
        SessionRecord {
            meta: SessionMeta {
                player_id: u64::from(client),
                profile: ProfileRecord {
                    uuid: Uuid::nil(),
                    username: "Steve".into(),
                    properties: vec![PropertyRecord {
                        name: "textures".into(),
                        value: "e30=".into(),
                        signature: None,
                    }],
                },
                protocol_version: 769,
                mode: ProxyMode::ZeroCopy,
                client_addr: addr(40_000),
                server: "survival".into(),
                connected_address: Some("10.0.0.2:25565".into()),
                virtual_host: Some("play.example.com".into()),
                connected_at_ms: 1_700_000_000_000,
            },
            client: SocketRecord {
                handle: HandleId(client),
                local: addr(25_565),
                peer: addr(40_000),
            },
            backend: SocketRecord {
                handle: HandleId(backend),
                local: addr(50_000),
                peer: addr(25_566),
            },
            client_to_backend: DirectionRecord {
                tail: c2b,
                source_eof: false,
                copied: 10,
            },
            backend_to_client: DirectionRecord {
                tail: b2c,
                source_eof: true,
                copied: 20,
            },
            first_closed: Some(SideRecord::Backend),
        }
    }

    fn state(sessions: Vec<SessionRecord>) -> HandoverState {
        HandoverState {
            protocol: HANDOVER_PROTOCOL,
            from_version: "2.0.0-beta.6".into(),
            created_at_ms: 1,
            next_player_id: 42,
            listener: Some(ListenerRecord {
                handle: HandleId(0),
                local_addr: SocketAddr::from(([0, 0, 0, 0], 25_565)),
                bind: SocketAddr::from(([0, 0, 0, 0], 25_565)),
            }),
            sessions,
            resumes: Vec::new(),
        }
    }

    #[test]
    fn state_and_tails_round_trip() {
        let mut blob = TailBlob::default();
        let empty = blob.push(&[]).unwrap();
        let one = blob.push(&[7]).unwrap();
        let big = blob.push(&vec![9; 64 * 1024]).unwrap();
        let tails = blob.into_inner();
        let original = state(vec![session(1, 2, empty, one), session(3, 4, big, empty)]);

        let bytes = encode(&original, &tails).unwrap();
        let (decoded, decoded_tails) = decode(&bytes).unwrap();

        assert_eq!(decoded, original);
        assert_eq!(decoded_tails, tails);
        assert_eq!(tail(&decoded_tails, one).unwrap(), &[7]);
        assert!(tail(&decoded_tails, empty).unwrap().is_empty());
        assert_eq!(tail(&decoded_tails, big).unwrap().len(), 64 * 1024);
    }

    #[test]
    fn bad_magic_is_rejected() {
        let mut bytes = encode(&state(Vec::new()), &[]).unwrap();
        bytes[0] ^= 0xff;
        assert!(matches!(decode(&bytes), Err(HandoverError::Malformed(_))));
    }

    #[test]
    fn unknown_protocol_is_rejected() {
        let mut original = state(Vec::new());
        original.protocol = 999;
        let bytes = encode(&original, &[]).unwrap();
        assert!(matches!(
            decode(&bytes),
            Err(HandoverError::Protocol { found: 999, .. })
        ));
    }

    #[test]
    fn truncated_input_is_rejected() {
        let bytes = encode(&state(Vec::new()), &[]).unwrap();
        assert!(decode(&bytes[..10]).is_err());
        assert!(decode(&bytes[..bytes.len() - 1]).is_err());
    }

    #[test]
    fn a_tail_outside_the_blob_is_rejected() {
        let outside = TailRef { offset: 4, len: 8 };
        let bytes = encode(
            &state(vec![session(1, 2, outside, TailRef::default())]),
            &[0; 6],
        )
        .unwrap();
        assert!(matches!(decode(&bytes), Err(HandoverError::Malformed(_))));
    }

    #[test]
    fn a_handle_referenced_twice_is_rejected() {
        let bytes = encode(
            &state(vec![session(1, 0, TailRef::default(), TailRef::default())]),
            &[],
        )
        .unwrap();
        assert!(matches!(decode(&bytes), Err(HandoverError::Malformed(_))));
    }

    #[test]
    fn profiles_convert_both_ways() {
        let profile = GameProfile {
            uuid: Uuid::from_u128(5),
            username: "Alex".into(),
            properties: vec![ProfileProperty {
                name: "textures".into(),
                value: "v".into(),
                signature: Some("s".into()),
            }],
        };
        let back: GameProfile = ProfileRecord::from(&profile).into();
        assert_eq!(back, profile);
    }
}

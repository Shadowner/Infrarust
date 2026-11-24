use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::{SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use infrarust_transport::{FrozenDirection, FrozenRelay, RelaySide};

use super::HandoverError;
use super::state::{
    DirectionRecord, HandleId, HandoverState, ListenerRecord, SessionMeta, SessionRecord,
    SideRecord, SocketRecord, TailBlob, tail,
};
use crate::binary::HANDOVER_PROTOCOL;

#[derive(Debug)]
pub enum Handle {
    Listener(TcpListener),
    Stream(TcpStream),
}

#[derive(Debug, Default)]
pub struct HandleTable {
    handles: Vec<Option<Handle>>,
}

impl HandleTable {
    pub fn from_handles(handles: Vec<Handle>) -> Self {
        Self {
            handles: handles.into_iter().map(Some).collect(),
        }
    }

    pub fn push(&mut self, handle: Handle) -> Result<HandleId, HandoverError> {
        let id = u32::try_from(self.handles.len())
            .map_err(|_| HandoverError::Malformed("too many handles".into()))?;
        self.handles.push(Some(handle));
        Ok(HandleId(id))
    }

    pub fn len(&self) -> usize {
        self.handles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.handles.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (HandleId, &Handle)> {
        self.handles
            .iter()
            .enumerate()
            .filter_map(|(index, handle)| {
                let id = HandleId(u32::try_from(index).ok()?);
                handle.as_ref().map(|handle| (id, handle))
            })
    }

    pub fn get(&self, id: HandleId) -> Option<&Handle> {
        self.handles.get(id.0 as usize).and_then(Option::as_ref)
    }

    pub fn take(&mut self, id: HandleId) -> Result<Handle, HandoverError> {
        self.handles
            .get_mut(id.0 as usize)
            .and_then(Option::take)
            .ok_or(HandoverError::MissingHandle(id.0))
    }

    pub fn take_listener(&mut self, id: HandleId) -> Result<TcpListener, HandoverError> {
        match self.take(id)? {
            Handle::Listener(listener) => Ok(listener),
            Handle::Stream(_) => Err(HandoverError::WrongHandle(id.0, "listener")),
        }
    }

    pub fn take_stream(&mut self, id: HandleId) -> Result<TcpStream, HandoverError> {
        match self.take(id)? {
            Handle::Stream(stream) => Ok(stream),
            Handle::Listener(_) => Err(HandoverError::WrongHandle(id.0, "stream")),
        }
    }

    pub fn into_remaining(self) -> Vec<Handle> {
        self.handles.into_iter().flatten().collect()
    }
}

#[derive(Debug)]
pub struct ListenerHandover {
    pub socket: TcpListener,
    pub bind: SocketAddr,
}

#[derive(Debug)]
pub struct FrozenSession {
    pub meta: SessionMeta,
    pub relay: FrozenRelay,
}

#[derive(Debug)]
pub struct DroppedSession {
    pub meta: SessionMeta,
    pub error: HandoverError,
}

#[derive(Debug)]
pub struct HandoverPackage {
    pub state: HandoverState,
    pub tails: Vec<u8>,
    pub handles: HandleTable,
}

#[derive(Debug)]
pub struct Unpacked {
    pub from_version: String,
    pub created_at_ms: u64,
    pub next_player_id: u64,
    pub listener: Option<ListenerHandover>,
    pub sessions: Vec<FrozenSession>,
    pub dropped: Vec<DroppedSession>,
    pub held: Vec<Handle>,
}

#[derive(Debug, thiserror::Error)]
#[error("{error}")]
pub struct UnpackFailure {
    pub error: HandoverError,
    pub package: Box<HandoverPackage>,
}

impl HandoverPackage {
    pub fn build(
        from_version: &str,
        next_player_id: u64,
        listener: Option<ListenerHandover>,
        frozen: Vec<FrozenSession>,
    ) -> Result<(Self, Vec<DroppedSession>), HandoverError> {
        let mut handles = HandleTable::default();
        let mut blob = TailBlob::default();
        let listener = match listener {
            Some(ListenerHandover { socket, bind }) => {
                let local_addr = socket.local_addr()?;
                let handle = handles.push(Handle::Listener(socket))?;
                Some(ListenerRecord {
                    handle,
                    local_addr,
                    bind,
                })
            }
            None => None,
        };

        let mut sessions = Vec::with_capacity(frozen.len());
        let mut dropped = Vec::new();
        for FrozenSession { meta, relay } in frozen {
            let addresses = socket_addresses(&relay.client)
                .and_then(|client| Ok((client, socket_addresses(&relay.backend)?)));
            let ((client_local, client_peer), (backend_local, backend_peer)) = match addresses {
                Ok(addresses) => addresses,
                Err(error) => {
                    dropped.push(DroppedSession {
                        meta,
                        error: error.into(),
                    });
                    continue;
                }
            };
            let client_to_backend = direction(&mut blob, &relay.client_to_backend)?;
            let backend_to_client = direction(&mut blob, &relay.backend_to_client)?;
            let client = handles.push(Handle::Stream(relay.client))?;
            let backend = handles.push(Handle::Stream(relay.backend))?;
            sessions.push(SessionRecord {
                meta,
                client: SocketRecord {
                    handle: client,
                    local: client_local,
                    peer: client_peer,
                },
                backend: SocketRecord {
                    handle: backend,
                    local: backend_local,
                    peer: backend_peer,
                },
                client_to_backend,
                backend_to_client,
                first_closed: relay.first_closed.map(side_record),
            });
        }

        let state = HandoverState {
            protocol: HANDOVER_PROTOCOL,
            from_version: from_version.to_string(),
            created_at_ms: now_ms(),
            next_player_id,
            listener,
            sessions,
        };
        Ok((
            Self {
                state,
                tails: blob.into_inner(),
                handles,
            },
            dropped,
        ))
    }

    pub fn unpack(self) -> Result<Unpacked, UnpackFailure> {
        if let Err(error) = self.check_listener() {
            return Err(UnpackFailure {
                error,
                package: Box::new(self),
            });
        }
        let Self {
            state,
            tails,
            mut handles,
        } = self;
        let HandoverState {
            from_version,
            created_at_ms,
            next_player_id,
            listener,
            sessions: records,
            ..
        } = state;

        let mut held = Vec::new();
        let listener = listener.and_then(|record| match handles.take(record.handle) {
            Ok(Handle::Listener(socket)) => Some(ListenerHandover {
                socket,
                bind: record.bind,
            }),
            Ok(other) => {
                held.push(other);
                None
            }
            Err(_) => None,
        });

        let mut sessions = Vec::with_capacity(records.len());
        let mut dropped = Vec::new();
        for record in records {
            match restore_relay(&mut handles, &tails, &record) {
                Ok(relay) => sessions.push(FrozenSession {
                    meta: record.meta,
                    relay,
                }),
                Err((error, kept)) => {
                    held.extend(kept);
                    dropped.push(DroppedSession {
                        meta: record.meta,
                        error,
                    });
                }
            }
        }
        held.extend(handles.into_remaining());

        Ok(Unpacked {
            from_version,
            created_at_ms,
            next_player_id,
            listener,
            sessions,
            dropped,
            held,
        })
    }

    fn check_listener(&self) -> Result<(), HandoverError> {
        let Some(record) = &self.state.listener else {
            return Ok(());
        };
        match self.handles.get(record.handle) {
            None => Err(HandoverError::MissingHandle(record.handle.0)),
            Some(Handle::Stream(_)) => Err(HandoverError::WrongHandle(record.handle.0, "listener")),
            Some(Handle::Listener(listener)) => {
                let actual = listener.local_addr()?;
                if actual == record.local_addr {
                    Ok(())
                } else {
                    Err(HandoverError::Mismatch(
                        record.handle.0,
                        format!("listener bound to {actual}, expected {}", record.local_addr),
                    ))
                }
            }
        }
    }
}

type Refused = (HandoverError, Vec<Handle>);

fn restore_relay(
    handles: &mut HandleTable,
    tails: &[u8],
    record: &SessionRecord,
) -> Result<FrozenRelay, Refused> {
    let (client, backend) = take_streams(handles, record)?;
    let checked = check_socket(&client, record.client)
        .and_then(|()| check_socket(&backend, record.backend))
        .and_then(|()| {
            Ok((
                frozen_direction(tails, record.client_to_backend)?,
                frozen_direction(tails, record.backend_to_client)?,
            ))
        });
    match checked {
        Ok((client_to_backend, backend_to_client)) => Ok(FrozenRelay {
            client,
            backend,
            client_to_backend,
            backend_to_client,
            first_closed: record.first_closed.map(relay_side),
        }),
        Err(error) => Err((error, vec![Handle::Stream(client), Handle::Stream(backend)])),
    }
}

fn take_streams(
    handles: &mut HandleTable,
    record: &SessionRecord,
) -> Result<(TcpStream, TcpStream), Refused> {
    let client = handles.take(record.client.handle);
    let backend = handles.take(record.backend.handle);
    match (client, backend) {
        (Ok(Handle::Stream(client)), Ok(Handle::Stream(backend))) => Ok((client, backend)),
        (client, backend) => {
            let mut kept = Vec::new();
            let mut error = None;
            for (taken, id) in [
                (client, record.client.handle),
                (backend, record.backend.handle),
            ] {
                match taken {
                    Ok(handle @ Handle::Stream(_)) => kept.push(handle),
                    Ok(handle @ Handle::Listener(_)) => {
                        error.get_or_insert(HandoverError::WrongHandle(id.0, "stream"));
                        kept.push(handle);
                    }
                    Err(missing) => {
                        error.get_or_insert(missing);
                    }
                }
            }
            let error = error.unwrap_or_else(|| {
                HandoverError::Malformed("a session lost its sockets".to_string())
            });
            Err((error, kept))
        }
    }
}

fn check_socket(stream: &TcpStream, record: SocketRecord) -> Result<(), HandoverError> {
    let (local, peer) = socket_addresses(stream)?;
    if local != record.local || peer != record.peer {
        return Err(HandoverError::Mismatch(
            record.handle.0,
            format!(
                "socket is {local} -> {peer}, expected {} -> {}",
                record.local, record.peer
            ),
        ));
    }
    Ok(())
}

fn socket_addresses(stream: &TcpStream) -> std::io::Result<(SocketAddr, SocketAddr)> {
    Ok((stream.local_addr()?, stream.peer_addr()?))
}

fn direction(
    blob: &mut TailBlob,
    frozen: &FrozenDirection,
) -> Result<DirectionRecord, HandoverError> {
    Ok(DirectionRecord {
        tail: blob.push(&frozen.tail)?,
        source_eof: frozen.source_eof,
        copied: frozen.copied,
    })
}

fn frozen_direction(
    tails: &[u8],
    record: DirectionRecord,
) -> Result<FrozenDirection, HandoverError> {
    Ok(FrozenDirection {
        tail: Bytes::copy_from_slice(tail(tails, record.tail)?),
        source_eof: record.source_eof,
        copied: record.copied,
    })
}

const fn side_record(side: RelaySide) -> SideRecord {
    match side {
        RelaySide::Client => SideRecord::Client,
        RelaySide::Backend => SideRecord::Backend,
    }
}

const fn relay_side(side: SideRecord) -> RelaySide {
    match side {
        SideRecord::Client => RelaySide::Client,
        SideRecord::Backend => RelaySide::Backend,
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::io::{ErrorKind, Read, Write};

    use infrarust_config::ProxyMode;
    use uuid::Uuid;

    use super::*;
    use crate::hot_restart::state::{ProfileRecord, decode, encode};

    fn pair(listener: &TcpListener) -> (TcpStream, TcpStream) {
        let outer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (inner, _) = listener.accept().unwrap();
        (outer, inner)
    }

    fn meta(player_id: u64, client_addr: SocketAddr) -> SessionMeta {
        SessionMeta {
            player_id,
            profile: ProfileRecord {
                uuid: Uuid::from_u128(u128::from(player_id)),
                username: format!("player{player_id}"),
                properties: Vec::new(),
            },
            protocol_version: 769,
            mode: ProxyMode::Passthrough,
            client_addr,
            server: "lobby".into(),
            connected_address: None,
            virtual_host: None,
            connected_at_ms: 0,
        }
    }

    fn listening(bind: &str) -> ListenerHandover {
        ListenerHandover {
            socket: TcpListener::bind("127.0.0.1:0").unwrap(),
            bind: bind.parse().unwrap(),
        }
    }

    fn is_open(stream: &mut TcpStream) -> bool {
        stream.set_nonblocking(true).unwrap();
        let mut buf = [0; 1];
        let open = matches!(stream.read(&mut buf), Err(e) if e.kind() == ErrorKind::WouldBlock);
        stream.set_nonblocking(false).unwrap();
        open
    }

    #[test]
    fn a_package_survives_encoding_and_restores_live_sockets() {
        let front = TcpListener::bind("127.0.0.1:0").unwrap();
        let back = TcpListener::bind("127.0.0.1:0").unwrap();
        let (mut player, proxy_client) = pair(&front);
        let (proxy_backend, mut server) = pair(&back);

        let frozen = FrozenSession {
            meta: meta(7, player.local_addr().unwrap()),
            relay: FrozenRelay {
                client: proxy_client,
                backend: proxy_backend,
                client_to_backend: FrozenDirection {
                    tail: Bytes::from_static(b"to-server"),
                    source_eof: false,
                    copied: 3,
                },
                backend_to_client: FrozenDirection::default(),
                first_closed: None,
            },
        };
        let accept = listening("0.0.0.0:25565");
        let accept_addr = accept.socket.local_addr().unwrap();
        let (package, dropped) =
            HandoverPackage::build("2.0.0-beta.6", 8, Some(accept), vec![frozen]).unwrap();
        assert!(dropped.is_empty());
        assert_eq!(package.handles.len(), 3);
        let created_at_ms = package.state.created_at_ms;

        let bytes = encode(&package.state, &package.tails).unwrap();
        let (state, tails) = decode(&bytes).unwrap();
        let restored = HandoverPackage {
            state,
            tails,
            handles: package.handles,
        };

        let unpacked = restored.unpack().unwrap();
        assert_eq!(unpacked.next_player_id, 8);
        assert_eq!(unpacked.created_at_ms, created_at_ms);
        let listener = unpacked.listener.as_ref().unwrap();
        assert_eq!(listener.socket.local_addr().unwrap(), accept_addr);
        assert_eq!(listener.bind, "0.0.0.0:25565".parse().unwrap());
        assert!(unpacked.dropped.is_empty());
        assert!(unpacked.held.is_empty());
        let session = unpacked.sessions.into_iter().next().unwrap();
        assert_eq!(session.meta.player_id, 7);
        assert_eq!(&session.relay.client_to_backend.tail[..], b"to-server");
        assert_eq!(session.relay.client_to_backend.copied, 3);

        let mut client = session.relay.client;
        let mut backend = session.relay.backend;
        client.write_all(b"hello").unwrap();
        let mut buf = [0; 5];
        player.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"hello");
        backend.write_all(b"world").unwrap();
        server.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"world");
    }

    #[test]
    fn a_swapped_socket_is_dropped_alone_and_its_sockets_are_held() {
        let front = TcpListener::bind("127.0.0.1:0").unwrap();
        let back = TcpListener::bind("127.0.0.1:0").unwrap();
        let (mut player_a, client_a) = pair(&front);
        let (backend_a, mut server_a) = pair(&back);
        let (_player_b, client_b) = pair(&front);
        let (backend_b, _server_b) = pair(&back);

        let session = |id, client, backend| FrozenSession {
            meta: meta(id, SocketAddr::from(([127, 0, 0, 1], 1))),
            relay: FrozenRelay {
                client,
                backend,
                client_to_backend: FrozenDirection::default(),
                backend_to_client: FrozenDirection::default(),
                first_closed: Some(RelaySide::Client),
            },
        };
        let (mut package, _) = HandoverPackage::build(
            "v",
            1,
            None,
            vec![
                session(1, client_a, backend_a),
                session(2, client_b, backend_b),
            ],
        )
        .unwrap();
        let first = &mut package.state.sessions[0];
        std::mem::swap(&mut first.client.local, &mut first.client.peer);

        let unpacked = package.unpack().unwrap();
        assert_eq!(unpacked.dropped.len(), 1);
        assert_eq!(unpacked.dropped[0].meta.player_id, 1);
        assert!(matches!(
            unpacked.dropped[0].error,
            HandoverError::Mismatch(..)
        ));
        assert_eq!(unpacked.sessions.len(), 1);
        assert_eq!(
            unpacked.sessions[0].relay.first_closed,
            Some(RelaySide::Client)
        );
        assert_eq!(unpacked.held.len(), 2);
        assert!(is_open(&mut player_a));
        assert!(is_open(&mut server_a));

        drop(unpacked);

        let mut buf = [0; 1];
        assert_eq!(player_a.read(&mut buf).unwrap(), 0);
        assert_eq!(server_a.read(&mut buf).unwrap(), 0);
    }

    #[test]
    fn a_broken_listener_record_hands_the_whole_package_back() {
        let front = TcpListener::bind("127.0.0.1:0").unwrap();
        let back = TcpListener::bind("127.0.0.1:0").unwrap();
        let (mut player, client) = pair(&front);
        let (backend, mut server) = pair(&back);
        let frozen = FrozenSession {
            meta: meta(3, player.local_addr().unwrap()),
            relay: FrozenRelay {
                client,
                backend,
                client_to_backend: FrozenDirection::default(),
                backend_to_client: FrozenDirection::default(),
                first_closed: None,
            },
        };
        let (mut package, _) =
            HandoverPackage::build("v", 4, Some(listening("0.0.0.0:25565")), vec![frozen]).unwrap();
        let record = package.state.listener.as_mut().unwrap();
        let real = record.local_addr;
        record.local_addr = SocketAddr::from(([127, 0, 0, 1], 1));

        let failure = package.unpack().unwrap_err();

        assert!(
            matches!(failure.error, HandoverError::Mismatch(0, _)),
            "{failure}"
        );
        assert_eq!(failure.package.handles.iter().count(), 3);
        assert!(is_open(&mut player));
        assert!(is_open(&mut server));

        let mut package = *failure.package;
        package.state.listener.as_mut().unwrap().local_addr = real;
        let unpacked = package.unpack().unwrap();
        assert!(unpacked.listener.is_some());
        assert_eq!(unpacked.sessions.len(), 1);
    }

    #[test]
    fn a_listener_record_pointing_at_a_stream_is_refused_without_taking_anything() {
        let front = TcpListener::bind("127.0.0.1:0").unwrap();
        let (_player, client) = pair(&front);
        let package = HandoverPackage {
            state: HandoverState {
                protocol: HANDOVER_PROTOCOL,
                from_version: "v".into(),
                created_at_ms: 0,
                next_player_id: 1,
                listener: Some(ListenerRecord {
                    handle: HandleId(0),
                    local_addr: client.local_addr().unwrap(),
                    bind: client.local_addr().unwrap(),
                }),
                sessions: Vec::new(),
            },
            tails: Vec::new(),
            handles: HandleTable::from_handles(vec![Handle::Stream(client)]),
        };

        let failure = package.unpack().unwrap_err();

        assert!(matches!(
            failure.error,
            HandoverError::WrongHandle(0, "listener")
        ));
        assert!(failure.package.handles.get(HandleId(0)).is_some());
    }

    #[test]
    fn handles_no_record_points_at_are_held() {
        let stray = TcpListener::bind("127.0.0.1:0").unwrap();
        let (mut package, _) = HandoverPackage::build("v", 1, None, Vec::new()).unwrap();
        package.handles.push(Handle::Listener(stray)).unwrap();

        let unpacked = package.unpack().unwrap();

        assert!(matches!(unpacked.held.as_slice(), [Handle::Listener(_)]));
    }

    #[test]
    fn handles_can_be_taken_once_and_by_kind() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut table = HandleTable::default();
        let id = table.push(Handle::Listener(listener)).unwrap();
        assert!(table.get(id).is_some());
        assert!(matches!(
            table.take_stream(id),
            Err(HandoverError::WrongHandle(0, "stream"))
        ));
        assert!(table.get(id).is_none());
        assert!(matches!(
            table.take_listener(id),
            Err(HandoverError::MissingHandle(0))
        ));
        assert!(matches!(
            table.take(HandleId(9)),
            Err(HandoverError::MissingHandle(9))
        ));
    }
}

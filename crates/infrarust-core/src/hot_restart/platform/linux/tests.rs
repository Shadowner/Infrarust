#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::RefCell;
use std::ffi::OsString;
use std::io::{self, ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, IntoRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bytes::Bytes;
use infrarust_config::ProxyMode;
use infrarust_transport::handover::{
    HandoverSignal, SocketKind, claim_inherited, is_blocked, is_inheritable, sealed_memfd,
    set_inheritable,
};
use infrarust_transport::{FrozenDirection, FrozenRelay};
use uuid::Uuid;

use super::envelope::{self, EnvelopeHeader};
use super::launch::{ATTEMPT_VAR, Attempt, Failpoint, HANDOVER_FD_VAR, HandoverVars, Launch};
use super::{hand_over, inherit_from, owned_fds};
use crate::binary::{BuildInfo, ExeSnapshot, RunningBinary};
use crate::hot_restart::HandoverError;
use crate::hot_restart::package::{FrozenSession, Handle, HandoverPackage, ListenerHandover};
use crate::hot_restart::platform::{Inheritance, RestartPlan};
use crate::hot_restart::state::{HandoverState, ProfileRecord, SessionMeta};

const VERSION: &str = "2.0.0-beta.6";
const MISSING_EXE: &str = "/nonexistent/infrarust";

pub(super) struct Peers {
    pub players: Vec<TcpStream>,
    pub servers: Vec<TcpStream>,
    _front: TcpListener,
    _back: TcpListener,
}

pub(super) fn frozen_package(sessions: usize) -> (HandoverPackage, Peers) {
    let (package, peers, _) = build(sessions, false);
    (package, peers)
}

fn build(sessions: usize, watch: bool) -> (HandoverPackage, Peers, Vec<TcpStream>) {
    let front = TcpListener::bind("127.0.0.1:0").unwrap();
    let back = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut players = Vec::new();
    let mut servers = Vec::new();
    let mut watches = Vec::new();
    let mut frozen = Vec::new();
    for index in 0..sessions {
        let (player, client) = pair(&front);
        let (backend, server) = pair(&back);
        if watch {
            watches.push(client.try_clone().unwrap());
        }
        frozen.push(FrozenSession {
            meta: meta(index as u64 + 1, player.local_addr().unwrap()),
            relay: FrozenRelay {
                client,
                backend,
                client_to_backend: FrozenDirection {
                    tail: Bytes::from(format!("tail-{index}")),
                    source_eof: false,
                    copied: index as u64,
                },
                backend_to_client: FrozenDirection::default(),
                first_closed: None,
            },
        });
        players.push(player);
        servers.push(server);
    }
    let accept = TcpListener::bind("127.0.0.1:0").unwrap();
    let bind = accept.local_addr().unwrap();
    let listener = ListenerHandover {
        socket: accept,
        bind,
    };
    let (package, dropped) = HandoverPackage::build(VERSION, 100, Some(listener), frozen).unwrap();
    assert!(dropped.is_empty());
    let peers = Peers {
        players,
        servers,
        _front: front,
        _back: back,
    };
    (package, peers, watches)
}

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

struct Sent {
    memfd: RawFd,
    header: EnvelopeHeader,
    state: HandoverState,
}

fn send(
    package: HandoverPackage,
    image: Option<RawFd>,
    exe: &Path,
    edit: impl FnOnce(&mut Vec<RawFd>),
) -> Sent {
    let HandoverPackage {
        state,
        tails,
        mut handles,
    } = package;
    let fds = owned_fds(&mut handles).unwrap();
    for fd in &fds {
        set_inheritable(fd.as_fd(), true).unwrap();
    }
    let mut raw: Vec<RawFd> = fds.into_iter().map(IntoRawFd::into_raw_fd).collect();
    edit(&mut raw);
    let header = EnvelopeHeader {
        handles: raw,
        image,
        exe: exe.to_path_buf(),
        version: VERSION.into(),
    };
    let bytes = envelope::encode(&header, &state, &tails).unwrap();
    let memfd = sealed_memfd("infrarust-handover-test", &bytes).unwrap();
    set_inheritable(memfd.as_fd(), true).unwrap();
    Sent {
        memfd: memfd.into_raw_fd(),
        header,
        state,
    }
}

const fn vars(fd: RawFd, attempt: Attempt) -> HandoverVars {
    HandoverVars {
        fd,
        attempt,
        failpoint: None,
    }
}

fn argv() -> Vec<OsString> {
    ["infrarust", "-c", "proxy.toml"]
        .into_iter()
        .map(OsString::from)
        .collect()
}

#[derive(Debug)]
struct Recorded {
    program: PathBuf,
    argv0: OsString,
    args: Vec<OsString>,
    memfd: Option<RawFd>,
    attempt: Option<String>,
    header: Option<EnvelopeHeader>,
    inheritable: Vec<bool>,
    blocked: bool,
}

thread_local! {
    static RECORDED: RefCell<Vec<Recorded>> = const { RefCell::new(Vec::new()) };
}

fn record(launch: &Launch<'_>) -> io::Error {
    let value = |key: &str| {
        launch
            .env
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.to_string_lossy().into_owned())
    };
    let memfd = value(HANDOVER_FD_VAR).and_then(|fd| fd.parse::<RawFd>().ok());
    let header = memfd
        .and_then(|fd| std::fs::read(format!("/proc/self/fd/{fd}")).ok())
        .and_then(|bytes| envelope::decode(&bytes).ok())
        .map(|envelope| envelope.header);
    let inheritable = header
        .iter()
        .flat_map(|header| header.handles.iter().copied().chain(header.image))
        .chain(memfd)
        .map(inheritable_by_number)
        .collect();
    let blocked = HandoverSignal::ALL
        .into_iter()
        .all(|signal| is_blocked(signal).unwrap());
    RECORDED.with(|recorded| {
        recorded.borrow_mut().push(Recorded {
            program: launch.program.to_path_buf(),
            argv0: launch.argv0.to_os_string(),
            args: launch.args.to_vec(),
            memfd,
            attempt: value(ATTEMPT_VAR),
            header,
            inheritable,
            blocked,
        });
    });
    io::Error::other("recorded instead of executed")
}

fn refuse(launch: &Launch<'_>) -> io::Error {
    panic!("{} must not be executed", launch.program.display())
}

fn recorded() -> Vec<Recorded> {
    RECORDED.with(RefCell::take)
}

fn inheritable_by_number(fd: RawFd) -> bool {
    let info = std::fs::read_to_string(format!("/proc/self/fdinfo/{fd}")).unwrap();
    let flags = info
        .lines()
        .find_map(|line| line.strip_prefix("flags:"))
        .unwrap()
        .trim();
    u32::from_str_radix(flags, 8).unwrap() & 0o2_000_000 == 0
}

fn no_signal_blocked() -> bool {
    HandoverSignal::ALL
        .into_iter()
        .all(|signal| !is_blocked(signal).unwrap())
}

fn handle_fd(handle: &Handle) -> BorrowedFd<'_> {
    match handle {
        Handle::Listener(listener) => listener.as_fd(),
        Handle::Stream(stream) => stream.as_fd(),
    }
}

fn assert_open(stream: &mut TcpStream) {
    stream.set_nonblocking(true).unwrap();
    let mut buf = [0; 1];
    let read = stream.read(&mut buf);
    assert!(
        matches!(&read, Err(error) if error.kind() == ErrorKind::WouldBlock),
        "the peer saw {read:?}"
    );
    stream.set_nonblocking(false).unwrap();
}

fn assert_closed(stream: &mut TcpStream) {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut buf = [0; 1];
    assert_eq!(stream.read(&mut buf).unwrap(), 0);
}

struct Images {
    _dir: tempfile::TempDir,
    image: RawFd,
    exe: PathBuf,
    old: PathBuf,
}

fn images() -> Images {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("infrarust");
    let old = dir.path().join("infrarust.old");
    let image = dir.path().join("image");
    std::fs::write(&exe, b"new").unwrap();
    std::fs::write(&old, b"old").unwrap();
    std::fs::write(&image, b"running").unwrap();
    let image = std::fs::File::open(image).unwrap();
    set_inheritable(image.as_fd(), true).unwrap();
    let image = image.into_raw_fd();
    Images {
        _dir: dir,
        image,
        exe,
        old,
    }
}

fn plan(target: &str) -> RestartPlan {
    RestartPlan {
        binary: RunningBinary::new(
            BuildInfo {
                version: VERSION,
                target: "x86_64-unknown-linux-musl",
                official: false,
                docker: false,
            },
            ExeSnapshot::capture().unwrap(),
            vec!["-c".into(), "/etc/infrarust/proxy.toml".into()],
        ),
        target: PathBuf::from(target),
    }
}

#[test]
fn an_inherited_package_carries_live_sockets_once() {
    let (package, mut peers) = frozen_package(2);
    let sent = send(package, None, Path::new(MISSING_EXE), |_| {});
    let mut inheritance = inherit_from(vars(sent.memfd, Attempt::First), argv(), refuse).unwrap();

    let mut received = inheritance.take_package().unwrap();

    assert_eq!(received.state, sent.state);
    assert_eq!(received.handles.len(), 5);
    let sessions = received.state.sessions.clone();
    for (index, session) in sessions.iter().enumerate() {
        let mut client = received.handles.take_stream(session.client.handle).unwrap();
        let mut backend = received
            .handles
            .take_stream(session.backend.handle)
            .unwrap();
        client.write_all(b"ping").unwrap();
        backend.write_all(b"pong").unwrap();
        let mut buf = [0; 4];
        peers.players[index].read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"ping");
        peers.servers[index].read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"pong");
    }
    let listener_record = received.state.listener.clone().unwrap();
    let listener = received
        .handles
        .take_listener(listener_record.handle)
        .unwrap();
    let _client = TcpStream::connect(listener_record.local_addr).unwrap();
    listener.accept().unwrap();

    assert!(matches!(
        inheritance.take_package(),
        Err(HandoverError::Malformed(_))
    ));
}

#[test]
fn a_mismatched_descriptor_fails_the_take_and_closes_nothing() {
    let (package, mut peers) = frozen_package(1);
    let sent = send(package, None, Path::new(MISSING_EXE), |handles| {
        handles.swap(1, 2);
    });
    let mut inheritance = inherit_from(vars(sent.memfd, Attempt::First), argv(), refuse).unwrap();

    let error = inheritance.take_package().unwrap_err();

    assert!(matches!(error, HandoverError::Mismatch(1, _)), "{error}");
    assert!(matches!(
        inheritance.take_package(),
        Err(HandoverError::Malformed(_))
    ));
    assert_open(&mut peers.players[0]);
    assert_open(&mut peers.servers[0]);

    drop(inheritance);
    assert_closed(&mut peers.servers[0]);
    assert_open(&mut peers.players[0]);
    drop(claim_inherited(sent.header.handles[2], SocketKind::Stream).unwrap());
    assert_closed(&mut peers.players[0]);
}

#[test]
fn a_session_whose_player_reset_is_still_taken() {
    let (package, mut peers, mut watches) = build(2, true);
    let sent = send(package, None, Path::new(MISSING_EXE), |_| {});
    watches[0].write_all(b"unread").unwrap();
    let player = peers.players.remove(0);
    let deadline = Instant::now() + Duration::from_secs(5);
    while player.peek(&mut [0; 6]).unwrap() < 6 {
        assert!(Instant::now() < deadline, "the bytes never arrived");
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(player);
    while watches[0].peer_addr().is_ok() {
        assert!(Instant::now() < deadline, "the reset never arrived");
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut inheritance = inherit_from(vars(sent.memfd, Attempt::First), argv(), refuse).unwrap();

    let received = inheritance.take_package().unwrap();

    assert_eq!(received.handles.len(), 5);
    let mut buf = [0; 4];
    watches[1].write_all(b"live").unwrap();
    peers.players[0].read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"live");
}

#[test]
fn commit_closes_every_inherited_socket_on_exec() {
    let (package, _peers) = frozen_package(1);
    let sent = send(package, None, Path::new(MISSING_EXE), |_| {});
    let mut inheritance = inherit_from(vars(sent.memfd, Attempt::First), argv(), refuse).unwrap();
    let received = inheritance.take_package().unwrap();
    assert_eq!(received.handles.len(), 3);
    for (_, handle) in received.handles.iter() {
        assert!(is_inheritable(handle_fd(handle)).unwrap());
    }

    Box::new(inheritance).commit().unwrap();

    for (_, handle) in received.handles.iter() {
        assert!(!is_inheritable(handle_fd(handle)).unwrap());
    }
}

#[test]
fn a_roll_back_at_the_second_attempt_returns_without_exec() {
    let (package, mut peers) = frozen_package(1);
    let sent = send(package, None, Path::new(MISSING_EXE), |_| {});
    let mut inheritance = inherit_from(vars(sent.memfd, Attempt::Second), argv(), refuse).unwrap();
    let _held = inheritance.take_package().unwrap();

    let error = Box::new(inheritance).roll_back("the config is invalid");

    assert!(
        matches!(&error, HandoverError::RolledBack(reason) if reason == "the config is invalid"),
        "{error}"
    );
    assert_open(&mut peers.players[0]);
}

#[test]
fn a_roll_back_at_the_first_attempt_execs_each_older_image_with_the_same_memfd() {
    let images = images();
    let (package, _peers) = frozen_package(1);
    let sent = send(package, Some(images.image), &images.exe, |_| {});
    let mut inheritance = inherit_from(vars(sent.memfd, Attempt::First), argv(), record).unwrap();
    let _held = inheritance.take_package().unwrap();

    let error = Box::new(inheritance).roll_back("the config is invalid");

    assert!(matches!(error, HandoverError::Io(_)), "{error}");
    let launches = recorded();
    assert_eq!(
        launches
            .iter()
            .map(|l| l.program.clone())
            .collect::<Vec<_>>(),
        vec![
            PathBuf::from(format!("/proc/self/fd/{}", images.image)),
            images.old.clone(),
            images.exe.clone(),
        ]
    );
    for launch in &launches {
        assert_eq!(launch.argv0, "infrarust");
        assert_eq!(launch.args, argv()[1..]);
        assert_eq!(launch.memfd, Some(sent.memfd));
        assert_eq!(launch.attempt.as_deref(), Some("2"));
        assert_eq!(launch.header.as_ref(), Some(&sent.header));
        assert_eq!(launch.inheritable.len(), 5);
        assert!(launch.inheritable.iter().all(|inheritable| *inheritable));
        assert!(launch.blocked);
    }
    assert!(no_signal_blocked());
}

#[test]
fn a_roll_back_without_any_older_image_fails_without_exec() {
    let (package, _peers) = frozen_package(0);
    let sent = send(package, None, Path::new(MISSING_EXE), |_| {});
    let inheritance = inherit_from(vars(sent.memfd, Attempt::First), argv(), refuse).unwrap();

    let error = Box::new(inheritance).roll_back("the config is invalid");

    assert!(matches!(error, HandoverError::Io(_)), "{error}");
}

#[cfg(debug_assertions)]
#[test]
fn the_successor_precommit_failpoint_rolls_back_instead_of_committing() {
    let images = images();
    let (package, _peers) = frozen_package(1);
    let sent = send(package, Some(images.image), &images.exe, |_| {});
    let mut inheritance = inherit_from(
        HandoverVars {
            failpoint: Some(Failpoint::SuccessorPrecommit),
            ..vars(sent.memfd, Attempt::First)
        },
        argv(),
        record,
    )
    .unwrap();
    let received = inheritance.take_package().unwrap();

    let error = Box::new(inheritance).commit().unwrap_err();

    assert!(matches!(error, HandoverError::Io(_)), "{error}");
    let launches = recorded();
    assert_eq!(launches.len(), 3);
    assert!(
        launches
            .iter()
            .all(|launch| launch.attempt.as_deref() == Some("2"))
    );
    for (_, handle) in received.handles.iter() {
        assert!(is_inheritable(handle_fd(handle)).unwrap());
    }
}

#[test]
fn an_envelope_fd_that_is_not_a_sealed_memfd_is_refused() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();

    let error =
        inherit_from(vars(listener.as_raw_fd(), Attempt::First), argv(), refuse).unwrap_err();

    assert!(matches!(error, HandoverError::Io(_)), "{error}");
    let _client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    listener.accept().unwrap();
}

#[test]
fn hand_over_execs_the_target_then_the_running_image() {
    let (package, _peers) = frozen_package(1);
    let plan = plan("/opt/next/infrarust");
    let image = plan.binary.exe.image().unwrap().as_raw_fd();

    let error = hand_over(package, &plan, record, None).unwrap_err();

    assert!(matches!(error, HandoverError::Io(_)), "{error}");
    let launches = recorded();
    assert_eq!(
        launches
            .iter()
            .map(|l| l.program.clone())
            .collect::<Vec<_>>(),
        vec![
            PathBuf::from("/opt/next/infrarust"),
            PathBuf::from(format!("/proc/self/fd/{image}")),
            plan.binary.exe.path().to_path_buf(),
        ]
    );
    assert_eq!(
        launches
            .iter()
            .map(|l| l.attempt.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("1"), Some("2"), Some("2")]
    );
    let memfd = launches[0].memfd.unwrap();
    for launch in &launches {
        assert_eq!(launch.argv0, std::env::args_os().next().unwrap());
        assert_eq!(launch.args, plan.binary.args);
        assert_eq!(launch.memfd, Some(memfd));
        let header = launch.header.as_ref().unwrap();
        assert_eq!(header.handles.len(), 3);
        assert_eq!(header.image, Some(image));
        assert_eq!(header.exe, plan.binary.exe.path());
        assert_eq!(header.version, VERSION);
        assert_eq!(launch.inheritable, vec![true; 5]);
        assert!(launch.blocked);
    }
    assert!(!is_inheritable(plan.binary.exe.image().unwrap().as_fd()).unwrap());
    assert!(no_signal_blocked());
}

#[cfg(debug_assertions)]
#[test]
fn the_exec_target_failpoint_goes_straight_to_the_running_image() {
    let (package, _peers) = frozen_package(1);
    let plan = plan("/opt/next/infrarust");
    let image = plan.binary.exe.image().unwrap().as_raw_fd();

    let error = hand_over(package, &plan, record, Some(Failpoint::ExecTarget)).unwrap_err();

    assert!(matches!(error, HandoverError::Io(_)), "{error}");
    let launches = recorded();
    assert_eq!(
        launches
            .iter()
            .map(|l| l.program.clone())
            .collect::<Vec<_>>(),
        vec![
            PathBuf::from(format!("/proc/self/fd/{image}")),
            plan.binary.exe.path().to_path_buf(),
        ]
    );
    assert!(
        launches
            .iter()
            .all(|launch| launch.attempt.as_deref() == Some("2"))
    );
}

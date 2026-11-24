#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::os::fd::{AsFd, AsRawFd, IntoRawFd};
use std::os::unix::net::UnixStream;

use nix::errno::Errno;
use nix::fcntl::{FcntlArg, FdFlag, fcntl};
use nix::unistd::{dup, pipe};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use rusty_fork::rusty_fork_test;
use socket2::{Domain, Socket, Type};

use infrarust_transport::handover::{
    InheritedSocket, SocketIdentity, SocketKind, claim_inherited, set_inheritable, socket_identity,
};

fn close_on_exec(fd: impl AsFd) -> bool {
    FdFlag::from_bits_retain(fcntl(fd.as_fd(), FcntlArg::F_GETFD).unwrap())
        .contains(FdFlag::FD_CLOEXEC)
}

fn is_open(fd: impl AsFd) -> bool {
    fcntl(fd.as_fd(), FcntlArg::F_GETFD).is_ok()
}

fn connected() -> (TcpListener, TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (server, _) = listener.accept().unwrap();
    (listener, client, server)
}

#[test]
fn set_inheritable_toggles_close_on_exec() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    assert!(close_on_exec(&listener));

    set_inheritable(listener.as_fd(), true).unwrap();
    assert!(!close_on_exec(&listener));

    set_inheritable(listener.as_fd(), true).unwrap();
    assert!(!close_on_exec(&listener));

    set_inheritable(listener.as_fd(), false).unwrap();
    assert!(close_on_exec(&listener));
}

#[test]
fn a_listener_is_identified_without_a_peer() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    assert_eq!(
        socket_identity(listener.as_fd()).unwrap(),
        SocketIdentity {
            kind: SocketKind::Listener,
            local: listener.local_addr().unwrap(),
            peer: None,
        }
    );
}

#[test]
fn an_ipv6_listener_is_identified() {
    let Ok(listener) = TcpListener::bind("[::1]:0") else {
        return;
    };
    let identity = socket_identity(listener.as_fd()).unwrap();
    assert_eq!(identity.kind, SocketKind::Listener);
    assert_eq!(identity.local, listener.local_addr().unwrap());
}

#[test]
fn a_connected_stream_is_identified_with_its_peer() {
    let (listener, client, server) = connected();

    assert_eq!(
        socket_identity(server.as_fd()).unwrap(),
        SocketIdentity {
            kind: SocketKind::Stream,
            local: server.local_addr().unwrap(),
            peer: Some(client.local_addr().unwrap()),
        }
    );
    assert_eq!(
        socket_identity(client.as_fd()).unwrap().peer,
        Some(listener.local_addr().unwrap())
    );
}

#[test]
fn a_udp_socket_is_rejected() {
    let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
    let err = socket_identity(udp.as_fd()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
}

#[test]
fn a_pipe_is_rejected() {
    let (read, _write) = pipe().unwrap();
    let err = socket_identity(read.as_fd()).unwrap_err();
    assert_eq!(err.raw_os_error(), Some(Errno::ENOTSOCK as i32), "{err}");
}

#[test]
fn a_unix_stream_socket_is_rejected() {
    let (left, _right) = UnixStream::pair().unwrap();
    let err = socket_identity(left.as_fd()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
}

#[test]
fn an_unconnected_stream_socket_is_rejected() {
    let socket = Socket::new(Domain::IPV4, Type::STREAM, None).unwrap();
    socket
        .bind(
            &"127.0.0.1:0"
                .parse::<std::net::SocketAddr>()
                .unwrap()
                .into(),
        )
        .unwrap();
    let err = socket_identity(socket.as_fd()).unwrap_err();
    assert_eq!(err.raw_os_error(), Some(Errno::ENOTCONN as i32), "{err}");
}

#[test]
fn a_claimed_stream_carries_data_both_ways() {
    let (_listener, mut client, server) = connected();
    let raw = dup(&server).unwrap().into_raw_fd();
    drop(server);

    let InheritedSocket::Stream(mut claimed) = claim_inherited(raw, SocketKind::Stream).unwrap()
    else {
        panic!("expected a stream");
    };
    assert_eq!(claimed.as_raw_fd(), raw);
    assert!(!close_on_exec(&claimed));

    claimed.write_all(b"ping").unwrap();
    let mut buf = [0; 4];
    client.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"ping");

    client.write_all(b"pong").unwrap();
    claimed.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"pong");
}

#[test]
fn a_claimed_listener_accepts_connections() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let raw = dup(&listener).unwrap().into_raw_fd();
    drop(listener);

    let claimed = claim_inherited(raw, SocketKind::Listener).unwrap();
    assert_eq!(socket_identity(claimed.as_fd()).unwrap().local, addr);
    let InheritedSocket::Listener(claimed) = claimed else {
        panic!("expected a listener");
    };

    let client = TcpStream::connect(addr).unwrap();
    let (_, peer) = claimed.accept().unwrap();
    assert_eq!(peer, client.local_addr().unwrap());
}

#[test]
fn a_claim_of_the_wrong_kind_leaves_the_fd_open() {
    let (listener, _client, server) = connected();

    let err = claim_inherited(listener.as_raw_fd(), SocketKind::Stream).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    assert!(is_open(&listener));

    let err = claim_inherited(server.as_raw_fd(), SocketKind::Listener).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    assert!(is_open(&server));
}

#[test]
fn a_claim_of_a_udp_socket_leaves_the_fd_open() {
    let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
    for kind in [SocketKind::Listener, SocketKind::Stream] {
        let err = claim_inherited(udp.as_raw_fd(), kind).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    }
    assert!(is_open(&udp));
}

#[test]
fn a_claim_of_a_negative_fd_is_refused() {
    let err = claim_inherited(-1, SocketKind::Stream).unwrap_err();
    assert_eq!(err.raw_os_error(), Some(Errno::EBADF as i32), "{err}");
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
rusty_fork_test! {
    #[test]
    fn a_claim_of_a_closed_fd_is_refused() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let duplicate = dup(&listener).unwrap();
        let raw = duplicate.as_raw_fd();
        drop(duplicate);

        let err = claim_inherited(raw, SocketKind::Listener).unwrap_err();
        assert_eq!(err.raw_os_error(), Some(Errno::EBADF as i32), "{err}");
        assert!(is_open(&listener));
    }
}

#[cfg(target_os = "linux")]
mod memfd {
    use std::os::fd::AsFd;

    use nix::errno::Errno;
    use nix::fcntl::{FcntlArg, SealFlag, fcntl};
    use nix::sys::memfd::{MFdFlags, memfd_create};
    use nix::sys::uio::pwrite;
    use nix::unistd::{Whence, ftruncate, lseek, write};

    use infrarust_transport::handover::{read_sealed, sealed_memfd};

    use super::close_on_exec;

    #[test]
    fn a_sealed_memfd_reads_back_the_same_bytes_twice() {
        let payload: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let fd = sealed_memfd("infrarust-handover-test", &payload).unwrap();
        assert!(close_on_exec(&fd));

        let offset = lseek(&fd, 0, Whence::SeekCur).unwrap();
        assert_eq!(read_sealed(fd.as_fd()).unwrap(), payload);
        assert_eq!(read_sealed(fd.as_fd()).unwrap(), payload);
        assert_eq!(lseek(&fd, 0, Whence::SeekCur).unwrap(), offset);
    }

    #[test]
    fn an_empty_payload_round_trips() {
        let fd = sealed_memfd("infrarust-handover-empty", b"").unwrap();
        assert!(read_sealed(fd.as_fd()).unwrap().is_empty());
    }

    #[test]
    fn a_sealed_memfd_refuses_every_change() {
        let fd = sealed_memfd("infrarust-handover-test", b"state").unwrap();

        let seals = SealFlag::from_bits_retain(fcntl(&fd, FcntlArg::F_GET_SEALS).unwrap());
        assert!(seals.contains(
            SealFlag::F_SEAL_SHRINK
                | SealFlag::F_SEAL_GROW
                | SealFlag::F_SEAL_WRITE
                | SealFlag::F_SEAL_SEAL
        ));

        assert_eq!(write(&fd, b"x"), Err(Errno::EPERM));
        assert_eq!(pwrite(&fd, b"x", 0), Err(Errno::EPERM));
        assert_eq!(ftruncate(&fd, 0), Err(Errno::EPERM));
        assert_eq!(
            fcntl(&fd, FcntlArg::F_ADD_SEALS(SealFlag::F_SEAL_FUTURE_WRITE)),
            Err(Errno::EPERM)
        );
        assert_eq!(read_sealed(fd.as_fd()).unwrap(), b"state");
    }

    #[test]
    fn an_unsealed_memfd_is_not_read() {
        let fd = memfd_create("infrarust-handover-unsealed", MFdFlags::MFD_CLOEXEC).unwrap();
        write(&fd, b"state").unwrap();
        let err = read_sealed(fd.as_fd()).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData, "{err}");
    }
}

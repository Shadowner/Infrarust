#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::os::fd::{AsFd, AsRawFd, IntoRawFd};
use std::os::unix::net::UnixStream;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use nix::errno::Errno;
use nix::fcntl::{FcntlArg, FdFlag, fcntl};
use nix::unistd::{dup, pipe};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use rusty_fork::rusty_fork_test;
use socket2::{Domain, Socket, Type};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use infrarust_transport::handover::{
    HandoverSignal, block_signals, is_blocked, raise_signal, record_signal, unblock_signals,
};
use infrarust_transport::handover::{
    InheritedSocket, SocketIdentity, SocketKind, claim_inherited, close_on_exec_raw,
    is_inheritable, set_inheritable, socket_identity,
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
fn is_inheritable_reads_the_close_on_exec_flag() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    assert!(!is_inheritable(listener.as_fd()).unwrap());
    set_inheritable(listener.as_fd(), true).unwrap();
    assert!(is_inheritable(listener.as_fd()).unwrap());
}

#[test]
fn close_on_exec_raw_sets_the_flag_by_number() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    set_inheritable(listener.as_fd(), true).unwrap();

    close_on_exec_raw(listener.as_raw_fd()).unwrap();

    assert!(close_on_exec(&listener));
    let err = close_on_exec_raw(-1).unwrap_err();
    assert_eq!(err.raw_os_error(), Some(Errno::EBADF as i32), "{err}");
}

#[test]
fn a_stream_whose_peer_reset_can_still_be_claimed() {
    let (_listener, client, server) = connected();
    Socket::from(client)
        .set_linger(Some(Duration::ZERO))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while server.peer_addr().is_ok() {
        assert!(Instant::now() < deadline, "the reset never arrived");
        std::thread::sleep(Duration::from_millis(5));
    }
    let raw = dup(&server).unwrap().into_raw_fd();
    drop(server);

    let claimed = claim_inherited(raw, SocketKind::Stream).unwrap();

    let err = socket_identity(claimed.as_fd()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotConnected, "{err}");
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

    #[test]
    fn a_blocked_signal_is_recorded_once_it_is_unblocked() {
        static RECEIVED: AtomicU32 = AtomicU32::new(0);
        let recorder = record_signal(HandoverSignal::Restart, &RECEIVED).unwrap();

        block_signals(&[HandoverSignal::Restart]).unwrap();
        assert!(is_blocked(HandoverSignal::Restart).unwrap());
        assert!(!is_blocked(HandoverSignal::Terminate).unwrap());
        raise_signal(HandoverSignal::Restart).unwrap();
        assert_eq!(RECEIVED.load(Ordering::SeqCst), 0);

        unblock_signals(&[HandoverSignal::Restart]).unwrap();
        assert!(!is_blocked(HandoverSignal::Restart).unwrap());
        assert_eq!(RECEIVED.load(Ordering::SeqCst), HandoverSignal::Restart.bit());

        assert!(recorder.stop());
    }

    #[test]
    fn every_recorded_signal_sets_its_own_bit() {
        static RECEIVED: AtomicU32 = AtomicU32::new(0);
        let recorders: Vec<_> = HandoverSignal::ALL
            .into_iter()
            .map(|signal| record_signal(signal, &RECEIVED).unwrap())
            .collect();

        for signal in HandoverSignal::ALL {
            raise_signal(signal).unwrap();
            assert_ne!(RECEIVED.load(Ordering::SeqCst) & signal.bit(), 0, "{}", signal.name());
        }
        assert_eq!(RECEIVED.load(Ordering::SeqCst), 0b111);

        for recorder in recorders {
            assert!(recorder.stop());
        }
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

    use std::net::TcpListener;
    use std::os::fd::{AsRawFd, IntoRawFd};

    use nix::unistd::dup;

    use infrarust_transport::handover::{claim_file, claim_sealed, read_sealed, sealed_memfd};

    use super::{close_on_exec, is_open};

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

    #[test]
    fn a_sealed_memfd_is_claimed_by_number() {
        let fd = sealed_memfd("infrarust-handover-claim", b"state").unwrap();
        let raw = fd.into_raw_fd();

        let claimed = claim_sealed(raw).unwrap();

        assert_eq!(claimed.as_raw_fd(), raw);
        assert_eq!(read_sealed(claimed.as_fd()).unwrap(), b"state");
    }

    #[test]
    fn only_a_sealed_memfd_is_claimed_as_one() {
        let unsealed = memfd_create("infrarust-handover-unsealed", MFdFlags::MFD_CLOEXEC).unwrap();
        let err = claim_sealed(unsealed.as_raw_fd()).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData, "{err}");
        assert!(is_open(&unsealed));

        let file = std::fs::File::open("/proc/self/exe").unwrap();
        assert!(claim_sealed(file.as_raw_fd()).is_err());
        assert!(is_open(&file));

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let err = claim_sealed(listener.as_raw_fd()).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput, "{err}");
        assert!(is_open(&listener));

        let err = claim_sealed(-1).unwrap_err();
        assert_eq!(err.raw_os_error(), Some(Errno::EBADF as i32), "{err}");
    }

    #[test]
    fn a_regular_file_is_claimed_by_number() {
        let file = std::fs::File::open("/proc/self/exe").unwrap();
        let raw = dup(&file).unwrap().into_raw_fd();

        let claimed = claim_file(raw).unwrap();

        assert_eq!(claimed.as_raw_fd(), raw);
        assert_eq!(
            claimed.metadata().unwrap().len(),
            file.metadata().unwrap().len()
        );
    }

    #[test]
    fn a_socket_is_not_claimed_as_a_file() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let err = claim_file(listener.as_raw_fd()).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput, "{err}");
        assert!(is_open(&listener));

        let err = claim_file(-1).unwrap_err();
        assert_eq!(err.raw_os_error(), Some(Errno::EBADF as i32), "{err}");
    }
}

#[cfg(target_os = "linux")]
use std::io::Write;
use std::net::SocketAddr;
#[cfg(unix)]
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
#[cfg(unix)]
use std::sync::atomic::{AtomicU32, Ordering};
#[cfg(unix)]
use std::{fmt, io};

#[cfg(unix)]
use nix::errno::Errno;
#[cfg(target_os = "linux")]
use nix::fcntl::SealFlag;
#[cfg(unix)]
use nix::fcntl::{FcntlArg, FdFlag, fcntl};
#[cfg(target_os = "linux")]
use nix::libc::off_t;
#[cfg(target_os = "linux")]
use nix::sys::memfd::{MFdFlags, memfd_create};
#[cfg(unix)]
use nix::sys::signal::{SigSet, Signal, raise};
#[cfg(unix)]
use nix::sys::socket::{
    SockType, SockaddrLike, SockaddrStorage, getpeername, getsockname, getsockopt, sockopt,
};
#[cfg(target_os = "linux")]
use nix::sys::stat::{SFlag, fstat};
#[cfg(target_os = "linux")]
use nix::sys::uio::pread;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SocketKind {
    Listener,
    Stream,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SocketIdentity {
    pub kind: SocketKind,
    pub local: SocketAddr,
    pub peer: Option<SocketAddr>,
}

#[derive(Debug)]
pub enum InheritedSocket {
    Listener(std::net::TcpListener),
    Stream(std::net::TcpStream),
}

#[cfg(unix)]
impl AsFd for InheritedSocket {
    fn as_fd(&self) -> BorrowedFd<'_> {
        match self {
            Self::Listener(listener) => listener.as_fd(),
            Self::Stream(stream) => stream.as_fd(),
        }
    }
}

#[cfg(unix)]
pub fn set_inheritable(fd: BorrowedFd<'_>, inheritable: bool) -> io::Result<()> {
    let mut flags = FdFlag::from_bits_retain(fcntl(fd, FcntlArg::F_GETFD)?);
    flags.set(FdFlag::FD_CLOEXEC, !inheritable);
    fcntl(fd, FcntlArg::F_SETFD(flags))?;
    Ok(())
}

#[cfg(unix)]
pub fn is_inheritable(fd: BorrowedFd<'_>) -> io::Result<bool> {
    let flags = FdFlag::from_bits_retain(fcntl(fd, FcntlArg::F_GETFD)?);
    Ok(!flags.contains(FdFlag::FD_CLOEXEC))
}

#[cfg(unix)]
#[allow(unsafe_code)]
pub fn close_on_exec_raw(fd: RawFd) -> io::Result<()> {
    if fd < 0 {
        return Err(Errno::EBADF.into());
    }
    let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
    set_inheritable(borrowed, false)
}

#[cfg(unix)]
pub fn socket_identity(fd: BorrowedFd<'_>) -> io::Result<SocketIdentity> {
    let (kind, local) = socket_kind(fd)?;
    let peer = match kind {
        SocketKind::Listener => None,
        SocketKind::Stream => {
            let raw = fd.as_raw_fd();
            Some(inet_address(raw, getpeername(raw)?)?)
        }
    };
    Ok(SocketIdentity { kind, local, peer })
}

#[cfg(unix)]
#[allow(unsafe_code)]
pub fn claim_inherited(fd: RawFd, expect: SocketKind) -> io::Result<InheritedSocket> {
    if fd < 0 {
        return Err(Errno::EBADF.into());
    }
    let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
    fcntl(borrowed, FcntlArg::F_GETFD)?;
    let (kind, _) = socket_kind(borrowed)?;
    if kind != expect {
        return Err(mismatch(format_args!(
            "fd {fd} is a {kind:?} socket, expected a {expect:?} socket"
        )));
    }
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    Ok(match expect {
        SocketKind::Listener => InheritedSocket::Listener(owned.into()),
        SocketKind::Stream => InheritedSocket::Stream(owned.into()),
    })
}

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
pub fn claim_sealed(fd: RawFd) -> io::Result<OwnedFd> {
    if fd < 0 {
        return Err(Errno::EBADF.into());
    }
    let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
    regular_file(borrowed)?;
    sealed(borrowed)?;
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
pub fn claim_file(fd: RawFd) -> io::Result<std::fs::File> {
    if fd < 0 {
        return Err(Errno::EBADF.into());
    }
    let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
    regular_file(borrowed)?;
    Ok(std::fs::File::from(unsafe { OwnedFd::from_raw_fd(fd) }))
}

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
pub fn remove_env_vars(keys: &[&str]) -> io::Result<()> {
    let threads = std::fs::read_dir("/proc/self/task")?.count();
    if threads != 1 {
        return Err(io::Error::other(format!(
            "the environment can only change while one thread runs, {threads} are running"
        )));
    }
    for key in keys {
        unsafe { std::env::remove_var(key) };
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn sealed_memfd(name: &str, bytes: &[u8]) -> io::Result<OwnedFd> {
    let fd = memfd_create(name, MFdFlags::MFD_CLOEXEC | MFdFlags::MFD_ALLOW_SEALING)?;
    let mut file = std::fs::File::from(fd);
    file.write_all(bytes)?;
    fcntl(
        &file,
        FcntlArg::F_ADD_SEALS(IMMUTABLE | SealFlag::F_SEAL_SEAL),
    )?;
    Ok(OwnedFd::from(file))
}

#[cfg(target_os = "linux")]
pub fn read_sealed(fd: BorrowedFd<'_>) -> io::Result<Vec<u8>> {
    sealed(fd)?;
    let size = usize::try_from(fstat(fd)?.st_size).map_err(io::Error::other)?;
    let mut bytes = vec![0; size];
    let mut filled = 0;
    while filled < size {
        let offset = off_t::try_from(filled).map_err(io::Error::other)?;
        match pread(fd, &mut bytes[filled..], offset) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(read) => filled += read,
            Err(Errno::EINTR) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(bytes)
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HandoverSignal {
    Terminate,
    Interrupt,
    Restart,
}

#[cfg(unix)]
impl HandoverSignal {
    pub const ALL: [Self; 3] = [Self::Terminate, Self::Interrupt, Self::Restart];

    pub const fn bit(self) -> u32 {
        match self {
            Self::Terminate => 1,
            Self::Interrupt => 1 << 1,
            Self::Restart => 1 << 2,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Terminate => "SIGTERM",
            Self::Interrupt => "SIGINT",
            Self::Restart => "SIGUSR2",
        }
    }

    const fn signal(self) -> Signal {
        match self {
            Self::Terminate => Signal::SIGTERM,
            Self::Interrupt => Signal::SIGINT,
            Self::Restart => Signal::SIGUSR2,
        }
    }
}

#[cfg(unix)]
#[derive(Debug)]
pub struct SignalRecorder(signal_hook_registry::SigId);

#[cfg(unix)]
impl SignalRecorder {
    pub fn stop(self) -> bool {
        signal_hook_registry::unregister(self.0)
    }
}

#[cfg(unix)]
#[allow(unsafe_code)]
pub fn record_signal(
    signal: HandoverSignal,
    into: &'static AtomicU32,
) -> io::Result<SignalRecorder> {
    let bit = signal.bit();
    let id = unsafe {
        signal_hook_registry::register(signal.signal() as nix::libc::c_int, move || {
            into.fetch_or(bit, Ordering::SeqCst);
        })
    }?;
    Ok(SignalRecorder(id))
}

#[cfg(unix)]
pub fn block_signals(signals: &[HandoverSignal]) -> io::Result<()> {
    signal_set(signals).thread_block()?;
    Ok(())
}

#[cfg(unix)]
pub fn unblock_signals(signals: &[HandoverSignal]) -> io::Result<()> {
    signal_set(signals).thread_unblock()?;
    Ok(())
}

#[cfg(unix)]
pub fn is_blocked(signal: HandoverSignal) -> io::Result<bool> {
    Ok(SigSet::thread_get_mask()?.contains(signal.signal()))
}

#[cfg(unix)]
pub fn raise_signal(signal: HandoverSignal) -> io::Result<()> {
    raise(signal.signal())?;
    Ok(())
}

#[cfg(unix)]
fn signal_set(signals: &[HandoverSignal]) -> SigSet {
    let mut set = SigSet::empty();
    for signal in signals {
        set.add(signal.signal());
    }
    set
}

#[cfg(target_os = "linux")]
const IMMUTABLE: SealFlag = SealFlag::F_SEAL_SHRINK
    .union(SealFlag::F_SEAL_GROW)
    .union(SealFlag::F_SEAL_WRITE);

#[cfg(target_os = "linux")]
fn sealed(fd: BorrowedFd<'_>) -> io::Result<()> {
    let seals = SealFlag::from_bits_retain(fcntl(fd, FcntlArg::F_GET_SEALS)?);
    if !seals.contains(IMMUTABLE) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("fd {} is not sealed against writes", fd.as_raw_fd()),
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn regular_file(fd: BorrowedFd<'_>) -> io::Result<()> {
    let mode = SFlag::from_bits_truncate(fstat(fd)?.st_mode);
    if mode & SFlag::S_IFMT != SFlag::S_IFREG {
        return Err(mismatch(format_args!(
            "fd {} is not a regular file",
            fd.as_raw_fd()
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn socket_kind(fd: BorrowedFd<'_>) -> io::Result<(SocketKind, SocketAddr)> {
    let raw = fd.as_raw_fd();
    let sock_type = getsockopt(&fd, sockopt::SockType)?;
    if sock_type != SockType::Stream {
        return Err(mismatch(format_args!(
            "fd {raw} is a {sock_type:?} socket, not a stream socket"
        )));
    }
    let local = inet_address(raw, getsockname(raw)?)?;
    let kind = if getsockopt(&fd, sockopt::AcceptConn)? {
        SocketKind::Listener
    } else {
        SocketKind::Stream
    };
    Ok((kind, local))
}

#[cfg(unix)]
fn inet_address(fd: RawFd, address: SockaddrStorage) -> io::Result<SocketAddr> {
    if let Some(v4) = address.as_sockaddr_in() {
        Ok(SocketAddr::from(*v4))
    } else if let Some(v6) = address.as_sockaddr_in6() {
        Ok(SocketAddr::from(*v6))
    } else {
        Err(mismatch(format_args!(
            "fd {fd} is a {:?} socket, not an inet socket",
            address.family()
        )))
    }
}

#[cfg(unix)]
fn mismatch(message: fmt::Arguments<'_>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.to_string())
}

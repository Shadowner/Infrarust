#[cfg(target_os = "linux")]
use std::io::Write;
use std::net::SocketAddr;
#[cfg(unix)]
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
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
use nix::sys::socket::{
    SockType, SockaddrLike, SockaddrStorage, getpeername, getsockname, getsockopt, sockopt,
};
#[cfg(target_os = "linux")]
use nix::sys::stat::fstat;
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
pub fn socket_identity(fd: BorrowedFd<'_>) -> io::Result<SocketIdentity> {
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
    let peer = match kind {
        SocketKind::Listener => None,
        SocketKind::Stream => Some(inet_address(raw, getpeername(raw)?)?),
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
    let identity = socket_identity(borrowed)?;
    if identity.kind != expect {
        return Err(mismatch(format_args!(
            "fd {fd} is a {:?} socket, expected a {expect:?} socket",
            identity.kind
        )));
    }
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    Ok(match expect {
        SocketKind::Listener => InheritedSocket::Listener(owned.into()),
        SocketKind::Stream => InheritedSocket::Stream(owned.into()),
    })
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
    let seals = SealFlag::from_bits_retain(fcntl(fd, FcntlArg::F_GET_SEALS)?);
    if !seals.contains(IMMUTABLE) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("fd {} is not sealed against writes", fd.as_raw_fd()),
        ));
    }
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

#[cfg(target_os = "linux")]
const IMMUTABLE: SealFlag = SealFlag::F_SEAL_SHRINK
    .union(SealFlag::F_SEAL_GROW)
    .union(SealFlag::F_SEAL_WRITE);

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

mod envelope;
mod launch;
mod signals;
#[cfg(test)]
mod tests;

use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::net::SocketAddr;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, Once, PoisonError, TryLockError};

use infrarust_transport::handover::{
    InheritedSocket, SocketKind, claim_file, claim_inherited, claim_sealed, close_on_exec_raw,
    read_sealed, remove_env_vars, sealed_memfd, set_inheritable, socket_identity,
};

pub use launch::{ATTEMPT_VAR, FAILPOINT_VAR, HANDOVER_FD_VAR};
pub use signals::{EarlySignals, HandoverSignal, defer_stop, early_signals};

use self::envelope::{Envelope, EnvelopeHeader};
use self::launch::{
    Attempt, Exec, Failpoint, FileId, HandoverVars, INHERITED_VARS, RollbackPlan, candidate_paths,
    exec_image, launch_each, next_env, proc_fd_path, rollback_candidates,
};
use super::{Handed, Inheritance, RestartPlan, Successor};
use crate::hot_restart::HandoverError;
use crate::hot_restart::package::{Handle, HandleTable, HandoverPackage};
use crate::hot_restart::state::{HandleId, HandoverState};

const MEMFD_NAME: &str = "infrarust-handover";

static ARMED: Mutex<Option<RollbackPlan>> = Mutex::new(None);
static PANIC_HOOK: Once = Once::new();

pub fn successor() -> Result<Box<dyn Successor>, HandoverError> {
    signals::record_stops();
    Ok(Box::new(LinuxSuccessor { exec: exec_image }))
}

pub fn inherit() -> Result<Option<Box<dyn Inheritance>>, HandoverError> {
    let Some(fd) = std::env::var_os(HANDOVER_FD_VAR) else {
        return Ok(None);
    };
    signals::install_early()?;
    let vars = HandoverVars::parse(
        &fd,
        std::env::var_os(ATTEMPT_VAR).as_deref(),
        std::env::var_os(FAILPOINT_VAR).as_deref(),
    )?;
    let mut inheritance = inherit_from(vars, std::env::args_os().collect(), exec_image)?;
    if let Err(error) = remove_env_vars(&INHERITED_VARS) {
        return Err(Box::new(inheritance).roll_back(&format!(
            "the handover variables could not be cleared: {error}"
        )));
    }
    inheritance.arm();
    Ok(Some(Box::new(inheritance)))
}

#[derive(Debug)]
struct LinuxSuccessor {
    exec: Exec,
}

impl Successor for LinuxSuccessor {
    fn platform(&self) -> &'static str {
        "linux"
    }

    fn hand_over(
        self: Box<Self>,
        package: HandoverPackage,
        plan: &RestartPlan,
    ) -> Result<Handed, HandoverError> {
        hand_over(package, plan, self.exec, Failpoint::from_env())
    }
}

fn hand_over(
    package: HandoverPackage,
    plan: &RestartPlan,
    exec: Exec,
    failpoint: Option<Failpoint>,
) -> Result<Handed, HandoverError> {
    let HandoverPackage {
        state,
        tails,
        mut handles,
    } = package;
    let fds = owned_fds(&mut handles)?;
    let image = plan.binary.exe.image();
    let header = EnvelopeHeader {
        handles: fds.iter().map(AsRawFd::as_raw_fd).collect(),
        image: image.map(AsRawFd::as_raw_fd),
        exe: plan.binary.exe.path().to_path_buf(),
        version: plan.binary.build.version.to_string(),
    };
    let memfd = sealed_memfd(MEMFD_NAME, &envelope::encode(&header, &state, &tails)?)?;
    let inherited: Vec<BorrowedFd<'_>> = fds
        .iter()
        .map(AsFd::as_fd)
        .chain(image.map(AsFd::as_fd))
        .chain(std::iter::once(memfd.as_fd()))
        .collect();
    if let Err(error) = inherited
        .iter()
        .try_for_each(|fd| set_inheritable(*fd, true))
    {
        close_on_exec(&inherited);
        return Err(error.into());
    }
    let failures = start_successor(plan, &header, memfd.as_raw_fd(), exec, failpoint);
    close_on_exec(&inherited);
    Err(HandoverError::Io(io::Error::other(format!(
        "no image took the hot restart over: {}",
        failures.join("; ")
    ))))
}

fn start_successor(
    plan: &RestartPlan,
    header: &EnvelopeHeader,
    memfd: RawFd,
    exec: Exec,
    failpoint: Option<Failpoint>,
) -> Vec<String> {
    if let Err(error) = signals::hold() {
        eprintln!("infrarust: could not block the stop signals before the hot restart: {error}");
    }
    let argv0 = std::env::args_os()
        .next()
        .unwrap_or_else(|| plan.target.clone().into_os_string());
    let current: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    let mut failures = Vec::new();
    if failpoint == Some(Failpoint::ExecTarget) {
        eprintln!(
            "infrarust: the exec-target failpoint keeps {} from starting",
            plan.target.display()
        );
        failures.push(format!(
            "{}: the exec-target failpoint is set",
            plan.target.display()
        ));
    } else {
        let env = next_env(current.iter().cloned(), memfd, Attempt::First);
        failures.extend(launch_each(
            exec,
            std::slice::from_ref(&plan.target),
            &argv0,
            &plan.binary.args,
            &env,
        ));
    }
    let env = next_env(current, memfd, Attempt::Second);
    failures.extend(launch_each(
        exec,
        &running_images(header),
        &argv0,
        &plan.binary.args,
        &env,
    ));
    signals::release();
    failures
}

fn running_images(header: &EnvelopeHeader) -> Vec<PathBuf> {
    header
        .image
        .map(proc_fd_path)
        .into_iter()
        .chain(std::iter::once(header.exe.clone()))
        .collect()
}

fn owned_fds(handles: &mut HandleTable) -> Result<Vec<OwnedFd>, HandoverError> {
    (0..handles.len())
        .map(|index| {
            let id = u32::try_from(index)
                .map_err(|_| HandoverError::Malformed("too many handles".into()))?;
            Ok(match handles.take(HandleId(id))? {
                Handle::Listener(listener) => OwnedFd::from(listener),
                Handle::Stream(stream) => OwnedFd::from(stream),
            })
        })
        .collect()
}

fn close_on_exec(fds: &[BorrowedFd<'_>]) {
    for fd in fds {
        if let Err(error) = set_inheritable(*fd, false) {
            eprintln!(
                "infrarust: fd {} stays open across exec: {error}",
                fd.as_raw_fd()
            );
        }
    }
}

#[derive(Debug)]
struct LinuxInheritance {
    wire: Option<(HandoverState, Vec<u8>)>,
    handles: Vec<RawFd>,
    claimed: Vec<InheritedSocket>,
    memfd: Option<OwnedFd>,
    image: Option<File>,
    rollback: RollbackPlan,
    failpoint: Option<Failpoint>,
    armed: bool,
    settled: bool,
}

impl LinuxInheritance {
    fn arm(&mut self) {
        signals::record_stops();
        PANIC_HOOK.call_once(|| {
            let previous = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                if let Some(plan) = try_disarm()
                    && plan.attempt == Attempt::First
                {
                    let error = plan.execute(&format!("the new image panicked: {info}"));
                    eprintln!("infrarust: {error}");
                }
                previous(info);
            }));
        });
        *ARMED.lock().unwrap_or_else(PoisonError::into_inner) = Some(self.rollback.clone());
        self.armed = true;
    }

    fn settle(&mut self) {
        self.settled = true;
        if self.armed {
            disarm();
        }
    }
}

impl Inheritance for LinuxInheritance {
    fn take_package(&mut self) -> Result<HandoverPackage, HandoverError> {
        let (state, tails) = self.wire.take().ok_or_else(|| {
            HandoverError::Malformed("the inherited package was already taken".into())
        })?;
        let expected = expectations(&state, self.handles.len())?;
        for (index, (fd, expect)) in self.handles.iter().zip(expected).enumerate() {
            let id = u32::try_from(index)
                .map_err(|_| HandoverError::Malformed("too many handles".into()))?;
            let socket = claim_inherited(*fd, expect.kind())
                .map_err(|error| HandoverError::Mismatch(id, error.to_string()))?;
            let checked = expect.check(&socket);
            self.claimed.push(socket);
            checked.map_err(|reason| HandoverError::Mismatch(id, reason))?;
        }
        let handles = self
            .claimed
            .drain(..)
            .map(|socket| match socket {
                InheritedSocket::Listener(listener) => Handle::Listener(listener),
                InheritedSocket::Stream(stream) => Handle::Stream(stream),
            })
            .collect();
        Ok(HandoverPackage {
            state,
            tails,
            handles: HandleTable::from_handles(handles),
        })
    }

    fn commit(mut self: Box<Self>) -> Result<(), HandoverError> {
        if cfg!(debug_assertions)
            && self.failpoint == Some(Failpoint::SuccessorPrecommit)
            && self.rollback.attempt == Attempt::First
        {
            return Err(self.roll_back("the successor-precommit failpoint is set"));
        }
        for fd in &self.handles {
            if let Err(error) = close_on_exec_raw(*fd) {
                tracing::warn!(fd, %error, "an inherited descriptor stays open across exec");
            }
        }
        self.memfd = None;
        self.image = None;
        self.settle();
        Ok(())
    }

    fn roll_back(mut self: Box<Self>, reason: &str) -> HandoverError {
        self.settle();
        self.rollback.execute(reason)
    }
}

impl Drop for LinuxInheritance {
    fn drop(&mut self) {
        if self.settled || !self.armed {
            return;
        }
        self.settled = true;
        if let Some(plan) = disarm() {
            let error = plan.execute("the inherited sessions were dropped before the commit");
            eprintln!("infrarust: {error}");
        }
    }
}

fn disarm() -> Option<RollbackPlan> {
    ARMED.lock().unwrap_or_else(PoisonError::into_inner).take()
}

fn try_disarm() -> Option<RollbackPlan> {
    match ARMED.try_lock() {
        Ok(mut plan) => plan.take(),
        Err(TryLockError::Poisoned(plan)) => plan.into_inner().take(),
        Err(TryLockError::WouldBlock) => None,
    }
}

fn inherit_from(
    vars: HandoverVars,
    argv: Vec<OsString>,
    exec: Exec,
) -> Result<LinuxInheritance, HandoverError> {
    let memfd = claim_sealed(vars.fd)?;
    let Envelope {
        header,
        state,
        tails,
    } = envelope::decode(&read_sealed(memfd.as_fd())?)?;
    if header.handles.contains(&vars.fd) || header.image == Some(vars.fd) {
        return Err(HandoverError::Malformed(format!(
            "fd {} is both the envelope and one of the descriptors it lists",
            vars.fd
        )));
    }
    let image = header.image.and_then(|fd| match claim_file(fd) {
        Ok(file) => Some(file),
        Err(error) => {
            eprintln!(
                "infrarust: the previous image (fd {fd}) cannot serve as a rollback: {error}"
            );
            None
        }
    });
    let old = image
        .as_ref()
        .and_then(|file| file.metadata().ok())
        .map(|meta| FileId::of(&meta));
    let candidates = rollback_candidates(
        candidate_paths(image.as_ref().map(AsRawFd::as_raw_fd), &header.exe),
        FileId::of_path(Path::new("/proc/self/exe")),
        old,
        FileId::of_path,
    );
    Ok(LinuxInheritance {
        wire: Some((state, tails)),
        handles: header.handles,
        claimed: Vec::new(),
        memfd: Some(memfd),
        image,
        rollback: RollbackPlan {
            version: header.version,
            memfd: vars.fd,
            attempt: vars.attempt,
            argv,
            candidates,
            exec,
        },
        failpoint: vars.failpoint,
        armed: false,
        settled: false,
    })
}

#[derive(Debug, Clone, Copy)]
enum Expected {
    Listener { local: SocketAddr },
    Stream { local: SocketAddr, peer: SocketAddr },
}

impl Expected {
    const fn kind(self) -> SocketKind {
        match self {
            Self::Listener { .. } => SocketKind::Listener,
            Self::Stream { .. } => SocketKind::Stream,
        }
    }

    fn check(self, socket: &InheritedSocket) -> Result<(), String> {
        match (self, socket_identity(socket.as_fd())) {
            (Self::Listener { local }, Ok(identity)) if identity.local == local => Ok(()),
            (Self::Listener { local }, Ok(identity)) => Err(format!(
                "listener bound to {}, expected {local}",
                identity.local
            )),
            (Self::Stream { local, peer }, Ok(identity))
                if identity.local == local && identity.peer == Some(peer) =>
            {
                Ok(())
            }
            (Self::Stream { local, peer }, Ok(identity)) => Err(format!(
                "socket is {} -> {:?}, expected {local} -> {peer}",
                identity.local, identity.peer
            )),
            (Self::Stream { .. }, Err(error)) if error.kind() == io::ErrorKind::NotConnected => {
                Ok(())
            }
            (_, Err(error)) => Err(error.to_string()),
        }
    }
}

fn expectations(state: &HandoverState, count: usize) -> Result<Vec<Expected>, HandoverError> {
    let mut slots: Vec<Option<Expected>> = vec![None; count];
    if let Some(listener) = &state.listener {
        expect(
            &mut slots,
            listener.handle,
            Expected::Listener {
                local: listener.local_addr,
            },
        )?;
    }
    for session in &state.sessions {
        for socket in [session.client, session.backend] {
            expect(
                &mut slots,
                socket.handle,
                Expected::Stream {
                    local: socket.local,
                    peer: socket.peer,
                },
            )?;
        }
    }
    slots
        .into_iter()
        .zip(0u32..)
        .map(|(slot, id)| slot.ok_or(HandoverError::MissingHandle(id)))
        .collect()
}

fn expect(
    slots: &mut [Option<Expected>],
    handle: HandleId,
    expected: Expected,
) -> Result<(), HandoverError> {
    let slot = slots
        .get_mut(handle.0 as usize)
        .ok_or(HandoverError::MissingHandle(handle.0))?;
    *slot = Some(expected);
    Ok(())
}

use std::process::ExitCode;
use std::sync::Arc;

use infrarust_core::binary::{HandoverSupport, RunningBinary};
use infrarust_core::hot_restart::platform;
use infrarust_core::hot_restart::{Handed, Inheritance, Successor};
use infrarust_core::runtime::RestartHandover;

use crate::signals::StopSignal;

pub(crate) fn refuse_command_line(error: clap::Error) -> ExitCode {
    match platform::inherit() {
        Ok(None) => error.exit(),
        Ok(Some(inheritance)) => {
            eprint!("{error}");
            give_up(
                Some(inheritance),
                "the command line of the new image is invalid",
            )
        }
        Err(inherit) => {
            eprintln!("error: cannot take over from the previous image: {inherit}");
            error.exit()
        }
    }
}

pub(crate) fn inherit() -> Result<Option<Box<dyn Inheritance>>, ExitCode> {
    platform::inherit().map_err(|error| {
        eprintln!("error: cannot take over from the previous image: {error}");
        ExitCode::FAILURE
    })
}

pub(crate) fn give_up(inheritance: Option<Box<dyn Inheritance>>, reason: &str) -> ExitCode {
    eprintln!("error: {reason}");
    if let Some(inheritance) = inheritance {
        let error = inheritance.roll_back(reason);
        eprintln!("error: {error}");
    }
    ExitCode::FAILURE
}

pub(crate) fn abandon(
    inheritance: Option<Box<dyn Inheritance>>,
    error: anyhow::Error,
) -> anyhow::Error {
    match inheritance {
        Some(inheritance) => {
            let returned = inheritance.roll_back(&format!("{error:#}"));
            error.context(returned.to_string())
        }
        None => error,
    }
}

pub(crate) fn successor(binary: Option<&Arc<RunningBinary>>) -> Option<Box<dyn Successor>> {
    HandoverSupport::current()?;
    binary?;
    match platform::successor() {
        Ok(successor) => Some(successor),
        Err(error) => {
            tracing::warn!(%error, "hot restart is unavailable");
            None
        }
    }
}

pub(crate) fn hand_over(
    successor: Option<Box<dyn Successor>>,
    handover: RestartHandover,
) -> ExitCode {
    let RestartHandover { package, plan } = handover;
    let sessions = package.state.sessions.len();
    let successor = match successor.map_or_else(platform::successor, Ok) {
        Ok(successor) => successor,
        Err(error) => {
            eprintln!("error: the hot restart cannot start, {sessions} sessions are lost: {error}");
            return ExitCode::FAILURE;
        }
    };
    match successor.hand_over(package, &plan) {
        Ok(Handed) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: the hot restart failed, {sessions} sessions are lost: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn record_early_restarts() {
    if let Err(error) = platform::linux::record_early_restarts() {
        eprintln!("warning: a SIGUSR2 sent before the proxy is ready may stop it: {error}");
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) const fn record_early_restarts() {}

#[cfg(target_os = "linux")]
pub(crate) fn defer_stop(stop: StopSignal) {
    use platform::linux::HandoverSignal;

    platform::linux::defer_stop(match stop {
        StopSignal::Interrupt => HandoverSignal::Interrupt,
        StopSignal::Terminate => HandoverSignal::Terminate,
    });
}

#[cfg(not(target_os = "linux"))]
pub(crate) const fn defer_stop(_stop: StopSignal) {}

use std::ffi::{OsStr, OsString};
use std::io;
use std::os::fd::RawFd;
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::signals;
use crate::hot_restart::HandoverError;
use crate::update::{BinaryPaths, Platform};

pub const HANDOVER_FD_VAR: &str = "INFRARUST_HANDOVER_FD";
pub const ATTEMPT_VAR: &str = "INFRARUST_HANDOVER_ATTEMPT";
pub const FAILPOINT_VAR: &str = "INFRARUST_HANDOVER_FAILPOINT";
pub const INHERITED_VARS: [&str; 3] = [HANDOVER_FD_VAR, ATTEMPT_VAR, FAILPOINT_VAR];
const DROPPED_VARS: [&str; 5] = [
    "LISTEN_PID",
    "LISTEN_FDS",
    "LISTEN_FDNAMES",
    HANDOVER_FD_VAR,
    ATTEMPT_VAR,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attempt {
    First,
    Second,
}

impl Attempt {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::First => "1",
            Self::Second => "2",
        }
    }

    fn parse(value: &OsStr) -> Result<Self, HandoverError> {
        match value.to_str() {
            Some("1") => Ok(Self::First),
            Some("2") => Ok(Self::Second),
            _ => Err(HandoverError::Malformed(format!(
                "{ATTEMPT_VAR}={} is neither 1 nor 2",
                value.to_string_lossy()
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failpoint {
    ExecTarget,
    SuccessorPrecommit,
}

impl Failpoint {
    pub fn from_env() -> Option<Self> {
        Self::parse(std::env::var_os(FAILPOINT_VAR).as_deref())
    }

    fn parse(value: Option<&OsStr>) -> Option<Self> {
        if !cfg!(debug_assertions) {
            return None;
        }
        match value?.to_str()? {
            "exec-target" => Some(Self::ExecTarget),
            "successor-precommit" => Some(Self::SuccessorPrecommit),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandoverVars {
    pub fd: RawFd,
    pub attempt: Attempt,
    pub failpoint: Option<Failpoint>,
}

impl HandoverVars {
    pub fn parse(
        fd: &OsStr,
        attempt: Option<&OsStr>,
        failpoint: Option<&OsStr>,
    ) -> Result<Self, HandoverError> {
        let fd = fd
            .to_str()
            .and_then(|text| text.parse::<RawFd>().ok())
            .filter(|fd| *fd >= 0)
            .ok_or_else(|| {
                HandoverError::Malformed(format!(
                    "{HANDOVER_FD_VAR}={} is not a descriptor number",
                    fd.to_string_lossy()
                ))
            })?;
        let attempt = attempt
            .ok_or_else(|| {
                HandoverError::Malformed(format!("{HANDOVER_FD_VAR} is set without {ATTEMPT_VAR}"))
            })
            .and_then(Attempt::parse)?;
        Ok(Self {
            fd,
            attempt,
            failpoint: Failpoint::parse(failpoint),
        })
    }
}

pub fn next_env(
    current: impl IntoIterator<Item = (OsString, OsString)>,
    memfd: RawFd,
    attempt: Attempt,
) -> Vec<(OsString, OsString)> {
    let mut env: Vec<(OsString, OsString)> = current
        .into_iter()
        .filter(|(key, _)| !DROPPED_VARS.iter().any(|dropped| key == dropped))
        .collect();
    env.push((HANDOVER_FD_VAR.into(), memfd.to_string().into()));
    env.push((ATTEMPT_VAR.into(), attempt.as_str().into()));
    env
}

#[derive(Debug, Clone, Copy)]
pub struct Launch<'a> {
    pub program: &'a Path,
    pub argv0: &'a OsStr,
    pub args: &'a [OsString],
    pub env: &'a [(OsString, OsString)],
}

pub type Exec = fn(&Launch<'_>) -> io::Error;

pub fn exec_image(launch: &Launch<'_>) -> io::Error {
    Command::new(launch.program)
        .arg0(launch.argv0)
        .args(launch.args)
        .env_clear()
        .envs(launch.env.iter().map(|(key, value)| (key, value)))
        .exec()
}

pub fn launch_each(
    exec: Exec,
    programs: &[PathBuf],
    argv0: &OsStr,
    args: &[OsString],
    env: &[(OsString, OsString)],
) -> Vec<String> {
    let mut failures = Vec::with_capacity(programs.len());
    for program in programs {
        signals::forward_pending_stop();
        let error = exec(&Launch {
            program,
            argv0,
            args,
            env,
        });
        eprintln!(
            "infrarust: could not start {} for the hot restart: {error}",
            program.display()
        );
        failures.push(format!("{}: {error}", program.display()));
    }
    failures
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileId {
    dev: u64,
    ino: u64,
}

impl FileId {
    pub fn of_path(path: &Path) -> Option<Self> {
        std::fs::metadata(path).ok().map(|meta| Self::of(&meta))
    }

    pub fn of(meta: &std::fs::Metadata) -> Self {
        Self {
            dev: meta.dev(),
            ino: meta.ino(),
        }
    }
}

pub fn proc_fd_path(fd: RawFd) -> PathBuf {
    PathBuf::from(format!("/proc/self/fd/{fd}"))
}

pub fn candidate_paths(image: Option<RawFd>, exe: &Path) -> Vec<PathBuf> {
    image
        .map(proc_fd_path)
        .into_iter()
        .chain(BinaryPaths::for_exe_on(exe, Platform::Linux).map(|paths| paths.old))
        .chain(std::iter::once(exe.to_path_buf()))
        .collect()
}

pub fn rollback_candidates(
    paths: Vec<PathBuf>,
    current: Option<FileId>,
    old: Option<FileId>,
    identify: impl Fn(&Path) -> Option<FileId>,
) -> Vec<PathBuf> {
    let mut seen = Vec::with_capacity(paths.len());
    paths
        .into_iter()
        .filter(|path| {
            let Some(id) = identify(path) else {
                return false;
            };
            if seen.contains(&id) || (Some(id) == current && Some(id) != old) {
                return false;
            }
            seen.push(id);
            true
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct RollbackPlan {
    pub version: String,
    pub memfd: RawFd,
    pub attempt: Attempt,
    pub argv: Vec<OsString>,
    pub candidates: Vec<PathBuf>,
    pub exec: Exec,
}

impl RollbackPlan {
    pub fn execute(&self, reason: &str) -> HandoverError {
        if self.attempt == Attempt::Second {
            return HandoverError::RolledBack(reason.to_string());
        }
        eprintln!(
            "infrarust: rolling the hot restart back to {}: {reason}",
            self.version
        );
        if self.candidates.is_empty() {
            return HandoverError::Io(io::Error::other(format!(
                "no image is left to roll the hot restart back to ({reason})"
            )));
        }
        if let Err(error) = signals::hold() {
            eprintln!("infrarust: could not block the stop signals before the rollback: {error}");
        }
        let env = next_env(std::env::vars_os(), self.memfd, Attempt::Second);
        let (argv0, args) = match self.argv.split_first() {
            Some((argv0, args)) => (argv0.as_os_str(), args),
            None => (OsStr::new("infrarust"), &[][..]),
        };
        let failures = launch_each(self.exec, &self.candidates, argv0, args, &env);
        signals::release();
        HandoverError::Io(io::Error::other(format!(
            "could not roll the hot restart back ({reason}): {}",
            failures.join("; ")
        )))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::collections::HashMap;

    use super::*;

    fn os(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        pairs
            .iter()
            .map(|(key, value)| (OsString::from(key), OsString::from(value)))
            .collect()
    }

    #[test]
    fn the_next_image_loses_socket_activation_and_gets_the_handover() {
        let current = os(&[
            ("PATH", "/usr/bin"),
            ("LISTEN_PID", "42"),
            ("LISTEN_FDS", "1"),
            ("LISTEN_FDNAMES", "game"),
            (HANDOVER_FD_VAR, "9"),
            (ATTEMPT_VAR, "1"),
            (FAILPOINT_VAR, "successor-precommit"),
            ("RUST_LOG", "info"),
        ]);

        let env = next_env(current, 17, Attempt::Second);

        assert_eq!(
            env,
            os(&[
                ("PATH", "/usr/bin"),
                (FAILPOINT_VAR, "successor-precommit"),
                ("RUST_LOG", "info"),
                (HANDOVER_FD_VAR, "17"),
                (ATTEMPT_VAR, "2"),
            ])
        );
    }

    #[test]
    fn the_handover_variables_parse() {
        let vars = HandoverVars::parse(
            OsStr::new("12"),
            Some(OsStr::new("2")),
            Some(OsStr::new("exec-target")),
        )
        .unwrap();
        assert_eq!(vars.fd, 12);
        assert_eq!(vars.attempt, Attempt::Second);
        assert_eq!(
            vars.failpoint,
            cfg!(debug_assertions).then_some(Failpoint::ExecTarget)
        );

        let vars = HandoverVars::parse(OsStr::new("3"), Some(OsStr::new("1")), None).unwrap();
        assert_eq!(vars.attempt, Attempt::First);
        assert_eq!(vars.failpoint, None);
    }

    #[test]
    fn malformed_handover_variables_are_rejected() {
        for (fd, attempt) in [
            ("", Some("1")),
            ("-1", Some("1")),
            ("x", Some("1")),
            ("4", None),
            ("4", Some("3")),
            ("4", Some("")),
        ] {
            assert!(
                matches!(
                    HandoverVars::parse(OsStr::new(fd), attempt.map(OsStr::new), None),
                    Err(HandoverError::Malformed(_))
                ),
                "{fd:?} {attempt:?}"
            );
        }
    }

    #[test]
    fn an_unknown_failpoint_is_ignored() {
        assert_eq!(Failpoint::parse(Some(OsStr::new("nope"))), None);
        assert_eq!(Failpoint::parse(None), None);
        assert_eq!(
            Failpoint::parse(Some(OsStr::new("successor-precommit"))),
            cfg!(debug_assertions).then_some(Failpoint::SuccessorPrecommit)
        );
    }

    #[test]
    fn candidates_come_from_the_image_fd_the_old_binary_and_the_original_path() {
        assert_eq!(
            candidate_paths(Some(7), Path::new("/opt/infrarust/infrarust")),
            vec![
                PathBuf::from("/proc/self/fd/7"),
                PathBuf::from("/opt/infrarust/infrarust.old"),
                PathBuf::from("/opt/infrarust/infrarust"),
            ]
        );
        assert_eq!(
            candidate_paths(None, Path::new("/srv/mc-proxy")),
            vec![
                PathBuf::from("/srv/mc-proxy.old"),
                PathBuf::from("/srv/mc-proxy")
            ]
        );
    }

    fn ids(files: &[(&str, u64)]) -> impl Fn(&Path) -> Option<FileId> + use<> {
        let files: HashMap<PathBuf, FileId> = files
            .iter()
            .map(|(path, ino)| (PathBuf::from(path), FileId { dev: 1, ino: *ino }))
            .collect();
        move |path| files.get(path).copied()
    }

    fn id(ino: u64) -> Option<FileId> {
        Some(FileId { dev: 1, ino })
    }

    fn paths(list: &[&str]) -> Vec<PathBuf> {
        list.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn after_an_update_only_the_old_image_is_a_candidate() {
        let identify = ids(&[
            ("/proc/self/fd/7", 1),
            ("/opt/infrarust.old", 1),
            ("/opt/infrarust", 2),
        ]);
        let all = paths(&["/proc/self/fd/7", "/opt/infrarust.old", "/opt/infrarust"]);

        assert_eq!(
            rollback_candidates(all, id(2), id(1), identify),
            paths(&["/proc/self/fd/7"])
        );
    }

    #[test]
    fn without_the_image_fd_the_old_binary_is_found_on_disk() {
        let identify = ids(&[("/opt/infrarust.old", 1), ("/opt/infrarust", 2)]);
        let all = paths(&["/proc/self/fd/7", "/opt/infrarust.old", "/opt/infrarust"]);

        assert_eq!(
            rollback_candidates(all, id(2), None, identify),
            paths(&["/opt/infrarust.old"])
        );
    }

    #[test]
    fn a_plain_restart_rolls_back_to_the_same_image() {
        let identify = ids(&[("/proc/self/fd/7", 1), ("/opt/infrarust", 1)]);
        let all = paths(&["/proc/self/fd/7", "/opt/infrarust.old", "/opt/infrarust"]);

        assert_eq!(
            rollback_candidates(all, id(1), id(1), identify),
            paths(&["/proc/self/fd/7"])
        );
    }

    #[test]
    fn the_current_image_is_skipped_when_the_old_one_is_unknown() {
        let identify = ids(&[("/opt/infrarust", 2)]);
        let all = paths(&["/opt/infrarust.old", "/opt/infrarust"]);

        assert!(rollback_candidates(all, id(2), None, identify).is_empty());
    }

    #[test]
    fn every_distinct_existing_image_is_kept_in_order() {
        let identify = ids(&[
            ("/proc/self/fd/7", 1),
            ("/opt/infrarust.old", 3),
            ("/opt/infrarust", 4),
        ]);
        let all = paths(&["/proc/self/fd/7", "/opt/infrarust.old", "/opt/infrarust"]);

        assert_eq!(
            rollback_candidates(all.clone(), id(2), id(1), identify),
            all
        );
    }

    #[test]
    fn a_rollback_at_the_second_attempt_never_runs_anything() {
        fn refuse(_: &Launch<'_>) -> io::Error {
            panic!("the second attempt must not exec")
        }
        let plan = RollbackPlan {
            version: "2.0.0-beta.5".into(),
            memfd: 5,
            attempt: Attempt::Second,
            argv: vec!["infrarust".into()],
            candidates: paths(&["/proc/self/fd/7"]),
            exec: refuse,
        };

        let error = plan.execute("the config is invalid");

        assert!(
            matches!(error, HandoverError::RolledBack(reason) if reason == "the config is invalid")
        );
    }
}

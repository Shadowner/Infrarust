use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use infrarust_api::services::RestartRequirement;

use crate::binary::{HandoverProbe, HandoverSupport};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Linux,
    OtherUnix,
    Windows,
}

impl Platform {
    pub const fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else {
            Self::OtherUnix
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryPaths {
    pub dir: PathBuf,
    pub exe: PathBuf,
    pub staged: PathBuf,
    pub old: PathBuf,
    pub link: PathBuf,
    pub download: PathBuf,
    pub lock: PathBuf,
}

impl BinaryPaths {
    pub fn for_exe(exe: &Path) -> Option<Self> {
        Self::for_exe_on(exe, Platform::current())
    }

    pub fn for_exe_on(exe: &Path, platform: Platform) -> Option<Self> {
        let name = exe.file_name()?;
        let dir = match exe.parent()? {
            parent if parent.as_os_str().is_empty() => PathBuf::from("."),
            parent => parent.to_path_buf(),
        };
        let stem = stem(name, platform);
        let named = |prefix: &str, suffix: &str| {
            let mut file = OsString::from(prefix);
            file.push(&stem);
            file.push(suffix);
            dir.join(file)
        };
        let (staged, old) = match platform {
            Platform::Windows => (named("", ".new.exe"), named("", ".old.exe")),
            Platform::Linux | Platform::OtherUnix => (named(".", ".new"), named("", ".old")),
        };
        Some(Self {
            exe: exe.to_path_buf(),
            staged,
            old,
            link: named(".", ".link"),
            download: named(".", ".download"),
            lock: named(".", ".lock"),
            dir,
        })
    }
}

fn stem(name: &OsStr, platform: Platform) -> OsString {
    if platform == Platform::Windows
        && let Some(text) = name.to_str()
        && let Some(cut) = text.len().checked_sub(4)
        && text
            .get(cut..)
            .is_some_and(|ext| ext.eq_ignore_ascii_case(".exe"))
        && let Some(stem) = text.get(..cut)
        && !stem.is_empty()
    {
        return OsString::from(stem);
    }
    name.to_os_string()
}

pub fn restart_kind(probe: &HandoverProbe) -> RestartRequirement {
    restart_on(
        Platform::current(),
        probe,
        HandoverSupport::current().as_ref(),
    )
}

pub fn restart_on(
    platform: Platform,
    probe: &HandoverProbe,
    running: Option<&HandoverSupport>,
) -> RestartRequirement {
    match platform {
        Platform::Windows => RestartRequirement::Manual,
        Platform::Linux if probe.can_inherit_from(running) => RestartRequirement::Hot,
        Platform::Linux | Platform::OtherUnix => RestartRequirement::Full,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwapStep {
    Remove(PathBuf),
    HardLink { from: PathBuf, to: PathBuf },
    Rename { from: PathBuf, to: PathBuf },
    Replace { from: PathBuf, to: PathBuf },
    SyncDir(PathBuf),
}

enum Undo {
    Stop,
    Skip,
    Run(SwapStep),
}

impl SwapStep {
    fn undo(&self) -> Undo {
        match self {
            Self::Replace { .. } => Undo::Stop,
            Self::Remove(_) | Self::SyncDir(_) => Undo::Skip,
            Self::HardLink { to, .. } => Undo::Run(Self::Remove(to.clone())),
            Self::Rename { from, to } => Undo::Run(Self::Rename {
                from: to.clone(),
                to: from.clone(),
            }),
        }
    }
}

impl fmt::Display for SwapStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Remove(path) => write!(f, "remove {}", path.display()),
            Self::HardLink { from, to } => {
                write!(f, "link {} as {}", from.display(), to.display())
            }
            Self::Rename { from, to } | Self::Replace { from, to } => {
                write!(f, "rename {} to {}", from.display(), to.display())
            }
            Self::SyncDir(dir) => write!(f, "sync {}", dir.display()),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("cannot {step}: {source}{}", if *.restored { "" } else { "; check the binary directory by hand" })]
pub struct SwapError {
    pub step: SwapStep,
    #[source]
    pub source: io::Error,
    pub restored: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwapPlan {
    steps: Vec<SwapStep>,
}

impl SwapPlan {
    pub fn commit(paths: &BinaryPaths, platform: Platform) -> Self {
        let steps = match platform {
            Platform::Windows => vec![
                SwapStep::Remove(paths.old.clone()),
                SwapStep::Rename {
                    from: paths.exe.clone(),
                    to: paths.old.clone(),
                },
                SwapStep::Rename {
                    from: paths.staged.clone(),
                    to: paths.exe.clone(),
                },
            ],
            Platform::Linux | Platform::OtherUnix => vec![
                SwapStep::Remove(paths.link.clone()),
                SwapStep::HardLink {
                    from: paths.exe.clone(),
                    to: paths.link.clone(),
                },
                SwapStep::Replace {
                    from: paths.staged.clone(),
                    to: paths.exe.clone(),
                },
                SwapStep::Replace {
                    from: paths.link.clone(),
                    to: paths.old.clone(),
                },
                SwapStep::SyncDir(paths.dir.clone()),
            ],
        };
        Self { steps }
    }

    pub fn replace_keeping_old(paths: &BinaryPaths, platform: Platform) -> Self {
        let replace = SwapStep::Replace {
            from: paths.staged.clone(),
            to: paths.exe.clone(),
        };
        let steps = match platform {
            Platform::Windows => vec![replace],
            Platform::Linux | Platform::OtherUnix => {
                vec![replace, SwapStep::SyncDir(paths.dir.clone())]
            }
        };
        Self { steps }
    }

    pub fn rollback(paths: &BinaryPaths, platform: Platform) -> Self {
        let steps = match platform {
            Platform::Windows => vec![
                SwapStep::Remove(paths.staged.clone()),
                SwapStep::Rename {
                    from: paths.exe.clone(),
                    to: paths.staged.clone(),
                },
                SwapStep::Rename {
                    from: paths.old.clone(),
                    to: paths.exe.clone(),
                },
                SwapStep::Rename {
                    from: paths.staged.clone(),
                    to: paths.old.clone(),
                },
            ],
            Platform::Linux | Platform::OtherUnix => vec![
                SwapStep::Remove(paths.staged.clone()),
                SwapStep::HardLink {
                    from: paths.exe.clone(),
                    to: paths.staged.clone(),
                },
                SwapStep::Replace {
                    from: paths.old.clone(),
                    to: paths.exe.clone(),
                },
                SwapStep::Rename {
                    from: paths.staged.clone(),
                    to: paths.old.clone(),
                },
                SwapStep::SyncDir(paths.dir.clone()),
            ],
        };
        Self { steps }
    }

    #[cfg(test)]
    fn steps(&self) -> &[SwapStep] {
        &self.steps
    }

    fn undo_after(&self, completed: usize) -> Option<Vec<SwapStep>> {
        let mut undo = Vec::new();
        for step in self.steps.iter().take(completed).rev() {
            match step.undo() {
                Undo::Stop => return None,
                Undo::Skip => {}
                Undo::Run(step) => undo.push(step),
            }
        }
        Some(undo)
    }

    pub fn execute(&self) -> Result<(), SwapError> {
        for (index, step) in self.steps.iter().enumerate() {
            if let Err(source) = apply(step) {
                let restored = self
                    .undo_after(index)
                    .is_some_and(|undo| undo.iter().all(|step| apply(step).is_ok()));
                return Err(SwapError {
                    step: step.clone(),
                    source,
                    restored,
                });
            }
        }
        Ok(())
    }
}

fn apply(step: &SwapStep) -> io::Result<()> {
    match step {
        SwapStep::Remove(path) => match std::fs::remove_file(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        },
        SwapStep::HardLink { from, to } => std::fs::hard_link(from, to),
        SwapStep::Rename { from, to } | SwapStep::Replace { from, to } => std::fs::rename(from, to),
        SwapStep::SyncDir(dir) => {
            if let Err(e) = sync_dir(dir) {
                tracing::warn!(dir = %dir.display(), error = %e, "cannot sync the binary directory");
            }
            Ok(())
        }
    }
}

#[cfg(unix)]
fn sync_dir(dir: &Path) -> io::Result<()> {
    std::fs::File::open(dir)?.sync_all()
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn paths(platform: Platform) -> BinaryPaths {
        let exe = match platform {
            Platform::Windows => "/opt/infrarust/infrarust.exe",
            Platform::Linux | Platform::OtherUnix => "/opt/infrarust/infrarust",
        };
        BinaryPaths::for_exe_on(Path::new(exe), platform).unwrap()
    }

    fn file_name(path: &Path) -> &str {
        path.file_name().unwrap().to_str().unwrap()
    }

    #[test]
    fn unix_names() {
        let paths = paths(Platform::Linux);
        assert_eq!(paths.dir, Path::new("/opt/infrarust"));
        assert_eq!(file_name(&paths.exe), "infrarust");
        assert_eq!(file_name(&paths.staged), ".infrarust.new");
        assert_eq!(file_name(&paths.old), "infrarust.old");
        assert_eq!(file_name(&paths.lock), ".infrarust.lock");
        assert_eq!(file_name(&paths.download), ".infrarust.download");
    }

    #[test]
    fn windows_names_keep_the_exe_extension() {
        let paths = paths(Platform::Windows);
        assert_eq!(file_name(&paths.staged), "infrarust.new.exe");
        assert_eq!(file_name(&paths.old), "infrarust.old.exe");
        assert_eq!(file_name(&paths.lock), ".infrarust.lock");
        let upper = BinaryPaths::for_exe_on(Path::new("C:/proxy/Infrarust.EXE"), Platform::Windows)
            .unwrap();
        assert_eq!(file_name(&upper.old), "Infrarust.old.exe");
    }

    #[test]
    fn names_follow_a_renamed_binary() {
        let paths = BinaryPaths::for_exe_on(Path::new("/srv/mc-proxy"), Platform::Linux).unwrap();
        assert_eq!(file_name(&paths.staged), ".mc-proxy.new");
        assert_eq!(file_name(&paths.old), "mc-proxy.old");
        let relative = BinaryPaths::for_exe_on(Path::new("infrarust"), Platform::Linux).unwrap();
        assert_eq!(relative.dir, Path::new("."));
        assert!(BinaryPaths::for_exe_on(Path::new("/"), Platform::Linux).is_none());
    }

    fn probe(handover: Option<HandoverSupport>) -> HandoverProbe {
        HandoverProbe {
            probe: 1,
            version: "2.0.0".to_string(),
            target: "x86_64-unknown-linux-musl".to_string(),
            official: true,
            docker: false,
            handover,
        }
    }

    #[test]
    fn restart_requirements() {
        let v1 = HandoverSupport {
            protocol: 1,
            accepts: vec![1],
        };
        let capable = probe(Some(v1.clone()));
        assert_eq!(
            restart_on(Platform::Linux, &capable, Some(&v1)),
            RestartRequirement::Hot
        );
        assert_eq!(
            restart_on(Platform::Linux, &capable, None),
            RestartRequirement::Full
        );
        assert_eq!(
            restart_on(Platform::Linux, &probe(None), Some(&v1)),
            RestartRequirement::Full
        );
        assert_eq!(
            restart_on(Platform::OtherUnix, &capable, Some(&v1)),
            RestartRequirement::Full
        );
        assert_eq!(
            restart_on(Platform::Windows, &capable, Some(&v1)),
            RestartRequirement::Manual
        );
        let here = match Platform::current() {
            Platform::Linux => RestartRequirement::Hot,
            Platform::OtherUnix => RestartRequirement::Full,
            Platform::Windows => RestartRequirement::Manual,
        };
        assert_eq!(restart_kind(&capable), here);
    }

    #[test]
    fn the_unix_commit_links_before_it_renames() {
        let p = paths(Platform::Linux);
        assert_eq!(
            SwapPlan::commit(&p, Platform::Linux).steps(),
            [
                SwapStep::Remove(p.link.clone()),
                SwapStep::HardLink {
                    from: p.exe.clone(),
                    to: p.link.clone()
                },
                SwapStep::Replace {
                    from: p.staged.clone(),
                    to: p.exe.clone()
                },
                SwapStep::Replace {
                    from: p.link.clone(),
                    to: p.old.clone()
                },
                SwapStep::SyncDir(p.dir.clone()),
            ]
        );
    }

    #[test]
    fn a_second_install_only_replaces_the_exe() {
        for platform in [Platform::Linux, Platform::Windows] {
            let p = paths(platform);
            let plan = SwapPlan::replace_keeping_old(&p, platform);
            assert_eq!(
                plan.steps()[0],
                SwapStep::Replace {
                    from: p.staged.clone(),
                    to: p.exe.clone()
                },
                "{platform:?}"
            );
            assert!(
                plan.steps()
                    .iter()
                    .all(|step| matches!(step, SwapStep::Replace { .. } | SwapStep::SyncDir(_))),
                "{platform:?}"
            );
        }
        let p = paths(Platform::Linux);
        assert_eq!(
            SwapPlan::replace_keeping_old(&p, Platform::Linux)
                .steps()
                .last(),
            Some(&SwapStep::SyncDir(p.dir.clone()))
        );
    }

    #[test]
    fn the_windows_commit_moves_the_running_exe_aside() {
        let p = paths(Platform::Windows);
        assert_eq!(
            SwapPlan::commit(&p, Platform::Windows).steps(),
            [
                SwapStep::Remove(p.old.clone()),
                SwapStep::Rename {
                    from: p.exe.clone(),
                    to: p.old.clone()
                },
                SwapStep::Rename {
                    from: p.staged.clone(),
                    to: p.exe.clone()
                },
            ]
        );
    }

    #[test]
    fn undo_reverses_completed_steps_until_an_irreversible_one() {
        let p = paths(Platform::Windows);
        let commit = SwapPlan::commit(&p, Platform::Windows);
        assert_eq!(
            commit.undo_after(2),
            Some(vec![SwapStep::Rename {
                from: p.old.clone(),
                to: p.exe.clone()
            }])
        );
        assert_eq!(commit.undo_after(1), Some(Vec::new()));

        let p = paths(Platform::Linux);
        let rollback = SwapPlan::rollback(&p, Platform::Linux);
        assert_eq!(
            rollback.undo_after(2),
            Some(vec![SwapStep::Remove(p.staged.clone())])
        );
        assert_eq!(rollback.undo_after(3), None);
    }

    fn sandbox(platform: Platform) -> (tempfile::TempDir, BinaryPaths) {
        let dir = tempfile::tempdir().unwrap();
        let name = match platform {
            Platform::Windows => "infrarust.exe",
            Platform::Linux | Platform::OtherUnix => "infrarust",
        };
        let paths = BinaryPaths::for_exe_on(&dir.path().join(name), platform).unwrap();
        std::fs::write(&paths.exe, b"old").unwrap();
        std::fs::write(&paths.staged, b"new").unwrap();
        (dir, paths)
    }

    fn read(path: &Path) -> Option<Vec<u8>> {
        std::fs::read(path).ok()
    }

    #[test]
    fn both_plans_commit_and_roll_back_reversibly_on_disk() {
        for platform in [Platform::Linux, Platform::Windows] {
            let (_dir, p) = sandbox(platform);
            std::fs::write(&p.old, b"stale").unwrap();
            SwapPlan::commit(&p, platform).execute().unwrap();
            assert_eq!(read(&p.exe).as_deref(), Some(&b"new"[..]), "{platform:?}");
            assert_eq!(read(&p.old).as_deref(), Some(&b"old"[..]), "{platform:?}");
            assert!(!p.staged.exists());

            SwapPlan::rollback(&p, platform).execute().unwrap();
            assert_eq!(read(&p.exe).as_deref(), Some(&b"old"[..]), "{platform:?}");
            assert_eq!(read(&p.old).as_deref(), Some(&b"new"[..]), "{platform:?}");
            assert!(!p.staged.exists());

            SwapPlan::rollback(&p, platform).execute().unwrap();
            assert_eq!(read(&p.exe).as_deref(), Some(&b"new"[..]), "{platform:?}");
            assert_eq!(read(&p.old).as_deref(), Some(&b"old"[..]), "{platform:?}");
        }
    }

    #[test]
    fn a_failed_windows_commit_puts_the_exe_back() {
        let (_dir, p) = sandbox(Platform::Windows);
        std::fs::remove_file(&p.staged).unwrap();
        let error = SwapPlan::commit(&p, Platform::Windows)
            .execute()
            .unwrap_err();
        assert!(error.restored);
        assert_eq!(read(&p.exe).as_deref(), Some(&b"old"[..]));
        assert!(!p.old.exists());
    }

    #[test]
    fn a_failed_unix_commit_leaves_the_exe_alone() {
        let (_dir, p) = sandbox(Platform::Linux);
        std::fs::remove_file(&p.staged).unwrap();
        let error = SwapPlan::commit(&p, Platform::Linux).execute().unwrap_err();
        assert!(matches!(error.step, SwapStep::Replace { .. }));
        assert!(error.restored);
        assert_eq!(read(&p.exe).as_deref(), Some(&b"old"[..]));
        assert!(!p.old.exists());
        assert!(!p.link.exists());
    }

    #[test]
    fn a_failed_unix_commit_keeps_the_previous_rollback_binary() {
        let (_dir, p) = sandbox(Platform::Linux);
        std::fs::write(&p.old, b"previous").unwrap();
        std::fs::remove_file(&p.staged).unwrap();
        let error = SwapPlan::commit(&p, Platform::Linux).execute().unwrap_err();
        assert!(error.restored);
        assert_eq!(read(&p.exe).as_deref(), Some(&b"old"[..]));
        assert_eq!(read(&p.old).as_deref(), Some(&b"previous"[..]));
    }

    #[test]
    fn a_rollback_that_fails_after_replacing_the_exe_is_not_restored() {
        let (_dir, p) = sandbox(Platform::Linux);
        std::fs::remove_file(&p.staged).unwrap();
        std::fs::write(&p.old, b"previous").unwrap();
        let mut plan = SwapPlan::rollback(&p, Platform::Linux);
        plan.steps.insert(
            3,
            SwapStep::Rename {
                from: p.dir.join("missing"),
                to: p.old.clone(),
            },
        );
        let error = plan.execute().unwrap_err();
        assert!(!error.restored);
        assert_eq!(read(&p.exe).as_deref(), Some(&b"previous"[..]));
    }
}

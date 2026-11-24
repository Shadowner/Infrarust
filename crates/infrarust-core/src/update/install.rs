use std::ffi::OsString;
use std::fmt;
use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use infrarust_api::services::RestartRequirement;
use infrarust_config::UpdateChannel;
use semver::Version;

use crate::binary::{self, ConfigCheck, HandoverProbe, RunningBinary};

use super::archive::{self, ArchiveError, ArchiveFormat};
use super::channel::{ChannelFilter, parse_version};
use super::check::{self, Candidate};
use super::download;
use super::error::UpdateError;
use super::gate::{self, Refusal};
use super::keys::TrustedKeys;
use super::manifest::{Artifact, VerifiedManifest};
use super::platform::{self, BinaryPaths, Platform, SwapPlan};
use super::source::ReleaseSource;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallStep {
    Resolving,
    Resolved {
        version: Version,
        file: String,
        size: u64,
    },
    Downloaded,
    Extracting,
    Probing,
    CheckingConfig,
    Committing,
    Installed {
        version: Version,
        restart: RestartRequirement,
    },
}

impl fmt::Display for InstallStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Resolving => f.write_str("looking for the release"),
            Self::Resolved {
                version,
                file,
                size,
            } => write!(f, "downloading {file} ({size} bytes) for {version}"),
            Self::Downloaded => f.write_str("download verified"),
            Self::Extracting => f.write_str("extracting the new binary"),
            Self::Probing => f.write_str("probing the new binary"),
            Self::CheckingConfig => f.write_str("checking the configuration with the new binary"),
            Self::Committing => f.write_str("replacing the binary"),
            Self::Installed { version, restart } => match restart {
                RestartRequirement::Hot => {
                    write!(f, "installed {version}; a hot restart applies it")
                }
                RestartRequirement::Full => {
                    write!(f, "installed {version}; restart the proxy to apply it")
                }
                RestartRequirement::Manual => write!(
                    f,
                    "installed {version}; stop and start the proxy by hand to apply it"
                ),
            },
        }
    }
}

pub trait InstallObserver: Send + Sync {
    fn on_step(&self, _step: &InstallStep) {}

    fn on_download(&self, _received: u64, _total: u64) {}
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoopObserver;

impl InstallObserver for NoopObserver {}

#[derive(Debug, Clone, Copy, Default)]
pub struct LogObserver;

impl InstallObserver for LogObserver {
    fn on_step(&self, step: &InstallStep) {
        tracing::info!("update: {step}");
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallTarget {
    Latest(UpdateChannel),
    Version(Version),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub version: Version,
    pub restart: RestartRequirement,
    pub security: bool,
    pub probe: HandoverProbe,
    pub config: ConfigCheck,
    pub exe: PathBuf,
    pub previous: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RolledBack {
    pub version: String,
    pub restart: RestartRequirement,
    pub probe: HandoverProbe,
    pub exe: PathBuf,
    pub previous: PathBuf,
}

pub struct Installer<'a> {
    binary: &'a RunningBinary,
    source: &'a ReleaseSource,
    keys: &'a TrustedKeys,
}

impl<'a> Installer<'a> {
    pub const fn new(
        binary: &'a RunningBinary,
        source: &'a ReleaseSource,
        keys: &'a TrustedKeys,
    ) -> Self {
        Self {
            binary,
            source,
            keys,
        }
    }

    pub async fn install(
        &self,
        target: InstallTarget,
        observer: &dyn InstallObserver,
    ) -> Result<Installed, UpdateError> {
        let build = &self.binary.build;
        let requested = match &target {
            InstallTarget::Version(version) => Some(version.to_string()),
            InstallTarget::Latest(_) => None,
        };
        let refused = |refusal| UpdateError::Refused {
            refusal,
            version: requested.clone(),
        };
        if let Some(refusal) = gate::build_refusal(build) {
            return Err(refused(refusal));
        }
        let paths = gate::paths_for(self.binary.exe.path()).map_err(refused)?;
        let _lock = DirLock::acquire(&paths)
            .await
            .map_err(|e| e.into_update_error(&paths))?;
        let scratch = Scratch::new(&paths);
        scratch.clear()?;
        gate::check_writable(&paths).map_err(refused)?;

        observer.on_step(&InstallStep::Resolving);
        let candidate = self.resolve(target).await?;
        let (artifact, format) = select_artifact(&candidate, build.target)?;
        let asset =
            candidate
                .release
                .asset(&artifact.file)
                .ok_or_else(|| UpdateError::MissingAsset {
                    tag: candidate.release.tag_name.clone(),
                    file: artifact.file.clone(),
                })?;
        observer.on_step(&InstallStep::Resolved {
            version: candidate.version().clone(),
            file: artifact.file.clone(),
            size: artifact.size,
        });
        let progress = |received, total| observer.on_download(received, total);
        download::download(
            self.source,
            &asset.browser_download_url,
            &paths.download,
            artifact,
            &progress,
        )
        .await?;
        observer.on_step(&InstallStep::Downloaded);
        apply(
            &paths,
            &candidate.manifest,
            artifact,
            format,
            &self.binary.args,
            observer,
        )
        .await
    }

    async fn resolve(&self, target: InstallTarget) -> Result<Candidate, UpdateError> {
        match target {
            InstallTarget::Latest(channel) => {
                let current = running_version(self.binary)?;
                let filter = ChannelFilter::new(channel, &current);
                check::newest(self.source, self.keys, filter, &current)
                    .await?
                    .ok_or(UpdateError::NothingToInstall)
            }
            InstallTarget::Version(version) => {
                check::pinned(self.source, self.keys, &version).await
            }
        }
    }
}

pub(super) fn running_version(binary: &RunningBinary) -> Result<Version, UpdateError> {
    parse_version(binary.build.version)
        .ok_or_else(|| UpdateError::InvalidVersion(binary.build.version.to_string()))
}

fn select_artifact<'c>(
    candidate: &'c Candidate,
    target: &str,
) -> Result<(&'c Artifact, ArchiveFormat), UpdateError> {
    let artifact = candidate
        .manifest
        .artifact_for(target)
        .ok_or_else(|| UpdateError::Refused {
            refusal: Refusal::UnsupportedTarget {
                target: target.to_string(),
            },
            version: Some(candidate.version().to_string()),
        })?;
    let format = ArchiveFormat::for_target(target);
    if !artifact.file.ends_with(format.extension()) {
        return Err(ArchiveError::WrongFormat {
            file: artifact.file.clone(),
            expected: format.extension(),
        }
        .into());
    }
    Ok((artifact, format))
}

pub(super) async fn apply(
    paths: &BinaryPaths,
    manifest: &VerifiedManifest,
    artifact: &Artifact,
    format: ArchiveFormat,
    args: &[OsString],
    observer: &dyn InstallObserver,
) -> Result<Installed, UpdateError> {
    let _scratch = Scratch::new(paths);
    observer.on_step(&InstallStep::Extracting);
    stage(paths, format, &artifact.binary).await?;
    let _ = tokio::fs::remove_file(&paths.download).await;

    observer.on_step(&InstallStep::Probing);
    let probe = binary::probe(&paths.staged).await?;
    let probed = parse_version(&probe.version);
    if probed.as_ref() != Some(manifest.version()) || !artifact.targets.contains(&probe.target) {
        return Err(UpdateError::ProbeMismatch {
            expected: manifest.version().to_string(),
            found_version: probe.version,
            found_target: probe.target,
        });
    }

    observer.on_step(&InstallStep::CheckingConfig);
    let config = binary::check_config(&paths.staged, args).await?;
    if !config.ok {
        return Err(UpdateError::ConfigRejected {
            config: config.config,
            reason: config
                .error
                .unwrap_or_else(|| "no reason given".to_string()),
        });
    }

    observer.on_step(&InstallStep::Committing);
    SwapPlan::commit(paths, Platform::current()).execute()?;
    let restart = platform::restart_kind(&probe);
    observer.on_step(&InstallStep::Installed {
        version: manifest.version().clone(),
        restart,
    });
    Ok(Installed {
        version: manifest.version().clone(),
        restart,
        security: manifest.security(),
        probe,
        config,
        exe: paths.exe.clone(),
        previous: paths.old.clone(),
    })
}

pub async fn rollback(binary: &RunningBinary) -> Result<RolledBack, UpdateError> {
    let paths = gate::paths_for(binary.exe.path()).map_err(|refusal| UpdateError::Refused {
        refusal,
        version: None,
    })?;
    let _lock = DirLock::acquire(&paths)
        .await
        .map_err(|e| e.into_update_error(&paths))?;
    if std::fs::symlink_metadata(&paths.old).is_err() {
        return Err(UpdateError::NothingToRollBack);
    }
    let probe = binary::probe(&paths.old).await?;
    let config = binary::check_config(&paths.old, &binary.args).await?;
    if !config.ok {
        return Err(UpdateError::ConfigRejected {
            config: config.config,
            reason: config
                .error
                .unwrap_or_else(|| "no reason given".to_string()),
        });
    }
    SwapPlan::rollback(&paths, Platform::current()).execute()?;
    Ok(RolledBack {
        version: probe.version.clone(),
        restart: platform::restart_kind(&probe),
        probe,
        exe: paths.exe.clone(),
        previous: paths.old.clone(),
    })
}

async fn stage(
    paths: &BinaryPaths,
    format: ArchiveFormat,
    binary: &str,
) -> Result<(), UpdateError> {
    let archive = paths.download.clone();
    let staged = paths.staged.clone();
    let exe = paths.exe.clone();
    let binary = binary.to_string();
    tokio::task::spawn_blocking(move || stage_blocking(&archive, format, &binary, &staged, &exe))
        .await
        .map_err(|e| UpdateError::Io {
            context: "the extraction task failed".to_string(),
            source: io::Error::other(e),
        })?
}

fn stage_blocking(
    archive: &Path,
    format: ArchiveFormat,
    binary: &str,
    staged: &Path,
    exe: &Path,
) -> Result<(), UpdateError> {
    let context = |source| UpdateError::Io {
        context: format!("cannot write {}", staged.display()),
        source,
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(staged)
        .map_err(context)?;
    archive::extract(archive, format, binary, &mut file)?;
    finalize(&file, exe).map_err(context)?;
    file.sync_all().map_err(context)
}

#[cfg(unix)]
fn finalize(file: &File, exe: &Path) -> io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, fchown};

    let staged = file.metadata()?;
    if staged.uid() == 0 {
        let owner = std::fs::metadata(exe)?;
        if owner.uid() != staged.uid() || owner.gid() != staged.gid() {
            fchown(file, Some(owner.uid()), Some(owner.gid()))?;
        }
    }
    file.set_permissions(std::fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn finalize(_file: &File, _exe: &Path) -> io::Result<()> {
    Ok(())
}

pub(super) enum LockError {
    Busy,
    Open(io::Error),
    Lock(io::Error),
}

impl LockError {
    pub(super) fn into_update_error(self, paths: &BinaryPaths) -> UpdateError {
        match self {
            Self::Busy => UpdateError::Busy,
            Self::Open(e) => UpdateError::Refused {
                refusal: Refusal::NotWritable {
                    dir: paths.dir.clone(),
                    reason: e.to_string(),
                },
                version: None,
            },
            Self::Lock(source) => UpdateError::Io {
                context: format!("cannot lock {}", paths.lock.display()),
                source,
            },
        }
    }
}

const LOCK_ATTEMPTS: u32 = 20;
const LOCK_RETRY: Duration = Duration::from_millis(10);

pub(super) struct DirLock {
    _file: File,
}

impl DirLock {
    pub(super) async fn acquire(paths: &BinaryPaths) -> Result<Self, LockError> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&paths.lock)
            .map_err(LockError::Open)?;
        let mut attempt = 1;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Self { _file: file }),
                Err(TryLockError::WouldBlock) if attempt < LOCK_ATTEMPTS => {
                    attempt += 1;
                    tokio::time::sleep(LOCK_RETRY).await;
                }
                Err(TryLockError::WouldBlock) => return Err(LockError::Busy),
                Err(TryLockError::Error(e)) => return Err(LockError::Lock(e)),
            }
        }
    }
}

struct Scratch<'a> {
    paths: &'a BinaryPaths,
}

impl<'a> Scratch<'a> {
    const fn new(paths: &'a BinaryPaths) -> Self {
        Self { paths }
    }

    fn clear(&self) -> Result<(), UpdateError> {
        for path in [&self.paths.staged, &self.paths.download] {
            match std::fs::remove_file(path) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => {
                    return Err(UpdateError::Io {
                        context: format!("cannot remove the stale {}", path.display()),
                        source: e,
                    });
                }
                _ => {}
            }
        }
        Ok(())
    }
}

impl Drop for Scratch<'_> {
    fn drop(&mut self) {
        let _ = self.clear();
    }
}

#[cfg(all(test, unix))]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use crate::binary::{BuildInfo, ExeSnapshot};

    use super::super::testing::{
        ARCHIVE, Entry, FakeBinary, SigningKey, TEST_TARGET, manifest_json, sleep_command, tar_gz,
        write_executable,
    };
    use super::*;

    const BUILD: BuildInfo = BuildInfo {
        version: "2.0.0-beta.5",
        target: TEST_TARGET,
        official: true,
        docker: false,
    };

    struct Sandbox {
        dir: tempfile::TempDir,
        paths: BinaryPaths,
        binary: Arc<RunningBinary>,
        old_script: Vec<u8>,
    }

    impl Sandbox {
        fn new() -> Self {
            Self::with_current(FakeBinary::new("2.0.0-beta.5"))
        }

        fn with_current(current: FakeBinary) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let exe = dir.path().join("infrarust");
            let old_script = current.script();
            write_executable(&exe, &old_script);
            let paths = BinaryPaths::for_exe(&exe).unwrap();
            let binary = RunningBinary::new(
                BUILD,
                ExeSnapshot::at(exe),
                vec![OsString::from("-c"), OsString::from("proxy.toml")],
            );
            Self {
                dir,
                paths,
                binary,
                old_script,
            }
        }

        fn listing(&self) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(self.dir.path())
                .unwrap()
                .map(|entry| entry.unwrap().file_name().into_string().unwrap())
                .collect();
            names.sort();
            names
        }

        fn exe(&self) -> Vec<u8> {
            std::fs::read(&self.paths.exe).unwrap()
        }

        fn assert_untouched(&self) {
            assert_eq!(self.exe(), self.old_script);
            assert!(!self.binary.exe.changed_on_disk());
            assert_eq!(self.listing(), ["infrarust"]);
        }

        async fn apply(&self, version: &str, new: &FakeBinary) -> Result<Installed, UpdateError> {
            let archive = tar_gz(&[Entry::file("infrarust", &new.script())]);
            std::fs::write(&self.paths.download, &archive).unwrap();
            let manifest = manifest_json(version, &[(ARCHIVE, "infrarust", &archive)]);
            let manifest =
                VerifiedManifest::from_signed(manifest.as_bytes(), &format!("v{version}")).unwrap();
            let artifact = manifest.artifact_for(TEST_TARGET).unwrap().clone();
            apply(
                &self.paths,
                &manifest,
                &artifact,
                ArchiveFormat::TarGz,
                &self.binary.args,
                &NoopObserver,
            )
            .await
        }
    }

    #[tokio::test]
    async fn the_new_binary_replaces_the_exe_and_the_old_one_stays_linked() {
        let sandbox = Sandbox::new();
        let original = std::fs::metadata(&sandbox.paths.exe).unwrap();
        let new = FakeBinary::new("2.0.0-beta.6");
        let installed = sandbox.apply("2.0.0-beta.6", &new).await.unwrap();

        assert_eq!(installed.version, Version::parse("2.0.0-beta.6").unwrap());
        assert_eq!(installed.restart, RestartRequirement::Full);
        assert!(installed.config.ok);
        assert_eq!(sandbox.exe(), new.script());
        let old = std::fs::metadata(&sandbox.paths.old).unwrap();
        assert_eq!(old.ino(), original.ino());
        assert_eq!(
            std::fs::read(&sandbox.paths.old).unwrap(),
            sandbox.old_script
        );
        let mode = std::fs::metadata(&sandbox.paths.exe)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755);
        assert!(sandbox.binary.exe.changed_on_disk());
        assert_eq!(sandbox.listing(), ["infrarust", "infrarust.old"]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_exe_path_exists_at_every_instant() {
        let sandbox = Sandbox::new();
        std::fs::write(&sandbox.paths.old, b"stale").unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let misses = Arc::new(AtomicUsize::new(0));
        let watcher = {
            let exe = sandbox.paths.exe.clone();
            let stop = Arc::clone(&stop);
            let misses = Arc::clone(&misses);
            std::thread::spawn(move || {
                let mut looks = 0u64;
                while !stop.load(Ordering::SeqCst) {
                    if std::fs::symlink_metadata(&exe).is_err() {
                        misses.fetch_add(1, Ordering::SeqCst);
                    }
                    looks += 1;
                }
                looks
            })
        };
        sandbox
            .apply("2.0.0-beta.6", &FakeBinary::new("2.0.0-beta.6"))
            .await
            .unwrap();
        rollback(&sandbox.binary).await.unwrap();
        stop.store(true, Ordering::SeqCst);
        let looks = watcher.join().unwrap();
        assert!(looks > 0);
        assert_eq!(misses.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_probe_reporting_another_version_changes_nothing() {
        let sandbox = Sandbox::new();
        let result = sandbox
            .apply("2.0.0-beta.6", &FakeBinary::new("2.0.0-beta.7"))
            .await;
        assert!(
            matches!(result, Err(UpdateError::ProbeMismatch { .. })),
            "{result:?}"
        );
        sandbox.assert_untouched();
    }

    #[tokio::test]
    async fn a_probe_reporting_another_target_changes_nothing() {
        let sandbox = Sandbox::new();
        let mut new = FakeBinary::new("2.0.0-beta.6");
        new.probe = format!(
            "echo '{}'",
            super::super::testing::probe_json("2.0.0-beta.6", "aarch64-apple-darwin")
        );
        let result = sandbox.apply("2.0.0-beta.6", &new).await;
        assert!(
            matches!(result, Err(UpdateError::ProbeMismatch { .. })),
            "{result:?}"
        );
        sandbox.assert_untouched();
    }

    #[tokio::test]
    async fn a_binary_that_cannot_be_probed_changes_nothing() {
        let sandbox = Sandbox::new();
        let mut new = FakeBinary::new("2.0.0-beta.6");
        new.probe = "echo garbage".to_string();
        let result = sandbox.apply("2.0.0-beta.6", &new).await;
        assert!(matches!(result, Err(UpdateError::Probe(_))), "{result:?}");
        sandbox.assert_untouched();
    }

    #[tokio::test(start_paused = true)]
    async fn a_probe_timeout_changes_nothing() {
        let sandbox = Sandbox::new();
        let mut new = FakeBinary::new("2.0.0-beta.6");
        new.probe = format!("{} 30", sleep_command());
        let result = sandbox.apply("2.0.0-beta.6", &new).await;
        assert!(
            matches!(
                result,
                Err(UpdateError::Probe(binary::ProbeError::Timeout { .. }))
            ),
            "{result:?}"
        );
        sandbox.assert_untouched();
    }

    #[tokio::test]
    async fn a_config_rejection_changes_nothing() {
        let sandbox = Sandbox::new();
        let result = sandbox
            .apply(
                "2.0.0-beta.6",
                &FakeBinary::new("2.0.0-beta.6").rejecting_config(),
            )
            .await;
        match result {
            Err(UpdateError::ConfigRejected { reason, .. }) => {
                assert!(reason.contains("update"), "{reason}");
            }
            other => panic!("expected a config rejection, got {other:?}"),
        }
        sandbox.assert_untouched();
    }

    #[tokio::test]
    async fn a_corrupt_archive_changes_nothing() {
        let sandbox = Sandbox::new();
        let archive = tar_gz(&[Entry::symlink("infrarust", "/bin/sh")]);
        std::fs::write(&sandbox.paths.download, &archive).unwrap();
        let manifest = manifest_json("2.0.0-beta.6", &[(ARCHIVE, "infrarust", &archive)]);
        let manifest = VerifiedManifest::from_signed(manifest.as_bytes(), "v2.0.0-beta.6").unwrap();
        let artifact = manifest.artifact_for(TEST_TARGET).unwrap().clone();
        let result = apply(
            &sandbox.paths,
            &manifest,
            &artifact,
            ArchiveFormat::TarGz,
            &sandbox.binary.args,
            &NoopObserver,
        )
        .await;
        assert!(
            matches!(
                result,
                Err(UpdateError::Archive(ArchiveError::NotRegular(_)))
            ),
            "{result:?}"
        );
        sandbox.assert_untouched();
    }

    #[tokio::test]
    async fn the_directory_lock_blocks_a_second_install() {
        let sandbox = Sandbox::new();
        let source = ReleaseSource::new("http://127.0.0.1:9/releases", "2.0.0-beta.5").unwrap();
        let keys = TrustedKeys::from_strs([SigningKey::generate().public()]).unwrap();
        let installer = Installer::new(&sandbox.binary, &source, &keys);

        let held = DirLock::acquire(&sandbox.paths).await.ok().unwrap();
        let result = installer
            .install(InstallTarget::Latest(UpdateChannel::Auto), &NoopObserver)
            .await;
        assert!(matches!(result, Err(UpdateError::Busy)), "{result:?}");
        assert!(matches!(
            rollback(&sandbox.binary).await,
            Err(UpdateError::Busy)
        ));
        assert!(matches!(
            DirLock::acquire(&sandbox.paths).await,
            Err(LockError::Busy)
        ));
        drop(held);

        assert!(matches!(
            rollback(&sandbox.binary).await,
            Err(UpdateError::NothingToRollBack)
        ));
        assert_eq!(sandbox.exe(), sandbox.old_script);
    }

    #[tokio::test]
    async fn stale_scratch_files_are_cleared_under_the_lock() {
        let sandbox = Sandbox::new();
        std::fs::write(&sandbox.paths.staged, b"stale").unwrap();
        std::fs::write(&sandbox.paths.download, b"stale").unwrap();
        let source = ReleaseSource::new("http://127.0.0.1:9/releases", "2.0.0-beta.5").unwrap();
        let keys = TrustedKeys::from_strs([SigningKey::generate().public()]).unwrap();
        let result = Installer::new(&sandbox.binary, &source, &keys)
            .install(InstallTarget::Latest(UpdateChannel::Auto), &NoopObserver)
            .await;
        assert!(
            matches!(result, Err(UpdateError::Network { .. })),
            "{result:?}"
        );
        assert!(!sandbox.paths.staged.exists());
        assert!(!sandbox.paths.download.exists());
        assert_eq!(sandbox.exe(), sandbox.old_script);
    }

    #[tokio::test]
    async fn a_refused_build_touches_nothing() {
        let sandbox = Sandbox::new();
        let binary = RunningBinary::new(
            BuildInfo {
                official: false,
                ..BUILD
            },
            ExeSnapshot::at(sandbox.paths.exe.clone()),
            Vec::new(),
        );
        let source = ReleaseSource::new("http://127.0.0.1:9/releases", "2.0.0-beta.5").unwrap();
        let keys = TrustedKeys::from_strs([SigningKey::generate().public()]).unwrap();
        let result = Installer::new(&binary, &source, &keys)
            .install(
                InstallTarget::Version(Version::parse("2.0.0-beta.6").unwrap()),
                &NoopObserver,
            )
            .await;
        match result {
            Err(
                error @ UpdateError::Refused {
                    refusal: Refusal::NotOfficial,
                    ..
                },
            ) => {
                assert!(
                    error
                        .to_string()
                        .contains("cargo install infrarust --version 2.0.0-beta.6")
                );
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
        sandbox.assert_untouched();
    }

    #[tokio::test]
    async fn rollback_restores_the_previous_binary_and_is_reversible() {
        let sandbox = Sandbox::new();
        let original = std::fs::metadata(&sandbox.paths.exe).unwrap().ino();
        let new = FakeBinary::new("2.0.0-beta.6");
        sandbox.apply("2.0.0-beta.6", &new).await.unwrap();

        let back = rollback(&sandbox.binary).await.unwrap();
        assert_eq!(back.version, "2.0.0-beta.5");
        assert_eq!(sandbox.exe(), sandbox.old_script);
        assert_eq!(
            std::fs::metadata(&sandbox.paths.exe).unwrap().ino(),
            original
        );
        assert_eq!(std::fs::read(&sandbox.paths.old).unwrap(), new.script());
        assert!(!sandbox.binary.exe.changed_on_disk());

        let forward = rollback(&sandbox.binary).await.unwrap();
        assert_eq!(forward.version, "2.0.0-beta.6");
        assert_eq!(sandbox.exe(), new.script());
        assert_eq!(
            std::fs::read(&sandbox.paths.old).unwrap(),
            sandbox.old_script
        );
        assert_eq!(
            sandbox.listing(),
            [".infrarust.lock", "infrarust", "infrarust.old"]
        );
    }

    #[tokio::test]
    async fn rollback_refuses_a_previous_binary_that_rejects_the_config() {
        let sandbox = Sandbox::with_current(FakeBinary::new("2.0.0-beta.5").rejecting_config());
        let new = FakeBinary::new("2.0.0-beta.6");
        sandbox.apply("2.0.0-beta.6", &new).await.unwrap();
        let result = rollback(&sandbox.binary).await;
        assert!(
            matches!(result, Err(UpdateError::ConfigRejected { .. })),
            "{result:?}"
        );
        assert_eq!(sandbox.exe(), new.script());
        assert_eq!(
            std::fs::read(&sandbox.paths.old).unwrap(),
            sandbox.old_script
        );
    }
}

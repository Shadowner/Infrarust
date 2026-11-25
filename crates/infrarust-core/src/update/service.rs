use std::collections::HashSet;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::SystemTime;

use infrarust_api::services::{
    AvailableUpdate, InstallRequest, InstallState, ProxyRestart, RestartRequirement,
    UpdateRequestError, UpdateStatus,
};
use infrarust_config::{UpdateChannel, UpdateConfig};
use semver::Version;

use crate::binary::{HandoverProbe, PROBE_SCHEMA, RunningBinary};

use super::channel::{ChannelFilter, parse_version};
use super::check::{self, Candidate};
use super::error::UpdateError;
use super::gate::{self, Refusal};
use super::install::{
    self, DirLock, InstallObserver, InstallStep, InstallTarget, Installed, Installer, LockError,
    LogObserver, RolledBack,
};
use super::keys::TrustedKeys;
use super::platform;
use super::source::ReleaseSource;

pub struct UpdateService {
    me: Weak<Self>,
    pub(super) config: UpdateConfig,
    pub(super) binary: Arc<RunningBinary>,
    restart: Option<Arc<dyn ProxyRestart>>,
    keys: Option<TrustedKeys>,
    source: Result<ReleaseSource, String>,
    state: Mutex<State>,
    busy: AtomicBool,
    checking: tokio::sync::Mutex<()>,
}

struct State {
    last_check: Option<SystemTime>,
    last_error: Option<String>,
    available: Option<AvailableUpdate>,
    install: InstallState,
    refusal: Option<Refusal>,
    binary_changed: bool,
    announced: HashSet<Version>,
}

impl fmt::Debug for UpdateService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UpdateService")
            .field("config", &self.config)
            .field("binary", &self.binary)
            .field("restart", &self.restart.is_some())
            .finish_non_exhaustive()
    }
}

impl UpdateService {
    pub fn new(
        config: UpdateConfig,
        binary: Arc<RunningBinary>,
        restart: Option<Arc<dyn ProxyRestart>>,
    ) -> Arc<Self> {
        let keys = TrustedKeys::release().ok();
        Self::build(config, binary, restart, keys)
    }

    pub fn with_keys(
        config: UpdateConfig,
        binary: Arc<RunningBinary>,
        restart: Option<Arc<dyn ProxyRestart>>,
        keys: TrustedKeys,
    ) -> Arc<Self> {
        Self::build(config, binary, restart, Some(keys))
    }

    fn build(
        config: UpdateConfig,
        binary: Arc<RunningBinary>,
        restart: Option<Arc<dyn ProxyRestart>>,
        keys: Option<TrustedKeys>,
    ) -> Arc<Self> {
        let source =
            ReleaseSource::new(&config.url, binary.build.version).map_err(|e| e.to_string());
        let state = State {
            last_check: None,
            last_error: None,
            available: None,
            install: InstallState::Idle,
            refusal: gate::build_refusal(&binary.build),
            binary_changed: false,
            announced: HashSet::new(),
        };
        Arc::new_cyclic(|me| Self {
            me: Weak::clone(me),
            config,
            binary,
            restart,
            keys,
            source,
            state: Mutex::new(state),
            busy: AtomicBool::new(false),
            checking: tokio::sync::Mutex::new(()),
        })
    }

    pub const fn config(&self) -> &UpdateConfig {
        &self.config
    }

    pub fn binary(&self) -> &RunningBinary {
        &self.binary
    }

    pub fn status(&self) -> UpdateStatus {
        let state = self.state();
        let version = state.available.as_ref().map(|a| a.version.as_str());
        UpdateStatus {
            current_version: self.binary.build.version.to_string(),
            checks_enabled: self.config.check,
            last_check: state.last_check,
            last_error: state.last_error.clone(),
            available: state.available.clone(),
            install: state.install.clone(),
            install_refusal: state.refusal.as_ref().map(|r| r.instructions(version)),
            binary_changed: state.binary_changed,
        }
    }

    pub async fn check(
        &self,
        channel: Option<UpdateChannel>,
    ) -> Result<Option<AvailableUpdate>, UpdateError> {
        let _checking = self.checking.lock().await;
        let found = self.find(channel.unwrap_or(self.config.channel)).await;
        let refusal = self.refusal_now().await;
        let mut state = self.state();
        state.last_check = Some(SystemTime::now());
        if let Some(refusal) = refusal {
            state.refusal = refusal;
        }
        match found {
            Ok(candidate) => {
                let available = candidate.as_ref().map(|c| self.describe(c));
                if let (Some(candidate), Some(update)) = (&candidate, &available) {
                    announce(
                        &mut state,
                        candidate.version(),
                        update,
                        self.binary.build.version,
                    );
                }
                state.last_error = None;
                state.available.clone_from(&available);
                Ok(available)
            }
            Err(e) => {
                state.last_error = Some(e.to_string());
                Err(e)
            }
        }
    }

    pub async fn install(
        &self,
        request: InstallRequest,
        observer: &dyn InstallObserver,
    ) -> Result<Installed, UpdateError> {
        let _claim = Claim::take(&self.busy).ok_or(UpdateError::Busy)?;
        self.run_install(&request, observer).await
    }

    pub fn is_busy(&self) -> bool {
        self.busy.load(Ordering::Acquire)
    }

    pub async fn rollback(&self) -> Result<RolledBack, UpdateError> {
        let _claim = Claim::take(&self.busy).ok_or(UpdateError::Busy)?;
        let result = install::rollback(&self.binary).await;
        let mut state = self.state();
        state.binary_changed = self.binary.exe.changed_on_disk();
        if result.is_ok() {
            state.install = InstallState::Idle;
        }
        drop(state);
        result
    }

    pub fn start_install(&self, request: InstallRequest) -> Result<(), UpdateRequestError> {
        let refused = |reason: String| UpdateRequestError::Refused(reason);
        let Some(service) = self.me.upgrade() else {
            return Err(refused("the update service is shutting down".to_string()));
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return Err(refused("no async runtime is available".to_string()));
        };
        let restart = match (request.then_restart, &self.restart) {
            (false, _) => None,
            (true, Some(restart)) => Some(Arc::clone(restart)),
            (true, None) => {
                return Err(refused(
                    "this proxy cannot restart itself; install without a restart, then restart it by hand"
                        .to_string(),
                ));
            }
        };
        {
            let state = self.state();
            let version = request
                .version
                .as_deref()
                .or_else(|| state.available.as_ref().map(|a| a.version.as_str()));
            if let Some(refusal) = &state.refusal {
                return Err(refused(refusal.instructions(version)));
            }
            if request.version.is_none() {
                let Some(available) = &state.available else {
                    return Err(UpdateRequestError::NothingToInstall);
                };
                if restart.is_some() && available.restart != RestartRequirement::Hot {
                    return Err(refused(format!(
                        "{} needs a full restart; install it without a restart, then restart the proxy",
                        available.version
                    )));
                }
            }
        }
        let claim = OwnedClaim::take(service).ok_or(UpdateRequestError::Busy)?;
        runtime.spawn(async move {
            let service = Arc::clone(&claim.0);
            let result = service.run_install(&request, &LogObserver).await;
            drop(claim);
            match (result, restart) {
                (Ok(installed), Some(restart)) if installed.restart == RestartRequirement::Hot => {
                    if let Err(e) = restart.request_restart() {
                        tracing::warn!(error = %e, "the update is installed but the restart was refused");
                    }
                }
                (Ok(installed), Some(_)) => tracing::warn!(
                    version = %installed.version,
                    "the update is installed but needs a full restart"
                ),
                (Ok(_), None) => {}
                (Err(e), _) => tracing::warn!(error = %e, "update install failed"),
            }
        });
        Ok(())
    }

    pub(super) fn observe_binary(&self) -> bool {
        let changed = self.binary.exe.changed_on_disk();
        self.state().binary_changed = changed;
        changed
    }

    async fn find(&self, channel: UpdateChannel) -> Result<Option<Candidate>, UpdateError> {
        let (source, keys) = self.parts()?;
        let current = install::running_version(&self.binary)?;
        let filter = ChannelFilter::new(channel, &current);
        check::newest(source, keys, filter, &current).await
    }

    async fn run_install(
        &self,
        request: &InstallRequest,
        observer: &dyn InstallObserver,
    ) -> Result<Installed, UpdateError> {
        let label = request
            .version
            .clone()
            .or_else(|| self.state().available.as_ref().map(|a| a.version.clone()))
            .unwrap_or_else(|| "latest".to_string());
        let mut running = Running::start(self, label);
        let result = self.install_with(request, observer).await;
        running.finish(&result);
        result
    }

    async fn install_with(
        &self,
        request: &InstallRequest,
        observer: &dyn InstallObserver,
    ) -> Result<Installed, UpdateError> {
        let target = match &request.version {
            Some(text) => InstallTarget::Version(
                parse_version(text).ok_or_else(|| UpdateError::InvalidVersion(text.clone()))?,
            ),
            None => InstallTarget::Latest(self.config.channel),
        };
        let (source, keys) = self.parts()?;
        let tracking = Tracking {
            service: self,
            inner: observer,
        };
        Installer::new(&self.binary, source, keys)
            .install(target, &tracking)
            .await
    }

    fn parts(&self) -> Result<(&ReleaseSource, &TrustedKeys), UpdateError> {
        let source = self
            .source
            .as_ref()
            .map_err(|reason| UpdateError::InvalidSource {
                url: self.config.url.clone(),
                reason: reason.clone(),
            })?;
        let keys = self.keys.as_ref().ok_or(UpdateError::NoTrustedKey)?;
        Ok((source, keys))
    }

    async fn refusal_now(&self) -> Option<Option<Refusal>> {
        if let Some(refusal) = gate::build_refusal(&self.binary.build) {
            return Some(Some(refusal));
        }
        let paths = match gate::paths_for(self.binary.exe.path()) {
            Ok(paths) => paths,
            Err(refusal) => return Some(Some(refusal)),
        };
        if self.busy.load(Ordering::Acquire) {
            return None;
        }
        match DirLock::acquire(&paths).await {
            Ok(_lock) => Some(gate::check_writable(&paths).err()),
            Err(LockError::Open(e)) => Some(Some(Refusal::NotWritable {
                dir: paths.dir.clone(),
                reason: e.to_string(),
            })),
            Err(LockError::Busy | LockError::Lock(_)) => None,
        }
    }

    fn describe(&self, candidate: &Candidate) -> AvailableUpdate {
        let manifest = candidate.manifest.manifest();
        let successor = HandoverProbe {
            probe: PROBE_SCHEMA,
            version: manifest.version.clone(),
            target: self.binary.build.target.to_string(),
            official: true,
            docker: false,
            handover: manifest.handover.clone(),
        };
        AvailableUpdate {
            version: candidate.version().to_string(),
            published_at: manifest
                .published_at
                .clone()
                .or_else(|| candidate.release.published_at.clone()),
            notes_url: manifest
                .notes_url
                .clone()
                .or_else(|| candidate.release.html_url.clone()),
            security: manifest.security,
            restart: platform::restart_kind(&successor),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn announce(state: &mut State, version: &Version, update: &AvailableUpdate, current: &str) {
    if !state.announced.insert(version.clone()) {
        return;
    }
    let notes = update.notes_url.as_deref().unwrap_or("");
    if update.security {
        tracing::warn!(
            notes,
            "infrarust {} is available (running {current}) and fixes a security issue; run `update install`",
            update.version
        );
    } else {
        tracing::info!(
            notes,
            "infrarust {} is available (running {current}); run `update install`",
            update.version
        );
    }
}

struct Tracking<'a> {
    service: &'a UpdateService,
    inner: &'a dyn InstallObserver,
}

impl InstallObserver for Tracking<'_> {
    fn on_step(&self, step: &InstallStep) {
        if let InstallStep::Resolved { version, .. } = step {
            self.service.state().install = InstallState::Running {
                version: version.to_string(),
            };
        }
        self.inner.on_step(step);
    }

    fn on_download(&self, received: u64, total: u64) {
        self.inner.on_download(received, total);
    }
}

struct Running<'a> {
    service: &'a UpdateService,
    done: bool,
}

impl<'a> Running<'a> {
    fn start(service: &'a UpdateService, version: String) -> Self {
        service.state().install = InstallState::Running { version };
        Self {
            service,
            done: false,
        }
    }

    fn finish(&mut self, result: &Result<Installed, UpdateError>) {
        self.done = true;
        let mut state = self.service.state();
        match result {
            Ok(installed) => {
                state.install = InstallState::Installed {
                    version: installed.version.to_string(),
                };
                state.binary_changed = self.service.binary.exe.changed_on_disk();
            }
            Err(e) => {
                state.install = InstallState::Failed {
                    reason: e.to_string(),
                };
                if let UpdateError::Refused { refusal, .. } = e
                    && !matches!(refusal, Refusal::UnsupportedTarget { .. })
                {
                    state.refusal = Some(refusal.clone());
                }
            }
        }
    }
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        if !self.done {
            self.service.state().install = InstallState::Failed {
                reason: "the installation was interrupted".to_string(),
            };
        }
    }
}

struct Claim<'a>(&'a AtomicBool);

impl<'a> Claim<'a> {
    fn take(flag: &'a AtomicBool) -> Option<Self> {
        flag.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self(flag))
    }
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

struct OwnedClaim(Arc<UpdateService>);

impl OwnedClaim {
    fn take(service: Arc<UpdateService>) -> Option<Self> {
        service
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self(service))
    }
}

impl Drop for OwnedClaim {
    fn drop(&mut self) {
        self.0.busy.store(false, Ordering::Release);
    }
}

#[cfg(all(test, unix))]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::ffi::OsString;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    use infrarust_api::services::{ProxyUpdates, RestartRequestError};

    use crate::binary::{BuildInfo, ExeSnapshot};

    use super::super::install::NoopObserver;
    use super::super::testing::{
        Entry, FakeBinary, HttpServer, Reply, SigningKey, TEST_TARGET, tar_gz, write_executable,
    };
    use super::*;

    const BUILD: BuildInfo = BuildInfo {
        version: "2.0.0-beta.5",
        target: TEST_TARGET,
        official: true,
        docker: false,
    };

    #[derive(Default)]
    struct FakeRestart(AtomicUsize);

    impl infrarust_api::__sealed::Sealed for FakeRestart {}

    impl ProxyRestart for FakeRestart {
        fn request_restart_with(
            &self,
            _options: infrarust_api::services::RestartOptions,
        ) -> Result<(), RestartRequestError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct Fixture {
        dir: tempfile::TempDir,
        server: HttpServer,
        service: Arc<UpdateService>,
        new_script: Vec<u8>,
    }

    impl Fixture {
        async fn new(build: BuildInfo, restart: Option<Arc<dyn ProxyRestart>>) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let exe = dir.path().join("infrarust");
            write_executable(&exe, &FakeBinary::new("2.0.0-beta.5").script());
            let key = SigningKey::generate();
            let server = HttpServer::routed().await;
            let new_script = FakeBinary::new("2.0.0-beta.6").script();
            let archive = tar_gz(&[Entry::file("infrarust", &new_script)]);
            let release = server.publish_signed(&key, "2.0.0-beta.6", &archive);
            server.list(&[release]);
            let binary = RunningBinary::new(
                build,
                ExeSnapshot::at(exe),
                vec![OsString::from("-c"), OsString::from("proxy.toml")],
            );
            let config = UpdateConfig {
                check: false,
                url: server.url("/releases"),
                ..UpdateConfig::default()
            };
            let keys = TrustedKeys::from_strs([key.public()]).unwrap();
            let service = UpdateService::with_keys(config, binary, restart, keys);
            Self {
                dir,
                server,
                service,
                new_script,
            }
        }

        fn exe(&self) -> Vec<u8> {
            std::fs::read(self.dir.path().join("infrarust")).unwrap()
        }

        async fn installed(&self) -> UpdateStatus {
            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            loop {
                let status = self.service.status();
                if !matches!(
                    status.install,
                    InstallState::Running { .. } | InstallState::Idle
                ) {
                    return status;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "the install never finished"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    }

    #[tokio::test]
    async fn status_reads_the_cache_and_a_check_fills_it() {
        let f = Fixture::new(BUILD, None).await;
        let before = f.service.status();
        assert_eq!(before.current_version, "2.0.0-beta.5");
        assert!(!before.checks_enabled);
        assert!(before.last_check.is_none());
        assert!(before.available.is_none());
        assert_eq!(before.install, InstallState::Idle);
        assert!(f.server.requests().is_empty());

        let found = f.service.check(None).await.unwrap().unwrap();
        assert_eq!(found.version, "2.0.0-beta.6");
        assert_eq!(found.restart, RestartRequirement::Full);
        assert!(!found.security);
        assert_eq!(
            found.notes_url.as_deref(),
            Some("https://example.invalid/notes/2.0.0-beta.6")
        );
        let after = f.service.status();
        assert_eq!(after.available, Some(found));
        assert!(after.last_check.is_some());
        assert!(after.last_error.is_none());
        assert!(after.install_refusal.is_none());
        assert_eq!(
            std::fs::read_dir(f.dir.path()).unwrap().count(),
            2,
            "only the exe and the lock file"
        );
    }

    #[tokio::test]
    async fn the_stable_channel_override_hides_the_beta() {
        let f = Fixture::new(BUILD, None).await;
        assert!(
            f.service
                .check(Some(UpdateChannel::Stable))
                .await
                .unwrap()
                .is_none()
        );
        assert!(f.service.status().available.is_none());
    }

    #[tokio::test]
    async fn an_unofficial_build_reports_how_to_update_it() {
        let f = Fixture::new(
            BuildInfo {
                official: false,
                ..BUILD
            },
            None,
        )
        .await;
        assert!(
            f.service
                .status()
                .install_refusal
                .unwrap()
                .contains("cargo install infrarust`")
        );
        f.service.check(None).await.unwrap();
        let refusal = f.service.status().install_refusal.unwrap();
        assert!(
            refusal.contains("cargo install infrarust --version 2.0.0-beta.6"),
            "{refusal}"
        );
        assert!(matches!(
            f.service.start_install(InstallRequest::default()),
            Err(UpdateRequestError::Refused(_))
        ));
    }

    #[tokio::test]
    async fn errors_land_in_the_status() {
        let f = Fixture::new(BUILD, None).await;
        f.server.route(
            "/releases",
            Reply::status(403)
                .with_header("x-ratelimit-remaining", "0")
                .with_header("x-ratelimit-reset", "1700000000"),
        );
        assert!(matches!(
            f.service.check(None).await,
            Err(UpdateError::RateLimited { .. })
        ));
        let status = f.service.status();
        assert!(status.last_error.unwrap().contains("rate limit"));
        assert!(status.last_check.is_some());

        let keyless = UpdateService::build(
            UpdateConfig::default(),
            Arc::clone(&f.service.binary),
            None,
            None,
        );
        assert!(matches!(
            keyless.check(None).await,
            Err(UpdateError::NoTrustedKey)
        ));
        assert!(keyless.status().last_error.is_some());
    }

    #[tokio::test]
    async fn start_install_needs_an_update_or_a_version() {
        let f = Fixture::new(BUILD, None).await;
        assert_eq!(
            f.service.start_install(InstallRequest::default()),
            Err(UpdateRequestError::NothingToInstall)
        );
    }

    #[tokio::test]
    async fn a_restart_request_needs_a_restart_handle() {
        let f = Fixture::new(BUILD, None).await;
        f.service.check(None).await.unwrap();
        let request = InstallRequest {
            version: None,
            then_restart: true,
        };
        assert!(matches!(
            f.service.start_install(request),
            Err(UpdateRequestError::Refused(_))
        ));
        assert_eq!(f.service.status().install, InstallState::Idle);
    }

    #[tokio::test]
    async fn a_restart_request_needs_a_hot_update() {
        let restart = Arc::new(FakeRestart::default());
        let handle: Arc<dyn ProxyRestart> = restart.clone();
        let f = Fixture::new(BUILD, Some(handle)).await;
        f.service.check(None).await.unwrap();
        let request = InstallRequest {
            version: None,
            then_restart: true,
        };
        match f.service.start_install(request) {
            Err(UpdateRequestError::Refused(reason)) => {
                assert!(reason.contains("full restart"), "{reason}");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
        assert_eq!(restart.0.load(Ordering::SeqCst), 0);
        assert_eq!(f.exe(), FakeBinary::new("2.0.0-beta.5").script());
    }

    #[tokio::test]
    async fn start_install_is_single_flight_and_installs_in_the_background() {
        let f = Fixture::new(BUILD, None).await;
        f.service.check(None).await.unwrap();
        f.service.start_install(InstallRequest::default()).unwrap();
        assert_eq!(
            f.service.start_install(InstallRequest::default()),
            Err(UpdateRequestError::Busy)
        );
        assert!(matches!(
            f.service
                .install(InstallRequest::default(), &NoopObserver)
                .await,
            Err(UpdateError::Busy)
        ));

        let status = f.installed().await;
        assert_eq!(
            status.install,
            InstallState::Installed {
                version: "2.0.0-beta.6".to_string()
            }
        );
        assert!(status.binary_changed);
        assert_eq!(f.exe(), f.new_script);

        f.service.rollback().await.unwrap();
        let status = f.service.status();
        assert_eq!(status.install, InstallState::Idle);
        assert!(!status.binary_changed);
        assert_eq!(f.exe(), FakeBinary::new("2.0.0-beta.5").script());
    }

    #[tokio::test]
    async fn a_failed_install_is_reported() {
        let f = Fixture::new(BUILD, None).await;
        let result = f
            .service
            .install(
                InstallRequest {
                    version: Some("v9.9.9".to_string()),
                    then_restart: false,
                },
                &NoopObserver,
            )
            .await;
        assert!(
            matches!(result, Err(UpdateError::VersionNotFound { .. })),
            "{result:?}"
        );
        assert!(matches!(
            f.service.status().install,
            InstallState::Failed { .. }
        ));
        assert!(matches!(
            f.service
                .install(
                    InstallRequest {
                        version: Some("latest".to_string()),
                        then_restart: false,
                    },
                    &NoopObserver,
                )
                .await,
            Err(UpdateError::InvalidVersion(_))
        ));
    }

    #[tokio::test]
    async fn a_pinned_install_goes_through_the_service() {
        let f = Fixture::new(BUILD, None).await;
        let installed = f
            .service
            .install(
                InstallRequest {
                    version: Some("2.0.0-beta.6".to_string()),
                    then_restart: false,
                },
                &NoopObserver,
            )
            .await
            .unwrap();
        assert_eq!(installed.version.to_string(), "2.0.0-beta.6");
        assert_eq!(f.exe(), f.new_script);
    }

    #[tokio::test]
    async fn a_dropped_check_now_leaves_the_status_and_cache_consistent() {
        let f = Fixture::new(BUILD, None).await;
        let first = f.service.check_now().await;
        assert_eq!(first.available.as_ref().unwrap().version, "2.0.0-beta.6");

        let list = f.server.reply("/releases").unwrap();
        f.server.route("/releases", Reply::stall());
        let dropped = tokio::time::timeout(Duration::from_millis(200), f.service.check_now()).await;
        assert!(dropped.is_err());
        assert_eq!(f.service.status(), first);
        f.server.route("/releases", list);

        let manifest_path = "/assets/v2.0.0-beta.6/manifest.json";
        let manifest = f.server.reply(manifest_path).unwrap();
        f.server.route(manifest_path, Reply::stall());
        let dropped = tokio::time::timeout(Duration::from_millis(200), f.service.check_now()).await;
        assert!(dropped.is_err());
        assert_eq!(f.service.status(), first);
        f.server.route(manifest_path, manifest);

        let again = tokio::time::timeout(Duration::from_secs(5), f.service.check_now())
            .await
            .expect("a later check still completes");
        assert_eq!(again.available, first.available);
        assert!(again.last_error.is_none());
        assert!(again.last_check >= first.last_check);
        let lists: Vec<_> = f
            .server
            .requests()
            .into_iter()
            .filter(|r| r.path == "/releases")
            .collect();
        assert_eq!(lists.len(), 4);
        assert!(lists[0].header("if-none-match").is_none());
        assert!(
            lists[1..]
                .iter()
                .all(|r| r.header("if-none-match") == Some("\"releases\""))
        );
    }
}

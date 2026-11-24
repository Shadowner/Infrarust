use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use infrarust_api::BoxFuture;
use infrarust_api::services::{
    AvailableUpdate, InstallRequest, InstallState, ProxyUpdates, RestartRequirement,
    UpdateRequestError, UpdateStatus,
};

use crate::util::sync::lock;

const INSTALL_TIME: Duration = Duration::from_millis(10);

pub struct FakeUpdates {
    status: Arc<Mutex<UpdateStatus>>,
    on_check: Mutex<Option<UpdateStatus>>,
    outcome: Mutex<Result<InstallState, UpdateRequestError>>,
    requests: Mutex<Vec<InstallRequest>>,
    checks: AtomicUsize,
}

impl FakeUpdates {
    #[must_use]
    pub fn new(status: UpdateStatus) -> Self {
        Self {
            status: Arc::new(Mutex::new(status)),
            on_check: Mutex::new(None),
            outcome: Mutex::new(Err(UpdateRequestError::NothingToInstall)),
            requests: Mutex::new(Vec::new()),
            checks: AtomicUsize::new(0),
        }
    }

    #[must_use]
    pub fn idle(current_version: &str) -> UpdateStatus {
        UpdateStatus {
            current_version: current_version.to_string(),
            checks_enabled: true,
            last_check: Some(SystemTime::now()),
            last_error: None,
            available: None,
            install: InstallState::Idle,
            install_refusal: None,
            binary_changed: false,
        }
    }

    #[must_use]
    pub fn release(version: &str, security: bool) -> AvailableUpdate {
        AvailableUpdate {
            version: version.to_string(),
            published_at: None,
            notes_url: None,
            security,
            restart: RestartRequirement::Full,
        }
    }

    #[must_use]
    pub fn on_check(self, status: UpdateStatus) -> Self {
        *lock(&self.on_check) = Some(status);
        self
    }

    #[must_use]
    pub fn installs(self, outcome: InstallState) -> Self {
        *lock(&self.outcome) = Ok(outcome);
        self
    }

    #[must_use]
    pub fn refuses(self, error: UpdateRequestError) -> Self {
        *lock(&self.outcome) = Err(error);
        self
    }

    pub fn checks(&self) -> usize {
        self.checks.load(Ordering::SeqCst)
    }

    pub fn requests(&self) -> Vec<InstallRequest> {
        lock(&self.requests).clone()
    }
}

impl infrarust_api::__sealed::Sealed for FakeUpdates {}

impl ProxyUpdates for FakeUpdates {
    fn status(&self) -> UpdateStatus {
        lock(&self.status).clone()
    }

    fn check_now(&self) -> BoxFuture<'_, UpdateStatus> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        if let Some(found) = lock(&self.on_check).take() {
            *lock(&self.status) = found;
        }
        let status = self.status();
        Box::pin(async move { status })
    }

    fn start_install(&self, request: InstallRequest) -> Result<(), UpdateRequestError> {
        lock(&self.requests).push(request.clone());
        let outcome = lock(&self.outcome).clone()?;
        let version = request
            .version
            .or_else(|| self.status().available.map(|update| update.version))
            .unwrap_or_else(|| "latest".to_string());
        lock(&self.status).install = InstallState::Running { version };
        let status = Arc::clone(&self.status);
        tokio::spawn(async move {
            tokio::time::sleep(INSTALL_TIME).await;
            let mut status = lock(&status);
            status.binary_changed = matches!(outcome, InstallState::Installed { .. });
            status.install = outcome;
        });
        Ok(())
    }
}

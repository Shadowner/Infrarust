use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use infrarust_api::error::ServiceError;
use infrarust_api::event::BoxFuture;
use infrarust_api::events::ban::{BanIssuedEvent, BanRevokedEvent};
use infrarust_api::events::handshake::RejectReason;
use infrarust_api::player::Player;
use infrarust_api::services::ban_service::{
    BanFeatures, BanPage, BanProvider, BanProviderRejected, BanQuery, BanRequest, BanService,
    BanVerdict, LoginAttempt, LoginStage, UnbanRequest,
};
use infrarust_api::types::Component;
use infrarust_config::{BanConfig, BanProviderSelection};

use crate::ban::builtin::BuiltinBanProvider;
use crate::ban::file_storage::FileBanStorage;
use crate::ban::storage::BanStorage;
use crate::ban::types::{BanEntry, BanSource, BanTarget};
use crate::error::CoreError;
use crate::event_bus::EventBusImpl;
use crate::player::PlayerSession;
use crate::registry::ConnectionRegistry;

pub const BAN_CHECK_UNAVAILABLE: &str =
    "Your ban status cannot be checked right now. Please try again later.";

#[derive(Debug, Clone)]
pub struct Refusal {
    pub message: Component,
    pub reason: RejectReason,
}

#[derive(Debug, Clone)]
pub struct IssuedBan {
    pub entry: BanEntry,
    pub kicked: usize,
}

pub struct BanManager {
    selection: BanProviderSelection,
    builtin: Option<Arc<BuiltinBanProvider>>,
    registered: RwLock<Option<Arc<dyn BanProvider>>>,
    connection_registry: Arc<ConnectionRegistry>,
    event_bus: Arc<EventBusImpl>,
}

impl BanManager {
    fn with_selection(
        selection: BanProviderSelection,
        builtin: Option<Arc<BuiltinBanProvider>>,
        connection_registry: Arc<ConnectionRegistry>,
        event_bus: Arc<EventBusImpl>,
    ) -> Self {
        Self {
            selection,
            builtin,
            registered: RwLock::new(None),
            connection_registry,
            event_bus,
        }
    }

    pub fn builtin(
        storage: Arc<dyn BanStorage>,
        connection_registry: Arc<ConnectionRegistry>,
        event_bus: Arc<EventBusImpl>,
    ) -> Self {
        Self::with_selection(
            BanProviderSelection::Builtin,
            Some(Arc::new(BuiltinBanProvider::new(storage))),
            connection_registry,
            event_bus,
        )
    }

    pub fn plugin(
        plugin_id: impl Into<String>,
        connection_registry: Arc<ConnectionRegistry>,
        event_bus: Arc<EventBusImpl>,
    ) -> Self {
        Self::with_selection(
            BanProviderSelection::Plugin(plugin_id.into()),
            None,
            connection_registry,
            event_bus,
        )
    }

    pub fn disabled(
        connection_registry: Arc<ConnectionRegistry>,
        event_bus: Arc<EventBusImpl>,
    ) -> Self {
        Self::with_selection(
            BanProviderSelection::Disabled,
            None,
            connection_registry,
            event_bus,
        )
    }

    pub async fn from_config(
        config: &BanConfig,
        connection_registry: Arc<ConnectionRegistry>,
        event_bus: Arc<EventBusImpl>,
    ) -> Result<Self, CoreError> {
        let manager = match &config.provider {
            BanProviderSelection::Builtin => Self::builtin(
                Arc::new(FileBanStorage::new(config.file.clone())),
                connection_registry,
                event_bus,
            ),
            BanProviderSelection::Disabled => {
                tracing::warn!("ban checks are disabled ([ban] provider = \"none\")");
                Self::disabled(connection_registry, event_bus)
            }
            BanProviderSelection::Plugin(id) => {
                tracing::info!(plugin = %id, "bans are delegated to a plugin provider");
                Self::plugin(id.clone(), connection_registry, event_bus)
            }
        };
        if let Some(builtin) = &manager.builtin {
            builtin.load().await?;
        }
        Ok(manager)
    }

    pub const fn selection(&self) -> &BanProviderSelection {
        &self.selection
    }

    pub fn builtin_provider(&self) -> Option<&Arc<BuiltinBanProvider>> {
        self.builtin.as_ref()
    }

    fn active(&self) -> Result<Option<Arc<dyn BanProvider>>, ServiceError> {
        match &self.selection {
            BanProviderSelection::Builtin => Ok(self
                .builtin
                .as_ref()
                .map(|builtin| Arc::clone(builtin) as Arc<dyn BanProvider>)),
            BanProviderSelection::Disabled => Ok(None),
            BanProviderSelection::Plugin(id) => self
                .registered
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
                .map(Some)
                .ok_or_else(|| {
                    ServiceError::Unavailable(format!(
                        "ban provider plugin `{id}` has not registered a provider"
                    ))
                }),
        }
    }

    fn managed(&self) -> Result<Arc<dyn BanProvider>, ServiceError> {
        self.active()?.ok_or_else(|| {
            ServiceError::Unavailable("bans are disabled ([ban] provider = \"none\")".to_string())
        })
    }

    pub async fn check(&self, attempt: &LoginAttempt) -> Result<Option<BanVerdict>, ServiceError> {
        let Some(provider) = self.active()? else {
            return Ok(None);
        };
        let canonical = attempt.ip.to_canonical();
        if canonical == attempt.ip {
            return provider.check(attempt).await;
        }
        let mut attempt = attempt.clone();
        attempt.ip = canonical;
        provider.check(&attempt).await
    }

    pub async fn refusal(&self, attempt: &LoginAttempt) -> Option<Component> {
        self.refuse(attempt).await.map(|refusal| refusal.message)
    }

    pub async fn refuse(&self, attempt: &LoginAttempt) -> Option<Refusal> {
        match self.check(attempt).await {
            Ok(None) => None,
            Ok(Some(verdict)) => {
                tracing::info!(
                    stage = ?attempt.stage,
                    ip = %attempt.ip,
                    username = attempt.username.as_deref().unwrap_or("-"),
                    ban = %verdict.entry.id,
                    ban_target = %verdict.entry.target,
                    "connection refused: banned"
                );
                let reason = match verdict.entry.target {
                    BanTarget::Ip(_) | BanTarget::IpRange(_) => RejectReason::IpBanned,
                    _ => RejectReason::Banned,
                };
                Some(Refusal {
                    message: verdict.kick_message,
                    reason,
                })
            }
            Err(e) if attempt.stage == LoginStage::Status => {
                tracing::warn!(ip = %attempt.ip, error = %e, "could not check bans for a status request, answering it");
                None
            }
            Err(e) => {
                tracing::error!(
                    stage = ?attempt.stage,
                    ip = %attempt.ip,
                    username = attempt.username.as_deref().unwrap_or("-"),
                    error = %e,
                    "could not check bans, refusing the login"
                );
                Some(Refusal {
                    message: Component::text(BAN_CHECK_UNAVAILABLE),
                    reason: RejectReason::Banned,
                })
            }
        }
    }

    pub async fn issue(&self, request: BanRequest) -> Result<IssuedBan, ServiceError> {
        let provider = self.managed()?;
        let mut request = request;
        request.target = request.target.canonical();
        if matches!(request.target, BanTarget::IpRange(_)) && !provider.features().ip_ranges {
            return Err(ServiceError::OperationFailed(
                "the active ban provider does not support IP ranges".to_string(),
            ));
        }
        let source = request.source.get_or_insert(BanSource::System).clone();
        let (kick, silent) = (request.kick, request.silent);
        let entry = provider.ban(request).await?;
        self.event_bus
            .post(BanIssuedEvent::new(entry.clone(), source, silent));
        let kicked = if kick {
            self.kick_matching(provider.as_ref(), &entry).await
        } else {
            0
        };
        Ok(IssuedBan { entry, kicked })
    }

    pub async fn revoke(&self, request: UnbanRequest) -> Result<Option<BanEntry>, ServiceError> {
        let provider = self.managed()?;
        let mut request = request;
        request.target = request.target.canonical();
        let source = request.source.get_or_insert(BanSource::System).clone();
        let silent = request.silent;
        let removed = provider.unban(request).await?;
        if let Some(entry) = &removed {
            self.event_bus
                .post(BanRevokedEvent::new(entry.clone(), source, silent));
        }
        Ok(removed)
    }

    pub async fn get(&self, target: &BanTarget) -> Result<Option<BanEntry>, ServiceError> {
        let provider = self.managed()?;
        provider.get(&target.clone().canonical()).await
    }

    pub async fn list(&self, query: BanQuery) -> Result<BanPage, ServiceError> {
        self.managed()?.list(query).await
    }

    pub fn features(&self) -> BanFeatures {
        self.active()
            .ok()
            .flatten()
            .map_or_else(BanFeatures::new, |provider| provider.features())
    }

    async fn kick_matching(&self, provider: &dyn BanProvider, entry: &BanEntry) -> usize {
        let sessions: Vec<(Arc<PlayerSession>, LoginAttempt)> = self
            .connection_registry
            .all()
            .into_iter()
            .map(|session| {
                let attempt = session_attempt(&session);
                (session, attempt)
            })
            .filter(|(_, attempt)| entry.target.matches(attempt))
            .collect();

        for (session, attempt) in &sessions {
            let message = match provider.check(attempt).await {
                Ok(Some(verdict)) => verdict.kick_message,
                _ => BanVerdict::new(entry.clone()).kick_message,
            };
            tracing::info!(
                ban = %entry.id,
                ban_target = %entry.target,
                username = %session.profile().username,
                "kicking a connected player due to a ban"
            );
            session.disconnect(message).await;
        }
        sessions.len()
    }

    pub fn register_provider(
        &self,
        plugin_id: &str,
        provider: Arc<dyn BanProvider>,
    ) -> Result<(), BanProviderRejected> {
        match &self.selection {
            BanProviderSelection::Plugin(id) if id == plugin_id => {
                *self
                    .registered
                    .write()
                    .unwrap_or_else(PoisonError::into_inner) = Some(provider);
                tracing::info!(plugin = %plugin_id, "ban provider registered, bans now go through it");
                Ok(())
            }
            selected => {
                tracing::warn!(
                    plugin = %plugin_id,
                    selected = %selected,
                    "ignoring a ban provider: [ban] provider selects another one"
                );
                Err(BanProviderRejected::NotSelected {
                    selected: selected.to_string(),
                })
            }
        }
    }

    pub fn unregister_provider(&self, plugin_id: &str) {
        if self.selection.plugin_id() != Some(plugin_id) {
            return;
        }
        let removed = self
            .registered
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if removed.is_some() {
            tracing::error!(
                plugin = %plugin_id,
                "the ban provider plugin went away, logins are refused until it registers again"
            );
        }
    }

    pub fn report_missing_provider(&self) {
        if let BanProviderSelection::Plugin(id) = &self.selection
            && self.active().is_err()
        {
            tracing::error!(
                plugin = %id,
                "[ban] provider names a plugin that registered no ban provider; logins are refused until it does (set provider = \"builtin\" or \"none\" to change this)"
            );
        }
    }

    pub fn start_purge_task(
        &self,
        interval: Duration,
        shutdown: CancellationToken,
    ) -> Option<tokio::task::JoinHandle<()>> {
        self.builtin
            .as_ref()
            .map(|builtin| builtin.start_purge_task(interval, shutdown))
    }
}

fn session_attempt(session: &PlayerSession) -> LoginAttempt {
    let profile = session.profile();
    let attempt = LoginAttempt::post_auth(
        session.remote_addr().ip(),
        profile.username.clone(),
        profile.uuid,
        session.is_online_mode(),
    );
    match session.current_server() {
        Some(server) => attempt.server(server),
        None => attempt,
    }
}

impl infrarust_api::services::ban_service::private::Sealed for BanManager {}

impl BanService for BanManager {
    fn check<'a>(
        &'a self,
        attempt: &'a LoginAttempt,
    ) -> BoxFuture<'a, Result<Option<BanVerdict>, ServiceError>> {
        Box::pin(self.check(attempt))
    }

    fn ban(&self, request: BanRequest) -> BoxFuture<'_, Result<BanEntry, ServiceError>> {
        Box::pin(async move { self.issue(request).await.map(|issued| issued.entry) })
    }

    fn unban(
        &self,
        request: UnbanRequest,
    ) -> BoxFuture<'_, Result<Option<BanEntry>, ServiceError>> {
        Box::pin(self.revoke(request))
    }

    fn get<'a>(
        &'a self,
        target: &'a BanTarget,
    ) -> BoxFuture<'a, Result<Option<BanEntry>, ServiceError>> {
        Box::pin(self.get(target))
    }

    fn list(&self, query: BanQuery) -> BoxFuture<'_, Result<BanPage, ServiceError>> {
        Box::pin(self.list(query))
    }

    fn features(&self) -> BanFeatures {
        self.features()
    }
}

//! Premium player detection via Mojang API lookups.

use std::sync::Arc;

use infrarust_api::event::{BoxFuture, ResultedEvent};
use infrarust_api::events::lifecycle::{PreLoginEvent, PreLoginResult};
use infrarust_api::types::Component;

use crate::storage::AuthStorage;

use super::cache::{PremiumCache, PremiumStatus};
use super::config::{PremiumConfig, RateLimitAction};
use super::lookup::{LookupError, MojangApiLookup};

pub struct PremiumDetector {
    cache: Arc<PremiumCache>,
    lookup: Arc<MojangApiLookup>,
    storage: Arc<dyn AuthStorage>,
    config: Arc<PremiumConfig>,
}

impl PremiumDetector {
    pub fn new(
        cache: Arc<PremiumCache>,
        lookup: Arc<MojangApiLookup>,
        storage: Arc<dyn AuthStorage>,
        config: Arc<PremiumConfig>,
    ) -> Self {
        Self {
            cache,
            lookup,
            storage,
            config,
        }
    }

    pub fn pre_login_handler(
        self: &Arc<Self>,
    ) -> impl Fn(&mut PreLoginEvent) -> BoxFuture<'_, ()> + Send + Sync + 'static {
        let detector = Arc::clone(self);
        move |event: &mut PreLoginEvent| {
            let detector = Arc::clone(&detector);
            let username = event.profile.username.clone();
            Box::pin(async move {
                let canonical = crate::account::Username::new(&username);
                if detector.storage.is_force_cracked_blocking(&canonical) {
                    tracing::debug!(%username, "Skipping premium check: force_cracked");
                    return;
                }

                if detector.cache.is_auth_failed(&username) {
                    event.set_result(PreLoginResult::ForceOffline);
                    tracing::debug!(%username, "Recent auth failure — ForceOffline");
                    return;
                }

                if let Some(status) = detector.cache.get(&username) {
                    match status {
                        PremiumStatus::Premium { .. } => {
                            event.set_result(PreLoginResult::ForceOnline);
                            tracing::debug!(%username, "Premium (cached) — ForceOnline");
                        }
                        PremiumStatus::Cracked => {
                            tracing::debug!(%username, "Cracked (cached) — Allowed");
                        }
                    }
                    return;
                }

                match detector.lookup.lookup_username(&username).await {
                    Ok(Some(mojang_uuid)) => {
                        detector
                            .cache
                            .put(&username, PremiumStatus::Premium { mojang_uuid });
                        event.set_result(PreLoginResult::ForceOnline);
                        tracing::info!(%username, %mojang_uuid, "Premium detected — ForceOnline");
                    }
                    Ok(None) => {
                        detector.cache.put(&username, PremiumStatus::Cracked);
                        tracing::debug!(%username, "Not premium — Allowed");
                    }
                    Err(LookupError::RateLimited) => match detector.config.rate_limit_action {
                        RateLimitAction::AllowOffline => {
                            tracing::warn!(%username, "Mojang API rate limited — fail-open");
                        }
                        RateLimitAction::Deny => {
                            tracing::warn!(%username, "Mojang API rate limited — denying");
                            event.deny(Component::error(&detector.config.messages.rate_limited));
                        }
                    },
                    Err(e) => {
                        tracing::warn!(%username, error = %e, "Mojang API error — fail-open");
                    }
                }
            })
        }
    }
}

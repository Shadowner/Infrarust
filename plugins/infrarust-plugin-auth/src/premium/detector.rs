//! Premium player detection via Mojang API lookups.

use std::sync::Arc;

use infrarust_api::event::{BoxFuture, ResultedEvent};
use infrarust_api::events::lifecycle::{PreLoginEvent, PreLoginResult};

use crate::storage::AuthStorage;
use crate::util::parse_colored;

use super::cache::{FailedAuth, PremiumCache, PremiumStatus};
use super::config::{NameConflictAction, PremiumConfig, RateLimitAction};
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

                if let Some(failure) = detector.cache.failed_auth(&username) {
                    if failure == FailedAuth::PremiumName
                        && matches!(
                            detector.config.premium_name_conflict_action,
                            NameConflictAction::Kick
                        )
                    {
                        event.deny(parse_colored(
                            &detector.config.messages.premium_name_conflict,
                        ));
                        tracing::info!(
                            %username,
                            "Premium name after a failed online auth — denied"
                        );
                        return;
                    }
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
                            event.deny(parse_colored(&detector.config.messages.rate_limited));
                        }
                    },
                    Err(e) => match detector.config.lookup_error_action {
                        RateLimitAction::AllowOffline => {
                            tracing::warn!(%username, error = %e, "Mojang API error — fail-open");
                        }
                        RateLimitAction::Deny => {
                            tracing::warn!(%username, error = %e, "Mojang API error — denying");
                            event.deny(parse_colored(&detector.config.messages.lookup_failed));
                        }
                    },
                }
            })
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::time::Duration;

    use infrarust_api::types::ProtocolVersion;
    use uuid::Uuid;

    use super::*;
    use crate::test_support::{TestEnv, flatten, profile};

    struct Fixture {
        detector: Arc<PremiumDetector>,
        cache: Arc<PremiumCache>,
        _env: TestEnv,
    }

    async fn fixture(action: Option<NameConflictAction>) -> Fixture {
        let mut config = PremiumConfig {
            enabled: true,
            ..PremiumConfig::default()
        };
        if let Some(action) = action {
            config.premium_name_conflict_action = action;
        }
        fixture_with(config, MojangApiLookup::new(1)).await
    }

    async fn fixture_with(config: PremiumConfig, lookup: MojangApiLookup) -> Fixture {
        let env = TestEnv::new().await;
        let cache = Arc::new(PremiumCache::new(
            Duration::from_secs(60),
            Duration::from_secs(60),
        ));
        let detector = Arc::new(PremiumDetector::new(
            Arc::clone(&cache),
            Arc::new(lookup),
            Arc::clone(&env.storage),
            Arc::new(config),
        ));
        Fixture {
            detector,
            cache,
            _env: env,
        }
    }

    impl Fixture {
        async fn pre_login(&self, username: &str) -> PreLoginResult {
            let mut event = PreLoginEvent::new(
                profile(1, username),
                "127.0.0.1:40000".parse().unwrap(),
                ProtocolVersion::MINECRAFT_1_21,
                "lobby.test".to_string(),
            );
            let handler = self.detector.pre_login_handler();
            handler(&mut event).await;
            event.result().clone()
        }

        fn premium(&self, username: &str) {
            self.cache.put(
                username,
                PremiumStatus::Premium {
                    mojang_uuid: Uuid::nil(),
                },
            );
        }
    }

    async fn mojang_answering(status: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut request = [0u8; 1024];
                let _ = tokio::io::AsyncReadExt::read(&mut stream, &mut request).await;
                let response =
                    format!("HTTP/1.1 {status}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
                let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, response.as_bytes()).await;
            }
        });
        format!("http://{addr}")
    }

    async fn denial(config: PremiumConfig, status: &'static str) -> String {
        let lookup = MojangApiLookup::with_base_url(mojang_answering(status).await, 10);
        let fx = fixture_with(config, lookup).await;
        match fx.pre_login("Steve").await {
            PreLoginResult::Denied { reason } => flatten(&reason),
            other => panic!("the lookup failure should deny the login: {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_rate_limited_lookup_denies_with_its_colours_rendered() {
        let config = PremiumConfig {
            enabled: true,
            rate_limit_action: RateLimitAction::Deny,
            ..PremiumConfig::default()
        };
        assert_eq!(
            denial(config, "429 Too Many Requests").await,
            "The server is busy. Please try again in a moment."
        );
    }

    #[tokio::test]
    async fn a_failed_lookup_denies_with_its_colours_rendered() {
        let config = PremiumConfig {
            enabled: true,
            lookup_error_action: RateLimitAction::Deny,
            ..PremiumConfig::default()
        };
        assert_eq!(
            denial(config, "500 Internal Server Error").await,
            "Could not verify your account status. Please try again later."
        );
    }

    #[tokio::test]
    async fn by_default_a_premium_name_is_refused_after_a_failed_online_auth() {
        let fx = fixture(None).await;
        fx.premium("Hypixel");
        assert!(matches!(
            fx.pre_login("Hypixel").await,
            PreLoginResult::ForceOnline
        ));

        fx.cache.mark_auth_failed("Hypixel");

        match fx.pre_login("Hypixel").await {
            PreLoginResult::Denied { reason } => assert_eq!(
                flatten(&reason),
                "This username belongs to a premium account. Use the official Minecraft launcher."
            ),
            other => panic!("a cracked client must not play under a premium name: {other:?}"),
        }
    }

    #[tokio::test]
    async fn allow_cracked_lets_a_premium_name_log_in_offline_after_a_failed_online_auth() {
        let fx = fixture(Some(NameConflictAction::AllowCracked)).await;
        fx.premium("Hypixel");
        fx.cache.mark_auth_failed("Hypixel");

        assert!(matches!(
            fx.pre_login("Hypixel").await,
            PreLoginResult::ForceOffline
        ));
    }

    #[tokio::test]
    async fn a_name_that_is_not_premium_goes_offline_after_a_failed_online_auth() {
        let fx = fixture(Some(NameConflictAction::Kick)).await;
        fx.cache.put("Steve", PremiumStatus::Cracked);
        fx.cache.mark_auth_failed("Steve");

        assert!(matches!(
            fx.pre_login("Steve").await,
            PreLoginResult::ForceOffline
        ));
    }
}

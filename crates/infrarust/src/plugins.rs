//! Static plugin registration via Cargo features.

use infrarust_config::{ApiKeyError, ApiKeyState, WebConfig};
use infrarust_core::plugin::StaticPluginLoader;
use infrarust_plugin_admin_api::log_layer::LogBroadcast;

pub fn build_static_loader(
    web_config: Option<&WebConfig>,
    logs: Option<LogBroadcast>,
) -> anyhow::Result<StaticPluginLoader> {
    let loader = StaticPluginLoader::new();

    #[cfg(feature = "plugin-auth")]
    {
        use infrarust_api::plugin::Plugin;
        let auth = infrarust_plugin_auth::AuthPlugin::default();
        loader.register(auth.metadata(), || {
            Box::new(infrarust_plugin_auth::AuthPlugin::default())
        });
    }

    #[cfg(feature = "plugin-hello")]
    {
        use infrarust_api::plugin::Plugin;
        let hello = infrarust_plugin_hello::HelloPlugin;
        loader.register(hello.metadata(), || {
            Box::new(infrarust_plugin_hello::HelloPlugin)
        });
    }

    #[cfg(feature = "plugin-server-wake")]
    {
        use infrarust_api::plugin::Plugin;
        let wake = infrarust_plugin_server_wake::ServerWakePlugin::default();
        loader.register(wake.metadata(), || {
            Box::new(infrarust_plugin_server_wake::ServerWakePlugin::default())
        });
    }

    if let Some(web) = web_config.filter(|w| w.enable_api) {
        use infrarust_api::plugin::Plugin;
        use infrarust_plugin_admin_api::config::{ApiConfig, RateLimitConfig};

        let api_key = admin_api_key(web)
            .map_err(|e| anyhow::anyhow!("Invalid web API configuration: {e}"))?;

        let enable_webui = web.webui_enabled();

        let config = ApiConfig {
            bind: web.bind.clone(),
            api_key,
            cors_origins: web.cors_origins.clone(),
            rate_limit: RateLimitConfig {
                requests_per_minute: web.rate_limit.requests_per_minute,
            },
        };

        let admin_api = infrarust_plugin_admin_api::AdminApiPlugin::new(
            config.clone(),
            enable_webui,
            logs.clone(),
        );
        loader.register(admin_api.metadata(), move || {
            Box::new(infrarust_plugin_admin_api::AdminApiPlugin::new(
                config.clone(),
                enable_webui,
                logs.clone(),
            ))
        });
    }

    tracing::info!(
        count = loader.registered_count(),
        "Static plugins registered"
    );
    Ok(loader)
}

fn admin_api_key(web: &WebConfig) -> Result<String, ApiKeyError> {
    match web.api_key_state()? {
        ApiKeyState::Usable(key) => Ok(key),
        ApiKeyState::Redacted => Err(ApiKeyError::Redacted),
        ApiKeyState::MissingOnLoopback => Ok(ephemeral_api_key(&web.bind)),
    }
}

fn ephemeral_api_key(bind: &str) -> String {
    let key = uuid::Uuid::new_v4().to_string();
    tracing::warn!(
        "No API key configured for loopback bind ({bind}) — generated an ephemeral key, \
         printed once on stdout. Set `api_key` in the [web] section to keep it across restarts."
    );
    println!(
        "[web] ephemeral API key for {bind}: {key}\n\
         It changes on every restart and is not written to the configuration file."
    );
    key
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn web(enable_api: bool) -> WebConfig {
        WebConfig {
            enable_api,
            enable_webui: Some(enable_api),
            ..WebConfig::default()
        }
    }

    #[test]
    fn admin_api_is_registered_when_the_api_is_enabled() {
        let loader =
            build_static_loader(Some(&web(true)), None).expect("loopback bind is accepted");
        assert!(loader.registered_ids().contains(&"admin_api".to_string()));
    }

    #[test]
    fn admin_api_is_not_registered_when_the_api_is_disabled() {
        let loader = build_static_loader(Some(&web(false)), None).expect("nothing to resolve");
        assert!(!loader.registered_ids().contains(&"admin_api".to_string()));
    }

    #[test]
    fn a_loopback_bind_without_a_key_gets_a_fresh_ephemeral_one() {
        let config = web(true);
        let first = admin_api_key(&config).expect("loopback may mint a key");
        let second = admin_api_key(&config).expect("loopback may mint a key");
        assert!(!first.is_empty());
        assert_ne!(first, second);
    }

    #[test]
    fn a_configured_key_is_used_as_is() {
        let config = WebConfig {
            api_key: Some("a-sufficiently-long-api-key-value".to_string()),
            ..web(true)
        };
        assert_eq!(
            admin_api_key(&config).as_deref(),
            Ok("a-sufficiently-long-api-key-value")
        );
    }

    #[test]
    fn the_redaction_placeholder_and_an_exposed_bind_without_key_are_refused() {
        let redacted = WebConfig {
            api_key: Some(infrarust_config::secrets::REDACTED.to_string()),
            ..web(true)
        };
        assert_eq!(admin_api_key(&redacted), Err(ApiKeyError::Redacted));

        let exposed = WebConfig {
            bind: "0.0.0.0:8080".to_string(),
            ..web(true)
        };
        assert!(build_static_loader(Some(&exposed), None).is_err());
    }

    #[test]
    fn admin_api_is_not_registered_without_a_web_section() {
        let loader = build_static_loader(None, None).expect("nothing to resolve");
        assert!(!loader.registered_ids().contains(&"admin_api".to_string()));
    }
}

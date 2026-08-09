//! Static plugin registration via Cargo features.

use infrarust_config::WebConfig;
use infrarust_core::plugin::StaticPluginLoader;

pub fn build_static_loader(
    web_config: Option<&mut WebConfig>,
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

    // Admin API: always compiled, conditionally registered based on [web] config
    if let Some(web) = web_config.filter(|w| w.enable_api) {
        use infrarust_api::plugin::Plugin;
        use infrarust_plugin_admin_api::config::{ApiConfig, RateLimitConfig};

        let api_key = web
            .resolve_api_key()
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

        let admin_api =
            infrarust_plugin_admin_api::AdminApiPlugin::new(config.clone(), enable_webui);
        loader.register(admin_api.metadata(), move || {
            Box::new(infrarust_plugin_admin_api::AdminApiPlugin::new(
                config.clone(),
                enable_webui,
            ))
        });
    }

    tracing::info!(
        count = loader.registered_count(),
        "Static plugins registered"
    );
    Ok(loader)
}

#[cfg(test)]
mod tests {
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
        let mut config = web(true);
        let loader = build_static_loader(Some(&mut config)).expect("loopback bind is accepted");
        assert!(loader.registered_ids().contains(&"admin_api".to_string()));
    }

    #[test]
    fn admin_api_is_not_registered_when_the_api_is_disabled() {
        let mut config = web(false);
        let loader = build_static_loader(Some(&mut config)).expect("nothing to resolve");
        assert!(!loader.registered_ids().contains(&"admin_api".to_string()));
        assert!(config.api_key.is_none(), "no key should be generated");
    }

    #[test]
    fn admin_api_is_not_registered_without_a_web_section() {
        let loader = build_static_loader(None).expect("nothing to resolve");
        assert!(!loader.registered_ids().contains(&"admin_api".to_string()));
    }
}

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
    if let Some(web) = web_config {
        use infrarust_api::plugin::Plugin;
        use infrarust_plugin_admin_api::config::{ApiConfig, RateLimitConfig};

        let api_key = web
            .resolve_api_key()
            .map_err(|e| anyhow::anyhow!("Invalid web API configuration: {e}"))?;

        let enable_webui = web.enable_webui;

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

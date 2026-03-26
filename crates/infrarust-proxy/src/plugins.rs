//! Static plugin registration via Cargo features.

use infrarust_config::WebConfig;
use infrarust_core::plugin::StaticPluginLoader;

pub fn build_static_loader(web_config: Option<&WebConfig>) -> StaticPluginLoader {
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
        let enable_api = web.enable_api;
        let enable_webui = web.enable_webui;
        let admin_api =
            infrarust_plugin_admin_api::AdminApiPlugin::new(enable_api, enable_webui);
        loader.register(admin_api.metadata(), move || {
            Box::new(infrarust_plugin_admin_api::AdminApiPlugin::new(
                enable_api,
                enable_webui,
            ))
        });
    }

    tracing::info!(
        count = loader.registered_count(),
        "Static plugins registered"
    );
    loader
}

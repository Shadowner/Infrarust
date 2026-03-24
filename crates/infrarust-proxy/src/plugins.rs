//! Static plugin registration via Cargo features.

use infrarust_core::plugin::StaticPluginLoader;

pub fn build_static_loader() -> StaticPluginLoader {
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
        loader.register(hello.metadata(), || Box::new(infrarust_plugin_hello::HelloPlugin));
    }

    #[cfg(feature = "plugin-server-wake")]
    {
        use infrarust_api::plugin::Plugin;
        let wake = infrarust_plugin_server_wake::ServerWakePlugin::default();
        loader.register(wake.metadata(), || {
            Box::new(infrarust_plugin_server_wake::ServerWakePlugin::default())
        });
    }

    #[cfg(feature = "plugin-admin-api")]
    {
        use infrarust_api::plugin::Plugin;
        let admin_api = infrarust_plugin_admin_api::AdminApiPlugin::new();
        loader.register(admin_api.metadata(), || {
            Box::new(infrarust_plugin_admin_api::AdminApiPlugin::new())
        });
    }

    tracing::info!(count = loader.registered_count(), "Static plugins registered");
    loader
}

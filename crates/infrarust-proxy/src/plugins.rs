//! Static plugin registration via Cargo features.

use infrarust_core::plugin::StaticPluginLoader;

pub fn build_static_loader() -> StaticPluginLoader {
    let loader = StaticPluginLoader::new();

    #[cfg(feature = "plugin-hello")]
    {
        use infrarust_api::plugin::PluginMetadata;
        loader.register(
            PluginMetadata::new("hello", "Hello Plugin", "0.1.0")
                .author("Infrarust")
                .description("Example plugin: logs connections, /hello command, limbo test gate"),
            || Box::new(infrarust_plugin_hello::HelloPlugin),
        );
        tracing::debug!("Registered static plugin: hello");
    }

    tracing::info!(count = loader.registered_count(), "Static plugins registered");
    loader
}

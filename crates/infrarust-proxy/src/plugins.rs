//! Static plugin registration via Cargo features.

use infrarust_core::plugin::StaticPluginLoader;

pub fn build_static_loader() -> StaticPluginLoader {
    let loader = StaticPluginLoader::new();

    #[cfg(feature = "plugin-hello")]
    {
        use infrarust_api::plugin::Plugin;
        let hello = infrarust_plugin_hello::HelloPlugin;
        loader.register(hello.metadata(), || Box::new(infrarust_plugin_hello::HelloPlugin));
    }

    tracing::info!(count = loader.registered_count(), "Static plugins registered");
    loader
}

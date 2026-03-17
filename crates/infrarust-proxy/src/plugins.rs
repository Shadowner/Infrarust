//! Static plugin collection via Cargo features.

use infrarust_api::plugin::Plugin;

/// Collects plugins that were compiled in via Cargo features.
pub fn collect_static_plugins() -> Vec<Box<dyn Plugin>> {
    #[allow(unused_mut)]
    let mut plugins: Vec<Box<dyn Plugin>> = vec![];

    #[cfg(feature = "plugin-hello")]
    {
        plugins.push(Box::new(infrarust_plugin_hello::HelloPlugin));
        tracing::debug!("Registered static plugin: hello");
    }

    tracing::info!(count = plugins.len(), "Static plugins collected");
    plugins
}

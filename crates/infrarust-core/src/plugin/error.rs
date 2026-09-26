use infrarust_api::error::PluginError;
use infrarust_api::loader::LoaderError;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PluginManagerError {
    #[error("loader '{loader}' discovery failed: {source}")]
    Discovery {
        loader: String,
        #[source]
        source: LoaderError,
    },

    #[error("duplicate plugin id '{plugin}': found in loader '{first}' and '{second}'")]
    DuplicateId {
        plugin: String,
        first: String,
        second: String,
    },

    #[error("plugin '{plugin}' requires '{dependency}' which is not loaded")]
    MissingDependency { plugin: String, dependency: String },

    #[error("circular dependency detected involving: {}", .0.join(", "))]
    Cycle(Vec<String>),

    #[error("loader '{loader}' on_load failed: {source}")]
    Loader {
        loader: String,
        #[source]
        source: LoaderError,
    },

    #[error("no loader mapping for plugin '{0}'")]
    NoLoader(String),

    #[error("loader '{loader}' failed to load '{plugin}': {source}")]
    Load {
        loader: String,
        plugin: String,
        #[source]
        source: LoaderError,
    },

    #[error("plugin '{plugin}' failed to enable: {source}")]
    Enable {
        plugin: String,
        #[source]
        source: PluginError,
    },

    #[error("plugin '{0}' is not enabled")]
    UnknownPlugin(String),

    #[error("plugin '{plugin}' is required by '{dependent}'")]
    RequiredBy { plugin: String, dependent: String },
}

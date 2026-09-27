use std::path::{Path, PathBuf};

use infrarust_api::error::PluginError;
use infrarust_api::loader::LoaderError;
use infrarust_plugin_common::InvalidPluginId;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PluginManagerError {
    #[error("loader '{loader}' discovery failed: {source}")]
    Discovery {
        loader: String,
        #[source]
        source: LoaderError,
    },

    #[error(
        "plugin '{plugin}' from loader '{refused}'{} is refused: loader '{kept}' already provides that id",
        origin_suffix(.origin)
    )]
    DuplicateId {
        plugin: String,
        kept: String,
        refused: String,
        origin: Option<PathBuf>,
    },

    #[error("a plugin from loader '{loader}'{} is refused: {reason}", origin_suffix(.origin))]
    InvalidId {
        loader: String,
        origin: Option<PathBuf>,
        reason: InvalidPluginId,
    },

    #[error("plugin '{plugin}' requires '{dependency}', which was not found")]
    MissingDependency { plugin: String, dependency: String },

    #[error(
        "plugin '{plugin}' is refused: its dependencies form a cycle ({})",
        .cycle.join(", ")
    )]
    Cycle { plugin: String, cycle: Vec<String> },

    #[error("plugin '{plugin}' requires '{dependency}', which is not enabled")]
    DependencyNotEnabled { plugin: String, dependency: String },

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

impl PluginManagerError {
    #[must_use]
    pub fn plugin(&self) -> Option<&str> {
        match self {
            Self::DuplicateId { plugin, .. }
            | Self::MissingDependency { plugin, .. }
            | Self::Cycle { plugin, .. }
            | Self::DependencyNotEnabled { plugin, .. }
            | Self::Load { plugin, .. }
            | Self::Enable { plugin, .. }
            | Self::RequiredBy { plugin, .. } => Some(plugin),
            Self::NoLoader(plugin) | Self::UnknownPlugin(plugin) => Some(plugin),
            Self::Discovery { .. } | Self::InvalidId { .. } | Self::Loader { .. } => None,
        }
    }
}

fn origin_suffix(origin: &Option<PathBuf>) -> String {
    origin
        .as_deref()
        .map(Path::display)
        .map_or_else(String::new, |path| format!(" ({path})"))
}

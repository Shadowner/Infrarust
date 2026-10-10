use crate::context::Context;
use crate::error::PluginError;

pub use crate::bindings::types::{PluginDependency, PluginMetadata};

pub trait Plugin: 'static {
    fn metadata(&self) -> PluginMetadata;

    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError>;

    fn on_disable(&self, _ctx: &Context) -> Result<(), PluginError> {
        Ok(())
    }

    fn register_codec_filters(_reg: &mut crate::codec::CodecRegistrar)
    where
        Self: Sized,
    {
    }

    fn register_limbo_handlers(_reg: &mut crate::limbo::LimboRegistrar)
    where
        Self: Sized,
    {
    }
}

impl PluginMetadata {
    #[must_use]
    pub fn new(id: impl Into<String>, name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            version: version.into(),
            authors: Vec::new(),
            description: None,
            dependencies: Vec::new(),
        }
    }

    #[must_use]
    pub fn author(mut self, author: impl Into<String>) -> Self {
        self.authors.push(author.into());
        self
    }

    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    #[must_use]
    pub fn depends_on(mut self, id: impl Into<String>) -> Self {
        self.dependencies.push(PluginDependency {
            id: id.into(),
            optional: false,
        });
        self
    }

    #[must_use]
    pub fn optional_dependency(mut self, id: impl Into<String>) -> Self {
        self.dependencies.push(PluginDependency {
            id: id.into(),
            optional: true,
        });
        self
    }
}

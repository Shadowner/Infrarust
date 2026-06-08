//! The guest `Plugin` trait and metadata builder.

use crate::context::Context;

pub use crate::bindings::guest::PluginMetadata;
pub use crate::bindings::types::PluginDependency;

/// A WASM plugin. Implement this and tag the impl with `#[plugin]`.
///
/// Unlike the native `Plugin` trait this is synchronous, errors with `String`,
/// and takes a concrete [`Context`] the guest is single-threaded with no
/// async runtime. Keep mutable plugin state in `Cell`/`RefCell` fields.
pub trait Plugin: 'static {
    fn metadata(&self) -> PluginMetadata;
    fn on_enable(&self, ctx: &Context) -> Result<(), String>;
    fn on_disable(&self, _ctx: &Context) -> Result<(), String> {
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
    pub fn depends_on(mut self, id: impl Into<String>, optional: bool) -> Self {
        self.dependencies.push(PluginDependency {
            id: id.into(),
            optional,
        });
        self
    }
}

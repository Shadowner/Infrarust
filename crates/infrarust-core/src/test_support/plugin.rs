use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use infrarust_api::error::PluginError;
use infrarust_api::event::BoxFuture;
use infrarust_api::plugin::{Plugin, PluginContext, PluginMetadata};

use crate::plugin::static_loader::StaticPluginLoader;

type EnableHook = Arc<dyn Fn(&dyn PluginContext) + Send + Sync>;

#[derive(Clone)]
pub struct TestPlugin {
    metadata: PluginMetadata,
    fail_on_enable: bool,
    on_enable: Option<EnableHook>,
    enabled: Arc<AtomicBool>,
    enable_calls: Arc<AtomicUsize>,
    disable_calls: Arc<AtomicUsize>,
    enable_order: Arc<AtomicUsize>,
    disable_order: Arc<AtomicUsize>,
    order: Arc<AtomicUsize>,
}

impl TestPlugin {
    #[must_use]
    pub fn new(id: &str) -> Self {
        Self {
            metadata: PluginMetadata::new(id, id, "1.0.0"),
            fail_on_enable: false,
            on_enable: None,
            enabled: Arc::default(),
            enable_calls: Arc::default(),
            disable_calls: Arc::default(),
            enable_order: Arc::default(),
            disable_order: Arc::default(),
            order: Arc::default(),
        }
    }

    #[must_use]
    pub fn version(mut self, version: &str) -> Self {
        self.metadata.version = version.to_owned();
        self
    }

    #[must_use]
    pub fn depends_on(mut self, plugin_id: &str) -> Self {
        self.metadata = self.metadata.depends_on(plugin_id);
        self
    }

    #[must_use]
    pub const fn fail_on_enable(mut self) -> Self {
        self.fail_on_enable = true;
        self
    }

    #[must_use]
    pub fn on_enable(mut self, hook: impl Fn(&dyn PluginContext) + Send + Sync + 'static) -> Self {
        self.on_enable = Some(Arc::new(hook));
        self
    }

    #[must_use]
    pub fn sharing_order(mut self, counter: &Arc<AtomicUsize>) -> Self {
        self.order = Arc::clone(counter);
        self
    }

    pub fn register(&self, loader: &StaticPluginLoader) {
        let plugin = self.clone();
        loader.register(self.metadata.clone(), move || Box::new(plugin.clone()));
    }

    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    #[must_use]
    pub fn enable_calls(&self) -> usize {
        self.enable_calls.load(Ordering::SeqCst)
    }

    #[must_use]
    pub fn disable_calls(&self) -> usize {
        self.disable_calls.load(Ordering::SeqCst)
    }

    #[must_use]
    pub fn enable_order(&self) -> usize {
        self.enable_order.load(Ordering::SeqCst)
    }

    #[must_use]
    pub fn disable_order(&self) -> usize {
        self.disable_order.load(Ordering::SeqCst)
    }
}

impl Plugin for TestPlugin {
    fn metadata(&self) -> PluginMetadata {
        self.metadata.clone()
    }

    fn on_enable<'a>(
        &'a self,
        ctx: &'a dyn PluginContext,
    ) -> BoxFuture<'a, Result<(), PluginError>> {
        Box::pin(async move {
            self.enable_calls.fetch_add(1, Ordering::SeqCst);
            self.enable_order
                .store(self.order.fetch_add(1, Ordering::SeqCst), Ordering::SeqCst);
            if self.fail_on_enable {
                return Err(PluginError::InitFailed("test failure".into()));
            }
            if let Some(hook) = &self.on_enable {
                hook(ctx);
            }
            self.enabled.store(true, Ordering::SeqCst);
            Ok(())
        })
    }

    fn on_disable(&self) -> BoxFuture<'_, Result<(), PluginError>> {
        Box::pin(async move {
            self.disable_calls.fetch_add(1, Ordering::SeqCst);
            self.disable_order
                .store(self.order.fetch_add(1, Ordering::SeqCst), Ordering::SeqCst);
            self.enabled.store(false, Ordering::SeqCst);
            Ok(())
        })
    }
}

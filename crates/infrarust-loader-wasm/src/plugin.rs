//! `WasmPlugin`: adapts an instantiated component to the native `Plugin` trait.

use std::sync::{Arc, Weak};

use infrarust_api::error::PluginError;
use infrarust_api::event::BoxFuture;
use infrarust_api::plugin::{Plugin, PluginContext, PluginMetadata};
use tokio::sync::Mutex;
use wasmtime::Store;

use crate::bindings::Plugin as PluginBindings;
use crate::error::WasmLoaderError;
use crate::store_state::PluginStoreState;

/// The one async guest-call ceremony: upgrade the weak instance ref, serialize on
/// the instance lock, refuse poisoned stores, re-arm the epoch budget, then run
/// `call`; a trap (`Err`) poisons the instance. Returns `None` when the call did
/// not complete (instance gone, poisoned, or trapped). Call sites only provide the
/// guest operation, so a future worker-task restructure (C2: a task owning the
/// `Store` fed by mpsc+oneshot) only has to swap this body.
pub(crate) async fn call_guest<T>(
    instance: Weak<Mutex<WasmInstance>>,
    op: &'static str,
    call: impl for<'a> FnOnce(
        &'a mut Store<PluginStoreState>,
        &'a PluginBindings,
    ) -> BoxFuture<'a, wasmtime::Result<T>>,
) -> Option<T> {
    let arc = instance.upgrade()?;
    let mut guard = arc.lock().await;
    let WasmInstance { store, bindings } = &mut *guard;
    if store.data().is_poisoned() {
        return None;
    }
    store.data_mut().reset_epoch_budget();
    match call(store, bindings).await {
        Ok(value) => Some(value),
        Err(trap) => {
            tracing::error!(plugin = %store.data().plugin_id, op, error = %trap,
                "wasm guest trapped; poisoning instance");
            store.data_mut().set_poisoned();
            None
        }
    }
}

pub(crate) struct WasmPlugin {
    metadata: PluginMetadata,
    plugin_id: String,
    inner: Arc<Mutex<WasmInstance>>,
}

pub(crate) struct WasmInstance {
    pub(crate) store: Store<PluginStoreState>,
    pub(crate) bindings: PluginBindings,
}

impl WasmPlugin {
    pub(crate) fn new(
        metadata: PluginMetadata,
        store: Store<PluginStoreState>,
        bindings: PluginBindings,
    ) -> Self {
        let plugin_id = metadata.id.clone();
        let inner = Arc::new(Mutex::new(WasmInstance { store, bindings }));
        if let Ok(mut guard) = inner.try_lock() {
            let weak = Arc::downgrade(&inner);
            guard.store.data_mut().set_instance_ref(weak);
        }
        Self {
            metadata,
            plugin_id,
            inner,
        }
    }
}

impl Plugin for WasmPlugin {
    fn metadata(&self) -> PluginMetadata {
        self.metadata.clone()
    }

    fn on_enable<'a>(
        &'a self,
        _ctx: &'a dyn PluginContext,
    ) -> BoxFuture<'a, Result<(), PluginError>> {
        debug_assert_eq!(_ctx.plugin_id(), self.plugin_id);
        Box::pin(async move {
            let mut guard = self.inner.lock().await;
            let WasmInstance { store, bindings } = &mut *guard;
            store.data_mut().reset_epoch_budget();
            match bindings
                .infrarust_plugin_guest()
                .call_on_enable(&mut *store)
                .await
            {
                Ok(Ok(())) => Ok(()),
                Ok(Err(message)) => {
                    tracing::warn!(plugin = %self.plugin_id, %message,
                        "wasm guest on_enable returned an error");
                    Err(PluginError::InitFailed(message))
                }
                Err(trap) => {
                    let err = WasmLoaderError::Trap {
                        plugin_id: self.plugin_id.clone(),
                        op: "on-enable",
                        reason: trap.to_string(),
                    };
                    tracing::error!(plugin = %self.plugin_id, error = %err,
                        "wasm guest trapped during on_enable");
                    store.data_mut().set_poisoned();
                    Err(err.into_plugin_error())
                }
            }
        })
    }

    fn on_disable(&self) -> BoxFuture<'_, Result<(), PluginError>> {
        Box::pin(async move {
            let mut guard = self.inner.lock().await;
            let WasmInstance { store, bindings } = &mut *guard;
            if store.data().is_poisoned() {
                tracing::warn!(plugin = %self.plugin_id,
                    "skipping on_disable for a poisoned (previously trapped) wasm plugin");
                return Ok(());
            }
            store.data_mut().reset_epoch_budget();
            match bindings
                .infrarust_plugin_guest()
                .call_on_disable(&mut *store)
                .await
            {
                Ok(Ok(())) => Ok(()),
                Ok(Err(message)) => {
                    tracing::warn!(plugin = %self.plugin_id, %message,
                        "wasm guest on_disable returned an error");
                    Err(PluginError::Custom(message))
                }
                Err(trap) => {
                    let err = WasmLoaderError::Trap {
                        plugin_id: self.plugin_id.clone(),
                        op: "on-disable",
                        reason: trap.to_string(),
                    };
                    tracing::error!(plugin = %self.plugin_id, error = %err,
                        "wasm guest trapped during on_disable");
                    Err(PluginError::Custom(err.to_string()))
                }
            }
        })
    }
}

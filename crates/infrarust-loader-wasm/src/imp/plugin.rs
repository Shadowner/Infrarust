use std::sync::Arc;

use infrarust_api::error::PluginError;
use infrarust_api::event::BoxFuture;
use infrarust_api::plugin::{Plugin, PluginContext, PluginMetadata};
use wasmtime::Store;

use crate::actor::{CallFailure, InstanceRef, PluginActor};
use crate::bindings::Plugin as PluginBindings;
use crate::error::WasmLoaderError;
use crate::store_state::PluginStoreState;

pub(crate) async fn call_guest<T, F>(instance: InstanceRef, op: &'static str, call: F) -> Option<T>
where
    T: Send + 'static,
    F: for<'a> FnOnce(
            &'a mut Store<PluginStoreState>,
            &'a PluginBindings,
        ) -> BoxFuture<'a, wasmtime::Result<T>>
        + Send
        + 'static,
{
    instance.call(op, call).await.ok()
}

pub(crate) struct WasmPlugin {
    metadata: PluginMetadata,
    plugin_id: String,
    actor: Arc<PluginActor>,
}

impl WasmPlugin {
    pub(crate) fn new(metadata: PluginMetadata, actor: Arc<PluginActor>) -> Self {
        let plugin_id = metadata.id.clone();
        Self {
            metadata,
            plugin_id,
            actor,
        }
    }

    fn lifecycle_error(&self, op: &'static str, failure: CallFailure) -> WasmLoaderError {
        match failure {
            CallFailure::Trapped(reason) => WasmLoaderError::Trap {
                plugin_id: self.plugin_id.clone(),
                op,
                reason,
            },
            other => WasmLoaderError::CallFailed {
                plugin_id: self.plugin_id.clone(),
                op,
                reason: other.to_string(),
            },
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
            let result = self
                .actor
                .call_lifecycle("on-enable", false, |store, bindings| {
                    Box::pin(async move {
                        bindings
                            .infrarust_plugin_guest()
                            .call_on_enable(&mut *store)
                            .await
                    })
                })
                .await;
            match result {
                Ok(Ok(())) => Ok(()),
                Ok(Err(message)) => {
                    tracing::warn!(plugin = %self.plugin_id, %message,
                        "wasm guest on_enable returned an error");
                    Err(PluginError::InitFailed(message))
                }
                Err(failure) => Err(self
                    .lifecycle_error("on-enable", failure)
                    .into_plugin_error()),
            }
        })
    }

    fn on_disable(&self) -> BoxFuture<'_, Result<(), PluginError>> {
        Box::pin(async move {
            let result = self
                .actor
                .call_lifecycle("on-disable", true, |store, bindings| {
                    Box::pin(async move {
                        bindings
                            .infrarust_plugin_guest()
                            .call_on_disable(&mut *store)
                            .await
                    })
                })
                .await;
            self.actor.shutdown().await;
            match result {
                Ok(Ok(())) => Ok(()),
                Ok(Err(message)) => {
                    tracing::warn!(plugin = %self.plugin_id, %message,
                        "wasm guest on_disable returned an error");
                    Err(PluginError::Custom(message))
                }
                Err(CallFailure::Poisoned) => {
                    tracing::warn!(plugin = %self.plugin_id,
                        "skipping on_disable for a poisoned wasm plugin (a previous call trapped or was abandoned)");
                    Ok(())
                }
                Err(CallFailure::Stopped) => {
                    tracing::debug!(plugin = %self.plugin_id,
                        "skipping on_disable for a wasm plugin that is already stopped");
                    Ok(())
                }
                Err(failure) => Err(PluginError::Custom(
                    self.lifecycle_error("on-disable", failure).to_string(),
                )),
            }
        })
    }
}

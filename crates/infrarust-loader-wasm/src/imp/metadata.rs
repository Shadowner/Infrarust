use std::path::Path;
use std::time::Duration;

use infrarust_api::plugin::PluginMetadata;
use infrarust_plugin_common::validate_plugin_id;
use wasmtime::component::{Component, Linker};
use wasmtime::{Engine, Store};

use crate::bindings::Plugin as PluginBindings;
use crate::bindings::exports::infrarust::plugin::guest::PluginMetadata as WitMetadata;
use crate::config::SandboxLimits;
use crate::error::{WasmLoaderError, bounded};
use crate::store_state::{PluginStoreState, build_probe_state, install_epoch_control};

pub(crate) const METADATA_TIME_LIMIT: Duration = Duration::from_secs(5);

pub(crate) async fn extract_metadata(
    engine: &Engine,
    linker: &Linker<PluginStoreState>,
    component: &Component,
    path: &Path,
    sandbox: &SandboxLimits,
) -> Result<PluginMetadata, WasmLoaderError> {
    let limit = METADATA_TIME_LIMIT.min(sandbox.max_call_duration);
    let wit_md = tokio::time::timeout(
        limit,
        call_metadata(engine, linker, component, path, sandbox),
    )
    .await
    .map_err(|_| WasmLoaderError::Metadata {
        path: path.to_path_buf(),
        reason: format!("metadata() did not return within {limit:?}"),
    })??;
    validate_plugin_id(&wit_md.id).map_err(|invalid| WasmLoaderError::Metadata {
        path: path.to_path_buf(),
        reason: bounded(&invalid),
    })?;

    let mut metadata = PluginMetadata::new(wit_md.id, wit_md.name, wit_md.version);
    for author in wit_md.authors {
        metadata = metadata.author(author);
    }
    if let Some(description) = wit_md.description {
        metadata = metadata.description(description);
    }
    for dependency in wit_md.dependencies {
        metadata = if dependency.optional {
            metadata.optional_dependency(dependency.id)
        } else {
            metadata.depends_on(dependency.id)
        };
    }
    Ok(metadata)
}

async fn call_metadata(
    engine: &Engine,
    linker: &Linker<PluginStoreState>,
    component: &Component,
    path: &Path,
    sandbox: &SandboxLimits,
) -> Result<WitMetadata, WasmLoaderError> {
    let probe_id = path.display().to_string();
    let mut store = Store::new(engine, build_probe_state(probe_id.clone(), sandbox));
    install_epoch_control(&mut store, sandbox.max_epoch_yields);
    store.limiter(|s: &mut PluginStoreState| s.limits_mut() as &mut dyn wasmtime::ResourceLimiter);

    let bindings = PluginBindings::instantiate_async(&mut store, component, linker)
        .await
        .map_err(|e| WasmLoaderError::Metadata {
            path: path.to_path_buf(),
            reason: bounded(format_args!("{e:#}")),
        })?;

    bindings
        .infrarust_plugin_guest()
        .call_metadata(&mut store)
        .await
        .map_err(|e| WasmLoaderError::Metadata {
            path: path.to_path_buf(),
            reason: bounded(format_args!("metadata() trapped: {e:#}")),
        })
}

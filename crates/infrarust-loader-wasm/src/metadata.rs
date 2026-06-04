use std::path::Path;

use infrarust_api::plugin::PluginMetadata;
use wasmtime::component::Component;
use wasmtime::{Engine, Store};

use crate::bindings::Plugin as PluginBindings;
use crate::error::WasmLoaderError;
use crate::linker::build_probe_linker;
use crate::store_state::{PluginStoreState, build_probe_state, install_epoch_control};

pub(crate) async fn extract_metadata(
    engine: &Engine,
    component: &Component,
    path: &Path,
) -> Result<PluginMetadata, WasmLoaderError> {
    let probe_id = path.display().to_string();
    let mut store = Store::new(engine, build_probe_state(probe_id.clone()));
    install_epoch_control(&mut store);
    store.limiter(|s: &mut PluginStoreState| s.limits_mut() as &mut dyn wasmtime::ResourceLimiter);

    let linker = build_probe_linker(engine, &probe_id)?;
    let bindings = PluginBindings::instantiate_async(&mut store, component, &linker)
        .await
        .map_err(|e| WasmLoaderError::Metadata {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })?;

    let wit_md = bindings.infrarust_plugin_guest().call_metadata(&mut store).await.map_err(|e| {
        WasmLoaderError::Metadata {
            path: path.to_path_buf(),
            reason: format!("metadata() trapped: {e}"),
        }
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

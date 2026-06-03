#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WasmLoaderError {
    /// Failed to build or configure the wasmtime engine.
    #[error("wasmtime engine error: {0}")]
    Engine(wasmtime::Error),
}

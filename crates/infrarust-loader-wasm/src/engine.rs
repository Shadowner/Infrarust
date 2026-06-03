use infrarust_config::ProxyConfig;
use wasmtime::{Config, Engine};

use crate::error::WasmLoaderError;

pub fn build_engine(cfg: &ProxyConfig) -> Result<Engine, WasmLoaderError> {
    let _ = cfg;

    let mut config = Config::new();
    config.epoch_interruption(true);
    config.wasm_component_model(true);
    config.consume_fuel(false);

    Engine::new(&config).map_err(WasmLoaderError::Engine)
}

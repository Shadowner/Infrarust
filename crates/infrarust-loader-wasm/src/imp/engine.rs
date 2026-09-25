use infrarust_config::ProxyConfig;
use wasmtime::{Config, Engine};

use crate::error::WasmLoaderError;

pub fn build_engine(cfg: &ProxyConfig) -> Result<Engine, WasmLoaderError> {
    infrarust_config::validate_wasm_config(cfg)
        .map_err(|e| WasmLoaderError::Config(e.to_string()))?;

    let mut config = Config::new();
    config.epoch_interruption(true);
    config.wasm_component_model(true);
    config.consume_fuel(false);

    Engine::new(&config).map_err(WasmLoaderError::Engine)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_invalid_wasm_section_refuses_to_build_an_engine() {
        let config: ProxyConfig = toml::from_str("[wasm]\nepoch_tick = \"0s\"\n").unwrap();
        let err = build_engine(&config).expect_err("a zero epoch tick cannot drive the sandbox");
        assert!(err.to_string().contains("wasm.epoch_tick"), "{err}");
    }

    #[test]
    fn the_default_wasm_section_builds_an_engine() {
        let config: ProxyConfig = toml::from_str("").unwrap();
        assert!(build_engine(&config).is_ok());
    }
}

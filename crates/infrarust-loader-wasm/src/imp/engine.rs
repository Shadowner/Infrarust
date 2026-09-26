use infrarust_config::ProxyConfig;
use wasmtime::{Config, Engine, InstanceAllocationStrategy, PoolingAllocationConfig};

use crate::error::WasmLoaderError;

const CORE_INSTANCES_PER_SLOT: u32 = 4;
const TABLES_PER_SLOT: u32 = 2;

pub fn build_engine(cfg: &ProxyConfig) -> Result<Engine, WasmLoaderError> {
    infrarust_config::validate_wasm_config(cfg)
        .map_err(|e| WasmLoaderError::Config(e.to_string()))?;

    let pool = cfg.wasm.instance_pool;
    if pool > 0 {
        match Engine::new(&engine_config(Some(pool))) {
            Ok(engine) => return Ok(engine),
            Err(error) => tracing::warn!(
                instance_pool = pool,
                error = %error,
                "wasm.instance_pool could not be reserved; falling back to on-demand instance allocation"
            ),
        }
    }
    Engine::new(&engine_config(None)).map_err(WasmLoaderError::Engine)
}

fn engine_config(pool: Option<u32>) -> Config {
    let mut config = Config::new();
    config.epoch_interruption(true);
    config.wasm_component_model(true);
    config.consume_fuel(false);
    config.concurrency_support(false);
    if let Some(slots) = pool {
        config.allocation_strategy(InstanceAllocationStrategy::Pooling(pooling(slots)));
    }
    config
}

fn pooling(slots: u32) -> PoolingAllocationConfig {
    let mut pool = PoolingAllocationConfig::new();
    pool.total_component_instances(slots);
    pool.total_memories(slots);
    pool.total_stacks(slots);
    pool.total_core_instances(slots.saturating_mul(CORE_INSTANCES_PER_SLOT));
    pool.total_tables(slots.saturating_mul(TABLES_PER_SLOT));
    pool
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

    #[test]
    fn an_instance_pool_builds_an_engine() {
        let config: ProxyConfig = toml::from_str("[wasm]\ninstance_pool = 8\n").unwrap();
        assert!(build_engine(&config).is_ok());
    }

    #[test]
    fn an_oversized_instance_pool_is_refused() {
        let config: ProxyConfig = toml::from_str("[wasm]\ninstance_pool = 40000\n").unwrap();
        let err = build_engine(&config).expect_err("the pool must fit the address space");
        assert!(err.to_string().contains("wasm.instance_pool"), "{err}");
    }
}

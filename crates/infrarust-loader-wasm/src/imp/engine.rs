use infrarust_config::ProxyConfig;
use wasmtime::{
    Config, Engine, InstanceAllocationStrategy, PoolingAllocationConfig, StoreLimits,
    StoreLimitsBuilder,
};

use crate::error::WasmLoaderError;

const CORE_INSTANCES_PER_SLOT: u32 = 4;
const MEMORIES_PER_SLOT: u32 = 1;
const TABLES_PER_SLOT: u32 = 2;
const POOLED_TABLE_BYTES: usize = 4096;
const POOLED_TABLE_ELEMENTS: usize = POOLED_TABLE_BYTES / size_of::<usize>();
const MAX_TABLE_ELEMENTS: usize = 20_000;

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
    pool.total_memories(slots.saturating_mul(MEMORIES_PER_SLOT));
    pool.total_stacks(slots);
    pool.total_core_instances(slots.saturating_mul(CORE_INSTANCES_PER_SLOT));
    pool.total_tables(slots.saturating_mul(TABLES_PER_SLOT));
    pool.table_elements(POOLED_TABLE_ELEMENTS);
    pool
}

pub(crate) fn store_limits(memory_bytes: usize) -> StoreLimits {
    StoreLimitsBuilder::new()
        .memory_size(memory_bytes)
        .memories(MEMORIES_PER_SLOT as usize)
        .instances(CORE_INSTANCES_PER_SLOT as usize)
        .tables(TABLES_PER_SLOT as usize)
        .table_elements(MAX_TABLE_ELEMENTS)
        .trap_on_grow_failure(true)
        .build()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    use wasmtime::{Instance, Module, Store};

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

    fn leb128(mut value: u32, out: &mut Vec<u8>) {
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            if value == 0 {
                out.push(byte);
                return;
            }
            out.push(byte | 0x80);
        }
    }

    fn section(id: u8, body: &[u8], out: &mut Vec<u8>) {
        out.push(id);
        leb128(u32::try_from(body.len()).unwrap(), out);
        out.extend_from_slice(body);
    }

    fn component_with_a_table_of(elements: usize) -> Vec<u8> {
        let elements = u32::try_from(elements).unwrap();
        let mut tables = vec![1, 0x70, 1];
        leb128(elements, &mut tables);
        leb128(elements, &mut tables);
        let mut module = b"\0asm\x01\0\0\0".to_vec();
        section(4, &tables, &mut module);
        let mut component = b"\0asm\x0d\0\x01\0".to_vec();
        section(1, &module, &mut component);
        section(2, &[1, 0, 0, 0], &mut component);
        component
    }

    fn pooled_engine() -> Engine {
        Engine::new(&engine_config(Some(8))).unwrap()
    }

    #[test]
    fn a_pooled_engine_loads_a_component_whose_table_fills_one_page() {
        let bytes = component_with_a_table_of(POOLED_TABLE_ELEMENTS);
        wasmtime::component::Component::new(&pooled_engine(), &bytes).unwrap();
    }

    #[test]
    fn a_pooled_engine_refuses_a_component_whose_table_outgrows_one_page() {
        let bytes = component_with_a_table_of(POOLED_TABLE_ELEMENTS + 1);
        let err = wasmtime::component::Component::new(&pooled_engine(), &bytes)
            .expect_err("a table larger than a pooled table slot cannot be instantiated");
        let chain = format!("{err:#}");
        assert!(
            chain.contains(&format!("exceeds the limit of {POOLED_TABLE_ELEMENTS}")),
            "{chain}"
        );
    }

    fn on_demand_engine() -> Engine {
        Engine::new(&engine_config(None)).unwrap()
    }

    fn limited_store(engine: &Engine) -> Store<StoreLimits> {
        let mut store = Store::new(engine, store_limits(1 << 20));
        store.limiter(|limits| limits);
        store.set_epoch_deadline(1);
        store
    }

    fn instantiate(store: &mut Store<StoreLimits>, wat: &str) -> wasmtime::Result<Instance> {
        let module = Module::new(store.engine(), wat)?;
        Instance::new(&mut *store, &module, &[])
    }

    #[test]
    fn an_on_demand_store_refuses_a_component_whose_table_outgrows_the_store_limit() {
        let engine = on_demand_engine();
        let bytes = component_with_a_table_of(MAX_TABLE_ELEMENTS + 1);
        let component = wasmtime::component::Component::new(&engine, &bytes).unwrap();
        let linker = wasmtime::component::Linker::<StoreLimits>::new(&engine);
        assert!(
            linker
                .instantiate(&mut limited_store(&engine), &component)
                .is_err()
        );

        let fits = component_with_a_table_of(MAX_TABLE_ELEMENTS);
        let component = wasmtime::component::Component::new(&engine, &fits).unwrap();
        linker
            .instantiate(&mut limited_store(&engine), &component)
            .unwrap();
    }

    #[test]
    fn growing_a_table_past_the_store_limit_traps() {
        let engine = on_demand_engine();
        let mut store = limited_store(&engine);
        let instance = instantiate(
            &mut store,
            r#"
            (module
                (table 1 funcref)
                (func (export "grow") (param i32) (result i32)
                    (table.grow (ref.null func) (local.get 0))))
            "#,
        )
        .unwrap();
        let grow = instance
            .get_typed_func::<u32, i32>(&mut store, "grow")
            .unwrap();
        let room = u32::try_from(MAX_TABLE_ELEMENTS - 1).unwrap();
        assert_eq!(grow.call(&mut store, room).unwrap(), 1);
        assert!(grow.call(&mut store, 1).is_err());
    }

    #[test]
    fn a_store_holds_the_instances_tables_and_memories_of_one_pool_slot() {
        let engine = on_demand_engine();
        let mut store = limited_store(&engine);
        for _ in 0..CORE_INSTANCES_PER_SLOT {
            instantiate(&mut store, "(module)").unwrap();
        }
        assert!(instantiate(&mut store, "(module)").is_err());

        let mut store = limited_store(&engine);
        instantiate(&mut store, "(module (table 1 funcref) (table 1 funcref))").unwrap();
        assert!(instantiate(&mut store, "(module (table 1 funcref))").is_err());

        let mut store = limited_store(&engine);
        assert!(instantiate(&mut store, "(module (memory 1) (memory 1))").is_err());
        instantiate(&mut store, "(module (memory 1))").unwrap();
    }

    #[test]
    fn an_oversized_instance_pool_is_refused() {
        let config: ProxyConfig = toml::from_str("[wasm]\ninstance_pool = 40000\n").unwrap();
        let err = build_engine(&config).expect_err("the pool must fit the address space");
        assert!(err.to_string().contains("wasm.instance_pool"), "{err}");
    }
}

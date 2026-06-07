//! WASM-1 `memory-bomb` fixture: allocates far past the 64 MiB store limit in `on_enable`.
//! The `ResourceLimiter` refuses `memory.grow`; with `trap_on_grow_failure` that is a trap
//! the host observes as an error.

wit_bindgen::generate!({
    world: "plugin",
    path: "../../../../infrarust-plugin-wit/wit",
    generate_all,
});

struct Component;

fixture_common::raw_fixture!(
    Component,
    id: "memory-bomb",
    name: "Memory Bomb Fixture",
    description: Some("WASM-1 memory-bomb fixture".to_string()),
    on_enable: {
        // 8 MiB chunks; after ~8 we cross the 64 MiB cap and growth is refused (a trap).
        let mut sink: Vec<Vec<u8>> = Vec::new();
        loop {
            sink.push(std::hint::black_box(vec![0u8; 8 * 1024 * 1024]));
        }
    }
);

export!(Component);

use wasmtime::{StoreLimits, StoreLimitsBuilder};

pub(crate) struct CodecStoreState {
    limits: StoreLimits,
}

impl CodecStoreState {
    pub(crate) fn new(memory_bytes: usize) -> Self {
        Self {
            limits: StoreLimitsBuilder::new()
                .memory_size(memory_bytes)
                .trap_on_grow_failure(true)
                .build(),
        }
    }

    pub(crate) fn limits_mut(&mut self) -> &mut StoreLimits {
        &mut self.limits
    }
}

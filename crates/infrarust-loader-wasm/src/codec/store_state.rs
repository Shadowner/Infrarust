use wasmtime::{StoreLimits, StoreLimitsBuilder};

use crate::consts::MEMORY_LIMIT;

pub(crate) struct CodecStoreState {
    limits: StoreLimits,
}

impl CodecStoreState {
    pub(crate) fn new() -> Self {
        Self {
            limits: StoreLimitsBuilder::new()
                .memory_size(MEMORY_LIMIT)
                .trap_on_grow_failure(true)
                .build(),
        }
    }

    pub(crate) fn limits_mut(&mut self) -> &mut StoreLimits {
        &mut self.limits
    }
}

//! Tunable sandbox constants. Values are hardcoded for WASM-1; wiring them to
//! `ProxyConfig` is deferred (see the `TODO(config)` markers).

use std::time::Duration;

/// How often the dedicated OS thread bumps the engine epoch (§9.2).
pub(crate) const EPOCH_TICK_INTERVAL: Duration = Duration::from_millis(50);

/// Epoch ticks granted before the deadline callback first fires, re-granted on each
/// cooperative yield. One tick ≈ [`EPOCH_TICK_INTERVAL`].
pub(crate) const EPOCH_DEADLINE_TICKS: u64 = 1;

/// Cooperative yields tolerated per guest call before a hard trap (`Interrupt`).
/// ≈ `MAX_EPOCH_YIELDS_BEFORE_TRAP * EPOCH_TICK_INTERVAL` ≈ 3 s of pure spin.
/// The cpu-spin test's wall-clock timeout must stay strictly above this.
pub(crate) const MAX_EPOCH_YIELDS_BEFORE_TRAP: u32 = 60;

/// Epoch ticks granted to a single synchronous codec guest call before a hard
/// trap. Reset before every `create`/`filter`/lifecycle call (see `codec::proxies`).
/// A per-packet filter completes in microseconds, so this is ~16×50 ms of pure
/// headroom that only a runaway filter can exceed — false trips are impossible.
pub(crate) const CODEC_EPOCH_DEADLINE_TICKS: u64 = 16;

/// Linear-memory cap per plugin instance (§9.3). TODO(config): read from `ProxyConfig`.
/// Instance/table/memory *counts* keep wasmtime's defaults (a component needs several core
/// instances), so only memory growth is bounded here.
pub(crate) const MEMORY_LIMIT: usize = 64 * 1024 * 1024;

/// AOT cache subdirectory, created under the scanned plugin dir.
pub(crate) const CACHE_SUBDIR: &str = ".cache";

/// Coarse wasmtime-line marker mixed into the cache key (bump on major upgrade).
/// wasmtime's own embedded version section is the real cross-version safety net.
pub(crate) const WASMTIME_CACHE_TAG: &str = "wasmtime-45";

/// WIT world version mixed into the cache key so a world bump invalidates artifacts.
pub(crate) use infrarust_plugin_wit::WORLD_VERSION;

/// Upper bound on a single async host service call (ban/server start-stop). Epoch
/// interruption cannot preempt a guest parked inside a host `.await`, so each such
/// call is wrapped in this timeout; on expiry the guest sees `service-error`.
/// Generous so legitimately slow operations complete. TODO(config): make tunable.
pub(crate) const HOST_CALL_TIMEOUT: Duration = Duration::from_secs(30);

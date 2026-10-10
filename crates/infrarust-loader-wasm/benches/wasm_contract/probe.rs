use std::fmt::Write as _;
use std::hint::black_box;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use infrarust_api::player::Player;
use infrarust_api::plugin::Plugin;
use infrarust_api::services::config_service::ConfigService;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::test_util::{MockConfigService, MockPlayerRegistry};
use infrarust_api::types::{GameProfile, ProfileProperty};
use infrarust_loader_wasm::WasmPluginLoader;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::Interest;
use tracing::{Event, Level, Metadata, Subscriber};

use crate::support::{self, EnvOptions, TestEnv};

pub(crate) const PERF_PROBE: &str = "perf-probe";
pub(crate) const MODIFY_HOST: &str = "modify.perf";

pub(crate) struct SinkSubscriber {
    lines: AtomicU64,
}

struct Line<'a>(&'a mut String);

impl Visit for Line<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let _ = write!(self.0, "{}={:?} ", field.name(), value);
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        let _ = write!(self.0, "{}={} ", field.name(), value);
    }
}

impl Subscriber for SinkSubscriber {
    fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
        if self.enabled(metadata) {
            Interest::always()
        } else {
            Interest::never()
        }
    }

    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        *metadata.level() <= Level::INFO
    }

    fn max_level_hint(&self) -> Option<tracing::level_filters::LevelFilter> {
        Some(tracing::level_filters::LevelFilter::INFO)
    }

    fn new_span(&self, _span: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut line = String::with_capacity(128);
        event.record(&mut Line(&mut line));
        black_box(line);
        self.lines.fetch_add(1, Ordering::Relaxed);
    }

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}

pub(crate) fn install_sink() {
    let sink = SinkSubscriber {
        lines: AtomicU64::new(0),
    };
    tracing::subscriber::set_global_default(sink).expect("one global subscriber");
}

pub(crate) struct Probe {
    pub(crate) env: TestEnv,
    _plugin: Box<dyn Plugin>,
    _loader: WasmPluginLoader,
    _dir: tempfile::TempDir,
}

pub(crate) async fn load(
    players: Arc<dyn PlayerRegistry>,
    config: Arc<dyn ConfigService>,
) -> Probe {
    let (dir, plugins_dir): (tempfile::TempDir, PathBuf) = support::stage(PERF_PROBE);
    let options = EnvOptions {
        player_registry: players,
        config_service: config,
        ..EnvOptions::default()
    }
    .grant(PERF_PROBE, "codec-filter");
    let env = support::make_env_with(plugins_dir.clone(), options);
    let loader = support::fresh_loader();
    infrarust_api::loader::PluginLoader::discover(&loader, &plugins_dir)
        .await
        .unwrap();
    let plugin = support::load_enabled(&loader, &env.factory, PERF_PROBE).await;
    Probe {
        env,
        _plugin: plugin,
        _loader: loader,
        _dir: dir,
    }
}

pub(crate) async fn load_default() -> Probe {
    load(
        Arc::new(MockPlayerRegistry::new()),
        Arc::new(MockConfigService::new()),
    )
    .await
}

pub(crate) fn base64ish(len: usize) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    (0..len)
        .map(|i| ALPHABET[(i * 7 + 3) % ALPHABET.len()] as char)
        .collect()
}

pub(crate) fn textured_profile(username: &str) -> GameProfile {
    GameProfile {
        uuid: uuid::Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef),
        username: username.to_owned(),
        properties: vec![ProfileProperty {
            name: "textures".to_owned(),
            value: base64ish(980),
            signature: Some(base64ish(684)),
        }],
    }
}

pub(crate) fn player(id: u64) -> Arc<dyn Player> {
    support::session_player(
        id,
        textured_profile(&format!("Player{id:05}")),
        767,
        SocketAddr::from(([127, 0, 0, 1], 20_000 + (id % 40_000) as u16)),
    )
}

pub(crate) fn registry_with(count: usize) -> Arc<MockPlayerRegistry> {
    let registry = MockPlayerRegistry::new();
    for id in 0..count {
        registry.add_dyn(player(id as u64 + 1));
    }
    Arc::new(registry)
}

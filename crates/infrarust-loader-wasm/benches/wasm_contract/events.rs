use std::hint::black_box;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use infrarust_api::event::bus::{EventBus, EventBusExt};
use infrarust_api::event::{BoxFuture, Event, EventPriority};
use infrarust_api::events::client::PlayerClientBrandEvent;
use infrarust_api::events::lifecycle::GameProfileRequestEvent;
use infrarust_api::events::proxy::{PingResponse, ProxyPingEvent};
use infrarust_api::types::{Component, NamedColor, ProtocolVersion, ServerId};
use infrarust_core::event_bus::EventBusImpl;

use crate::probe::{self, MODIFY_HOST, base64ish, textured_profile};
use crate::report::{Samples, Table, runtime, scaled, us};

const WORKERS: usize = 4;

#[derive(Clone, Copy)]
enum Kind {
    ClientBrand,
    ProxyPing,
    ProxyPingModified,
    GameProfile,
    GameProfileModified,
}

impl Kind {
    const ALL: [Self; 5] = [
        Self::ClientBrand,
        Self::ProxyPing,
        Self::ProxyPingModified,
        Self::GameProfile,
        Self::GameProfileModified,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::ClientBrand => "client-brand (cheap)",
            Self::ProxyPing => "proxy-ping (heavy, unchanged)",
            Self::ProxyPingModified => "proxy-ping (heavy, max players changed)",
            Self::GameProfile => "game-profile-request (unchanged)",
            Self::GameProfileModified => "game-profile-request (username changed)",
        }
    }
}

fn remote() -> SocketAddr {
    SocketAddr::from(([203, 0, 113, 7], 51_234))
}

fn host(modified: bool) -> String {
    if modified {
        MODIFY_HOST.to_owned()
    } else {
        "play.example.com".to_owned()
    }
}

fn ping_event(modified: bool) -> ProxyPingEvent {
    let description = Component::text("A Minecraft Server ")
        .color(NamedColor::Gold)
        .bold()
        .append(Component::text("running Infrarust").color("#55ff55"))
        .append(Component::text(" | 1.8 - 1.21").italic());
    let mut response = PingResponse::new(
        description,
        1000,
        742,
        ProtocolVersion::new(767),
        "Infrarust 1.21".to_owned(),
        Some(format!("data:image/png;base64,{}", base64ish(8_000))),
    );
    response.player_sample = (0..12)
        .map(|i| (format!("Sample{i:02}"), uuid::Uuid::from_u128(i)))
        .collect();
    ProxyPingEvent::new(
        remote(),
        Some(ServerId::from("lobby")),
        Some(host(modified)),
        ProtocolVersion::new(767),
        false,
        response,
    )
}

fn profile_event(modified: bool) -> GameProfileRequestEvent {
    GameProfileRequestEvent::new(
        textured_profile("Steve"),
        true,
        remote(),
        Some(host(modified)),
        ProtocolVersion::new(767),
    )
}

fn brand_event() -> PlayerClientBrandEvent {
    PlayerClientBrandEvent::new(probe::player(1), "vanilla".to_owned())
}

async fn fire_loop<E: Event>(bus: &EventBusImpl, mut event: E, iterations: usize) -> Samples {
    for _ in 0..iterations / 10 {
        event = bus.fire(event).await;
    }
    let mut samples = Samples::with_capacity(iterations);
    for _ in 0..iterations {
        let started = Instant::now();
        event = bus.fire(event).await;
        samples.push(started.elapsed());
    }
    black_box(event);
    samples
}

async fn fire_kind(bus: &EventBusImpl, kind: Kind, iterations: usize) -> Samples {
    match kind {
        Kind::ClientBrand => fire_loop(bus, brand_event(), iterations).await,
        Kind::ProxyPing => fire_loop(bus, ping_event(false), iterations).await,
        Kind::ProxyPingModified => fire_loop(bus, ping_event(true), iterations).await,
        Kind::GameProfile => fire_loop(bus, profile_event(false), iterations).await,
        Kind::GameProfileModified => fire_loop(bus, profile_event(true), iterations).await,
    }
}

fn subscribe_native<E: Event>(bus: &EventBusImpl) {
    let bus: &dyn EventBus = bus;
    bus.subscribe_async::<E, _>(EventPriority::NORMAL, |event| -> BoxFuture<'_, ()> {
        Box::pin(async move {
            black_box(&*event);
        })
    });
}

fn native_bus(kind: Kind) -> EventBusImpl {
    let bus = EventBusImpl::new();
    match kind {
        Kind::ClientBrand => subscribe_native::<PlayerClientBrandEvent>(&bus),
        Kind::ProxyPing | Kind::ProxyPingModified => subscribe_native::<ProxyPingEvent>(&bus),
        Kind::GameProfile | Kind::GameProfileModified => {
            subscribe_native::<GameProfileRequestEvent>(&bus);
        }
    }
    bus
}

pub(crate) fn run(runs: usize) {
    println!("## Event dispatch: one WASM listener (perf-probe) vs one native listener\n");
    let iterations = scaled(20_000);
    let mut p50 = Table::new(
        format!("whole fire, back to back, {iterations} fires per run, p50"),
        "µs",
    );
    let mut p99 = Table::new(
        format!("whole fire, back to back, {iterations} fires per run, p99"),
        "µs",
    );
    for _ in 0..runs {
        let rt = runtime(WORKERS);
        let probe = rt.block_on(probe::load_default());
        let bus = Arc::clone(&probe.env.event_bus);
        for kind in Kind::ALL {
            let native = native_bus(kind);
            let bus = Arc::clone(&bus);
            let (native_s, wasm_s) = rt.block_on(async move {
                tokio::spawn(async move {
                    (
                        fire_kind(&native, kind, iterations).await,
                        fire_kind(&bus, kind, iterations).await,
                    )
                })
                .await
                .unwrap()
            });
            let label = kind.label();
            p50.record(
                &format!("{label}: native listener"),
                us(native_s.percentile(50.0)),
            );
            p50.record(
                &format!("{label}: WASM listener"),
                us(wasm_s.percentile(50.0)),
            );
            p99.record(
                &format!("{label}: native listener"),
                us(native_s.percentile(99.0)),
            );
            p99.record(
                &format!("{label}: WASM listener"),
                us(wasm_s.percentile(99.0)),
            );
        }
        drop(probe);
        rt.shutdown_background();
    }
    p50.print();
    p99.print();
}

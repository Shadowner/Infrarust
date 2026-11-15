#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use divan::{Bencher, black_box};

use infrarust_api::event::bus::{EventBus, EventBusExt};
use infrarust_api::event::{ConnectionState, Event, EventPriority, PacketDirection, PacketFilter};
use infrarust_core::event_bus::EventBusImpl;

fn main() {
    divan::main();
}

struct BenchEvent(u64);
impl Event for BenchEvent {}

fn poll_ready<F: Future>(future: F) -> F::Output {
    let mut cx = Context::from_waker(Waker::noop());
    match pin!(future).poll(&mut cx) {
        Poll::Ready(out) => out,
        Poll::Pending => panic!("dispatch did not complete in one poll"),
    }
}

const THREADS: &[usize] = &[1, 4, 8];

#[divan::bench(threads = THREADS)]
fn shared_bus_fire_no_handlers(bencher: Bencher) {
    let bus = EventBusImpl::new();
    bencher.bench(|| black_box(poll_ready(bus.fire(BenchEvent(black_box(1)))).0));
}

#[divan::bench(threads = THREADS)]
fn shared_bus_fire_sync_handler(bencher: Bencher) {
    let bus = EventBusImpl::new();
    let bus_ref: &dyn EventBus = &bus;
    bus_ref.subscribe::<BenchEvent, _>(EventPriority::NORMAL, |e| e.0 += 1);
    bencher.bench(|| black_box(poll_ready(bus.fire(BenchEvent(black_box(1)))).0));
}

#[divan::bench(threads = THREADS)]
fn shared_bus_has_packet_listeners(bencher: Bencher) {
    let bus = EventBusImpl::new();
    let filter = PacketFilter {
        packet_id: 0x10,
        state: ConnectionState::Play,
        direction: PacketDirection::Serverbound,
    };
    bus.subscribe_packet(filter, EventPriority::NORMAL, Box::new(|_| {}));
    bencher.bench(|| {
        black_box(bus.has_packet_listeners(
            black_box(0x11),
            ConnectionState::Play,
            PacketDirection::Serverbound,
        ))
    });
}

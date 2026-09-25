use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use divan::{Bencher, black_box};

use infrarust_api::event::bus::{EventBus, EventBusExt};
use infrarust_api::event::{BoxFuture, Event, EventPriority};
use infrarust_core::event_bus::EventBusImpl;

fn main() {
    divan::main();
}

struct BenchEvent(u64);
impl Event for BenchEvent {}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
}

fn poll_ready<F: Future>(future: F) -> F::Output {
    let mut cx = Context::from_waker(Waker::noop());
    match pin!(future).poll(&mut cx) {
        Poll::Ready(out) => out,
        Poll::Pending => panic!("dispatch did not complete in one poll"),
    }
}

fn run(bencher: Bencher, bus: &EventBusImpl) {
    let rt = runtime();
    let _guard = rt.enter();
    bencher.bench_local(|| black_box(poll_ready(bus.fire(BenchEvent(black_box(1)))).0));
}

#[divan::bench]
fn fire_no_handlers(bencher: Bencher) {
    run(bencher, &EventBusImpl::new());
}

#[divan::bench(args = [1, 4])]
fn fire_sync_handlers(bencher: Bencher, count: usize) {
    let bus = EventBusImpl::new();
    let bus_ref: &dyn EventBus = &bus;
    for _ in 0..count {
        bus_ref.subscribe::<BenchEvent, _>(EventPriority::NORMAL, |e| e.0 += 1);
    }
    run(bencher, &bus);
}

#[divan::bench(args = [1, 4])]
fn fire_async_handlers(bencher: Bencher, count: usize) {
    let bus = EventBusImpl::new();
    let bus_ref: &dyn EventBus = &bus;
    for _ in 0..count {
        bus_ref.subscribe_async::<BenchEvent, _>(EventPriority::NORMAL, |e| -> BoxFuture<'_, ()> {
            Box::pin(async move { e.0 += 1 })
        });
    }
    run(bencher, &bus);
}

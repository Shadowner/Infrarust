use std::any::{Any, TypeId, type_name};
use std::sync::{Arc, Weak};

use infrarust_api::event::Event;
use tokio::sync::{mpsc, oneshot};

use super::EventBusImpl;
use crate::event_bus::diagnostic::short_type_name;
use crate::util::sync::lock;

pub(super) enum Queued {
    Event(PostedEvent),
    Barrier(oneshot::Sender<()>),
}

pub(super) struct PostedEvent {
    type_id: TypeId,
    event_type: &'static str,
    event: Box<dyn Any + Send>,
}

impl EventBusImpl {
    pub fn post<E: Event>(&self, event: E) {
        let posted = PostedEvent {
            type_id: TypeId::of::<E>(),
            event_type: type_name::<E>(),
            event: Box::new(event),
        };
        if self.queue.send(Queued::Event(posted)).is_err() {
            tracing::warn!(
                event = short_type_name(type_name::<E>()),
                "the event dispatcher has stopped; a posted event was dropped"
            );
        }
    }

    pub fn start_dispatcher(self: &Arc<Self>) {
        let undispatched = lock(&self.undispatched).take();
        if let Some(queue) = undispatched {
            tokio::spawn(run_dispatcher(Arc::downgrade(self), queue));
        }
    }

    pub async fn flush(&self) {
        let undispatched = lock(&self.undispatched).as_mut().map(|queue| {
            let mut drained = Vec::new();
            while let Ok(queued) = queue.try_recv() {
                drained.push(queued);
            }
            drained
        });
        let Some(queued) = undispatched else {
            let (done, flushed) = oneshot::channel();
            if self.queue.send(Queued::Barrier(done)).is_ok() {
                let _ = flushed.await;
            }
            return;
        };
        for item in queued {
            match item {
                Queued::Barrier(done) => {
                    let _ = done.send(());
                }
                Queued::Event(mut posted) => {
                    self.dispatch(
                        posted.type_id,
                        posted.event_type,
                        &mut *posted.event,
                        &self.core_owner,
                    )
                    .await;
                }
            }
        }
    }
}

async fn run_dispatcher(bus: Weak<EventBusImpl>, mut queue: mpsc::UnboundedReceiver<Queued>) {
    while let Some(queued) = queue.recv().await {
        match queued {
            Queued::Barrier(done) => {
                let _ = done.send(());
            }
            Queued::Event(posted) => {
                let Some(live) = bus.upgrade() else {
                    return;
                };
                let mut posted = posted;
                live.dispatch(
                    posted.type_id,
                    posted.event_type,
                    &mut *posted.event,
                    &live.core_owner,
                )
                .await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use infrarust_api::event::EventPriority;
    use infrarust_api::event::bus::EventBus;

    use super::*;

    struct TestEvent;
    impl Event for TestEvent {}

    #[tokio::test]
    async fn flush_without_a_dispatcher_delivers_the_posted_events() {
        let bus = EventBusImpl::new();
        let seen = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&seen);
        bus.subscribe_erased(
            TypeId::of::<TestEvent>(),
            EventPriority::NORMAL,
            Box::new(move |_| flag.store(true, Ordering::SeqCst)),
        );

        bus.post(TestEvent);
        tokio::time::timeout(Duration::from_secs(1), bus.flush())
            .await
            .expect("flush must not wait for a dispatcher that never started");

        assert!(seen.load(Ordering::SeqCst));
    }
}

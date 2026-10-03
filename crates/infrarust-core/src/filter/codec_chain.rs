//! Per-connection codec filter chain.

use std::net::{IpAddr, SocketAddr};
use std::panic::{AssertUnwindSafe, catch_unwind};

use infrarust_api::event::ConnectionState;
use infrarust_api::filter::{
    CodecFilterInstance, CodecSessionInit, CodecVerdict, ConnectionSide, FrameOutput,
};
use infrarust_api::types::{ProtocolVersion, RawPacket};

use super::codec_registry::CodecFilterRegistryImpl;
use crate::event_bus::diagnostic::panic_message;

pub const CODEC_FILTER_FAILED: &str = "Disconnected: a required packet filter failed.";

pub const CODEC_FILTER_UNAVAILABLE: &str =
    "Connection refused: a required packet filter is not available.";

/// Result of processing a packet through the codec filter chain.
pub enum FilterResult {
    Pass { modified: bool },
    Dropped,
    Replaced(FrameOutput),
    PassWithInjections { output: FrameOutput, modified: bool },
    Closed(String),
}

fn same_payload(before: &bytes::Bytes, after: &bytes::Bytes) -> bool {
    (before.as_ptr() == after.as_ptr() && before.len() == after.len()) || before == after
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Health {
    Healthy,
    Erroring,
    Poisoned,
}

struct Link {
    instance: Box<dyn CodecFilterInstance>,
    health: Health,
}

impl Link {
    fn new(instance: Box<dyn CodecFilterInstance>) -> Self {
        Self {
            instance,
            health: Health::Healthy,
        }
    }

    fn live(&mut self) -> Option<&mut dyn CodecFilterInstance> {
        (self.health != Health::Poisoned).then_some(&mut *self.instance)
    }

    #[cold]
    fn errored(&mut self, error: &dyn std::fmt::Display) {
        if self.health == Health::Healthy {
            self.health = Health::Erroring;
            tracing::warn!(
                error = %error,
                "CodecFilter error, passing frame through; later errors from this filter on this connection are logged at debug"
            );
        } else {
            tracing::debug!(error = %error, "CodecFilter error, passing frame through");
        }
    }

    #[cold]
    fn panicked(&mut self, payload: &(dyn std::any::Any + Send)) -> Option<String> {
        self.health = Health::Poisoned;
        let instance = &self.instance;
        let reason = catch_unwind(AssertUnwindSafe(|| {
            instance.close_reason().map(str::to_owned)
        }))
        .unwrap_or_else(|_| Some(CODEC_FILTER_FAILED.to_owned()));
        tracing::warn!(
            panic = %panic_message(payload),
            closing = reason.is_some(),
            "CodecFilter panicked; it is skipped for the rest of this connection"
        );
        reason
    }
}

/// A chain of [`CodecFilterInstance`]s for one side of one connection.
pub struct CodecFilterChain {
    links: Vec<Link>,
    /// Resolved filter ids, parallel to `links`, for per-filter timing
    /// attribution under the `bench-timing` feature.
    #[cfg(feature = "bench-timing")]
    filter_ids: Vec<std::sync::Arc<str>>,
}

/// Emits the per-packet total codec time on the `infrarust::bench_timing`
/// tracing target (only built under the `bench-timing` feature).
#[cfg(feature = "bench-timing")]
#[inline]
fn emit_packet_total(packet: &RawPacket, start: quanta::Instant) {
    tracing::debug!(
        target: "infrarust::bench_timing",
        id = packet.packet_id,
        len = packet.data.len(),
        ns = start.elapsed().as_nanos() as u64,
        "codec_packet"
    );
}

impl CodecFilterChain {
    /// Processes a packet through all filters sequentially.
    ///
    /// Returns the filter result indicating what happened to the packet.
    /// This is a sync operation — no `.await`.
    pub fn process(&mut self, packet: &mut RawPacket) -> FilterResult {
        if self.links.is_empty() {
            return FilterResult::Pass { modified: false };
        }
        let id_before = packet.packet_id;
        let data_before = packet.data.clone();

        #[cfg(feature = "bench-timing")]
        let packet_start = quanta::Instant::now();

        let mut output = FrameOutput::new();

        #[cfg(feature = "bench-timing")]
        let mut ids = self.filter_ids.iter();

        for link in &mut self.links {
            #[cfg(feature = "bench-timing")]
            let filter_id = ids.next();
            let Some(instance) = link.live() else {
                continue;
            };

            #[cfg(feature = "bench-timing")]
            let filter_start = quanta::Instant::now();

            let verdict =
                match catch_unwind(AssertUnwindSafe(|| instance.filter(packet, &mut output))) {
                    Ok(verdict) => verdict,
                    Err(payload) => match link.panicked(payload.as_ref()) {
                        Some(reason) => return FilterResult::Closed(reason),
                        None => continue,
                    },
                };

            #[cfg(feature = "bench-timing")]
            tracing::trace!(
                target: "infrarust::bench_timing",
                filter = filter_id.map_or("?", |s| s.as_ref()),
                ns = filter_start.elapsed().as_nanos() as u64,
                "codec_filter"
            );

            match verdict {
                CodecVerdict::Pass => continue,
                CodecVerdict::Drop => {
                    #[cfg(feature = "bench-timing")]
                    emit_packet_total(packet, packet_start);
                    return FilterResult::Dropped;
                }
                CodecVerdict::Replace => {
                    #[cfg(feature = "bench-timing")]
                    emit_packet_total(packet, packet_start);
                    return FilterResult::Replaced(output);
                }
                CodecVerdict::Error(e) => {
                    if let Some(reason) = link.instance.close_reason() {
                        return FilterResult::Closed(reason.to_owned());
                    }
                    link.errored(&e);
                }
            }
        }

        #[cfg(feature = "bench-timing")]
        emit_packet_total(packet, packet_start);

        let modified = packet.packet_id != id_before || !same_payload(&data_before, &packet.data);
        if output.has_injections() {
            FilterResult::PassWithInjections { output, modified }
        } else {
            FilterResult::Pass { modified }
        }
    }

    /// Notifies all filter instances of a protocol state change.
    pub fn notify_state_change(&mut self, new_state: ConnectionState) {
        for instance in self.links.iter_mut().filter_map(Link::live) {
            instance.on_state_change(new_state);
        }
    }

    /// Notifies all filter instances of a compression threshold change.
    pub fn notify_compression_change(&mut self, threshold: i32) {
        for instance in self.links.iter_mut().filter_map(Link::live) {
            instance.on_compression_change(threshold);
        }
    }

    /// Calls `on_close()` on all filter instances.
    pub fn close(&mut self) {
        for instance in self.links.iter_mut().filter_map(Link::live) {
            instance.on_close();
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.links.is_empty()
    }

    #[must_use]
    pub fn close_reason(&self) -> Option<&str> {
        self.links
            .iter()
            .filter(|link| link.health != Health::Poisoned)
            .find_map(|link| link.instance.close_reason())
    }
}

/// Builds two codec filter chains (client-side and server-side) for a session.
///
/// Each registered factory's `create()` is called twice: once for each side.
pub fn build_codec_chains(
    registry: &CodecFilterRegistryImpl,
    client_version: ProtocolVersion,
    connection_id: u64,
    remote_addr: SocketAddr,
    real_ip: Option<IpAddr>,
) -> (CodecFilterChain, CodecFilterChain) {
    let client_init = CodecSessionInit {
        client_version,
        connection_id,
        remote_addr,
        real_ip,
        side: ConnectionSide::ClientSide,
    };
    let server_init = CodecSessionInit {
        client_version,
        connection_id,
        remote_addr,
        real_ip,
        side: ConnectionSide::ServerSide,
    };

    #[cfg(not(feature = "bench-timing"))]
    let (client_instances, server_instances) = (
        registry.create_instances(&client_init),
        registry.create_instances(&server_init),
    );
    #[cfg(feature = "bench-timing")]
    let ((client_instances, client_ids), (server_instances, server_ids)) = (
        registry.create_instances_with_ids(&client_init),
        registry.create_instances_with_ids(&server_init),
    );

    (
        CodecFilterChain {
            links: client_instances.into_iter().map(Link::new).collect(),
            #[cfg(feature = "bench-timing")]
            filter_ids: client_ids,
        },
        CodecFilterChain {
            links: server_instances.into_iter().map(Link::new).collect(),
            #[cfg(feature = "bench-timing")]
            filter_ids: server_ids,
        },
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use infrarust_api::filter::*;

    use super::*;

    struct MockFactory {
        id: &'static str,
        priority: FilterPriority,
        create_count: Arc<AtomicU32>,
        verdict: CodecVerdict,
    }

    type VerdictFn = Box<dyn FnMut(&mut RawPacket, &mut FrameOutput) -> CodecVerdict + Send>;

    struct MockInstance {
        verdict_fn: VerdictFn,
        call_count: Arc<AtomicU32>,
    }

    impl CodecFilterInstance for MockInstance {
        fn filter(&mut self, packet: &mut RawPacket, output: &mut FrameOutput) -> CodecVerdict {
            self.call_count.fetch_add(1, Ordering::Relaxed);
            (self.verdict_fn)(packet, output)
        }
    }

    impl CodecFilterFactory for MockFactory {
        fn metadata(&self) -> FilterMetadata {
            FilterMetadata {
                id: self.id.to_string(),
                priority: self.priority,
                after: vec![],
                before: vec![],
            }
        }

        fn create(&self, _ctx: &CodecSessionInit) -> Box<dyn CodecFilterInstance> {
            self.create_count.fetch_add(1, Ordering::Relaxed);
            let verdict = match self.verdict {
                CodecVerdict::Pass => CodecVerdict::Pass,
                CodecVerdict::Drop => CodecVerdict::Drop,
                _ => CodecVerdict::Pass,
            };
            Box::new(MockInstance {
                verdict_fn: Box::new(move |_, _| match verdict {
                    CodecVerdict::Pass => CodecVerdict::Pass,
                    CodecVerdict::Drop => CodecVerdict::Drop,
                    _ => CodecVerdict::Pass,
                }),
                call_count: Arc::new(AtomicU32::new(0)),
            })
        }
    }

    fn empty_chain(_side: ConnectionSide) -> CodecFilterChain {
        CodecFilterChain {
            links: vec![],
            #[cfg(feature = "bench-timing")]
            filter_ids: vec![],
        }
    }

    fn chain_with_instances(instances: Vec<Box<dyn CodecFilterInstance>>) -> CodecFilterChain {
        CodecFilterChain {
            links: instances.into_iter().map(Link::new).collect(),
            #[cfg(feature = "bench-timing")]
            filter_ids: vec![],
        }
    }

    struct CloseTrackingInstance {
        filter_count: Arc<AtomicU32>,
        close_count: Arc<AtomicU32>,
    }

    impl CodecFilterInstance for CloseTrackingInstance {
        fn filter(&mut self, _packet: &mut RawPacket, _output: &mut FrameOutput) -> CodecVerdict {
            self.filter_count.fetch_add(1, Ordering::Relaxed);
            CodecVerdict::Pass
        }

        fn on_close(&mut self) {
            self.close_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn chain_survives_segments_and_closes_exactly_once() {
        let filter_count = Arc::new(AtomicU32::new(0));
        let close_count = Arc::new(AtomicU32::new(0));
        let instance: Box<dyn CodecFilterInstance> = Box::new(CloseTrackingInstance {
            filter_count: Arc::clone(&filter_count),
            close_count: Arc::clone(&close_count),
        });
        let mut chain = chain_with_instances(vec![instance]);

        let mut p1 = RawPacket::new(0x00, bytes::Bytes::from_static(b"a"));
        let _ = chain.process(&mut p1);
        assert_eq!(
            close_count.load(Ordering::Relaxed),
            0,
            "chain must not be closed on a server switch"
        );

        let mut p2 = RawPacket::new(0x00, bytes::Bytes::from_static(b"b"));
        let _ = chain.process(&mut p2);
        assert_eq!(
            filter_count.load(Ordering::Relaxed),
            2,
            "filter must remain active after a switch"
        );

        chain.close();
        assert_eq!(close_count.load(Ordering::Relaxed), 1);
    }

    struct Failing {
        reason: Option<&'static str>,
    }

    impl CodecFilterInstance for Failing {
        fn filter(&mut self, _packet: &mut RawPacket, _output: &mut FrameOutput) -> CodecVerdict {
            CodecVerdict::Error(CodecFilterError::Internal("broken".to_owned()))
        }

        fn close_reason(&self) -> Option<&str> {
            self.reason
        }
    }

    #[test]
    fn an_error_from_a_filter_that_must_close_ends_the_chain_with_its_reason() {
        let later = Arc::new(AtomicU32::new(0));
        let mut chain = chain_with_instances(vec![
            Box::new(Failing {
                reason: Some("required filter trapped"),
            }),
            Box::new(MockInstance {
                verdict_fn: Box::new(|_, _| CodecVerdict::Pass),
                call_count: Arc::clone(&later),
            }),
        ]);
        assert_eq!(chain.close_reason(), Some("required filter trapped"));
        let mut packet = RawPacket::new(0x00, bytes::Bytes::new());
        assert!(matches!(
            chain.process(&mut packet),
            FilterResult::Closed(reason) if reason == "required filter trapped"
        ));
        assert_eq!(later.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn an_error_from_a_filter_that_may_fail_open_passes_the_frame_on() {
        let later = Arc::new(AtomicU32::new(0));
        let mut chain = chain_with_instances(vec![
            Box::new(Failing { reason: None }),
            Box::new(MockInstance {
                verdict_fn: Box::new(|_, _| CodecVerdict::Pass),
                call_count: Arc::clone(&later),
            }),
        ]);
        assert_eq!(chain.close_reason(), None);
        let mut packet = RawPacket::new(0x00, bytes::Bytes::new());
        assert!(matches!(
            chain.process(&mut packet),
            FilterResult::Pass { modified: false }
        ));
        assert_eq!(later.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_empty_chain_passes() {
        let mut chain = empty_chain(ConnectionSide::ClientSide);
        let mut packet = RawPacket::new(0x1A, bytes::Bytes::from_static(b"test"));
        let result = chain.process(&mut packet);
        assert!(matches!(result, FilterResult::Pass { modified: false }));
    }

    #[test]
    fn test_single_filter_drop() {
        let call_count = Arc::new(AtomicU32::new(0));
        let count = Arc::clone(&call_count);
        let instance: Box<dyn CodecFilterInstance> = Box::new(MockInstance {
            verdict_fn: Box::new(|_, _| CodecVerdict::Drop),
            call_count: count,
        });

        let mut chain = chain_with_instances(vec![instance]);
        let mut packet = RawPacket::new(0x00, bytes::Bytes::new());
        let result = chain.process(&mut packet);
        assert!(matches!(result, FilterResult::Dropped));
        assert_eq!(call_count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_pass_keeps_packet_bytes_pointer_stable() {
        let instance: Box<dyn CodecFilterInstance> = Box::new(MockInstance {
            verdict_fn: Box::new(|_, _| CodecVerdict::Pass),
            call_count: Arc::new(AtomicU32::new(0)),
        });

        let mut chain = chain_with_instances(vec![instance]);
        let data = bytes::Bytes::from_static(b"payload");
        let ptr = data.as_ptr();
        let mut packet = RawPacket::new(0x2A, data);
        let result = chain.process(&mut packet);
        assert!(matches!(result, FilterResult::Pass { modified: false }));
        assert_eq!(packet.packet_id, 0x2A);
        assert_eq!(packet.data.as_ptr(), ptr);
        assert_eq!(packet.data.len(), b"payload".len());
    }

    #[test]
    fn test_filter_modifies_payload() {
        let instance: Box<dyn CodecFilterInstance> = Box::new(MockInstance {
            verdict_fn: Box::new(|packet, _| {
                packet.data = bytes::Bytes::from_static(b"modified");
                CodecVerdict::Pass
            }),
            call_count: Arc::new(AtomicU32::new(0)),
        });

        let mut chain = chain_with_instances(vec![instance]);
        let mut packet = RawPacket::new(0x00, bytes::Bytes::from_static(b"original"));
        let result = chain.process(&mut packet);
        assert!(matches!(result, FilterResult::Pass { modified: true }));
        assert_eq!(&packet.data[..], b"modified");
    }

    #[test]
    fn rewriting_identical_bytes_does_not_count_as_a_modification() {
        let instance: Box<dyn CodecFilterInstance> = Box::new(MockInstance {
            verdict_fn: Box::new(|packet, _| {
                packet.data = bytes::Bytes::copy_from_slice(&packet.data);
                CodecVerdict::Pass
            }),
            call_count: Arc::new(AtomicU32::new(0)),
        });

        let mut chain = chain_with_instances(vec![instance]);
        let mut packet = RawPacket::new(0x00, bytes::Bytes::from_static(b"same"));
        assert!(matches!(
            chain.process(&mut packet),
            FilterResult::Pass { modified: false }
        ));

        let instance: Box<dyn CodecFilterInstance> = Box::new(MockInstance {
            verdict_fn: Box::new(|packet, _| {
                packet.packet_id = 0x01;
                CodecVerdict::Pass
            }),
            call_count: Arc::new(AtomicU32::new(0)),
        });
        let mut chain = chain_with_instances(vec![instance]);
        assert!(matches!(
            chain.process(&mut packet),
            FilterResult::Pass { modified: true }
        ));
    }

    #[test]
    fn test_inject_before() {
        let instance: Box<dyn CodecFilterInstance> = Box::new(MockInstance {
            verdict_fn: Box::new(|_, output| {
                output.inject_before(RawPacket::new(0xFF, bytes::Bytes::from_static(b"injected")));
                CodecVerdict::Pass
            }),
            call_count: Arc::new(AtomicU32::new(0)),
        });

        let mut chain = chain_with_instances(vec![instance]);
        let mut packet = RawPacket::new(0x00, bytes::Bytes::new());
        let result = chain.process(&mut packet);
        match result {
            FilterResult::PassWithInjections { mut output, .. } => {
                let before = output.take_before();
                assert_eq!(before.len(), 1);
                assert_eq!(before[0].packet_id, 0xFF);
            }
            _ => panic!("expected PassWithInjections"),
        }
    }

    #[test]
    fn test_drop_stops_chain() {
        let count_a = Arc::new(AtomicU32::new(0));
        let count_b = Arc::new(AtomicU32::new(0));
        let count_a_clone = Arc::clone(&count_a);
        let count_b_clone = Arc::clone(&count_b);

        let instance_a: Box<dyn CodecFilterInstance> = Box::new(MockInstance {
            verdict_fn: Box::new(|_, _| CodecVerdict::Drop),
            call_count: count_a_clone,
        });
        let instance_b: Box<dyn CodecFilterInstance> = Box::new(MockInstance {
            verdict_fn: Box::new(|_, _| CodecVerdict::Pass),
            call_count: count_b_clone,
        });

        let mut chain = chain_with_instances(vec![instance_a, instance_b]);
        let mut packet = RawPacket::new(0x00, bytes::Bytes::new());
        let result = chain.process(&mut packet);
        assert!(matches!(result, FilterResult::Dropped));
        assert_eq!(count_a.load(Ordering::Relaxed), 1);
        assert_eq!(
            count_b.load(Ordering::Relaxed),
            0,
            "filter B should not be called after drop"
        );
    }

    #[test]
    fn test_dual_pipeline_creates_two_instances() {
        let registry = CodecFilterRegistryImpl::new();
        let create_count = Arc::new(AtomicU32::new(0));

        registry
            .register_builtin(Box::new(MockFactory {
                id: "test",
                priority: FilterPriority::Normal,
                create_count: Arc::clone(&create_count),
                verdict: CodecVerdict::Pass,
            }))
            .unwrap();

        let (_client_chain, _server_chain) = build_codec_chains(
            &registry,
            ProtocolVersion::new(767),
            1,
            "127.0.0.1:12345".parse().unwrap(),
            None,
        );

        assert_eq!(
            create_count.load(Ordering::Relaxed),
            2,
            "factory should be called twice (client + server)"
        );
    }

    #[test]
    fn test_chain_order() {
        // Track execution order with a shared vec
        let order = Arc::new(std::sync::Mutex::new(Vec::new()));

        let order1 = Arc::clone(&order);
        let instance_1: Box<dyn CodecFilterInstance> = Box::new(MockInstance {
            verdict_fn: Box::new(move |_, _| {
                order1.lock().unwrap().push(1);
                CodecVerdict::Pass
            }),
            call_count: Arc::new(AtomicU32::new(0)),
        });

        let order2 = Arc::clone(&order);
        let instance_2: Box<dyn CodecFilterInstance> = Box::new(MockInstance {
            verdict_fn: Box::new(move |_, _| {
                order2.lock().unwrap().push(2);
                CodecVerdict::Pass
            }),
            call_count: Arc::new(AtomicU32::new(0)),
        });

        let order3 = Arc::clone(&order);
        let instance_3: Box<dyn CodecFilterInstance> = Box::new(MockInstance {
            verdict_fn: Box::new(move |_, _| {
                order3.lock().unwrap().push(3);
                CodecVerdict::Pass
            }),
            call_count: Arc::new(AtomicU32::new(0)),
        });

        let mut chain = chain_with_instances(vec![instance_1, instance_2, instance_3]);
        let mut packet = RawPacket::new(0x00, bytes::Bytes::new());
        chain.process(&mut packet);

        let executed = order.lock().unwrap();
        assert_eq!(*executed, vec![1, 2, 3]);
    }

    #[test]
    fn test_state_change_notifies_all() {
        use std::sync::atomic::AtomicBool;

        struct StateTracker {
            notified: Arc<AtomicBool>,
        }
        impl CodecFilterInstance for StateTracker {
            fn filter(
                &mut self,
                _packet: &mut RawPacket,
                _output: &mut FrameOutput,
            ) -> CodecVerdict {
                CodecVerdict::Pass
            }
            fn on_state_change(&mut self, _new_state: ConnectionState) {
                self.notified.store(true, Ordering::Relaxed);
            }
        }

        let notified1 = Arc::new(AtomicBool::new(false));
        let notified2 = Arc::new(AtomicBool::new(false));

        let mut chain = chain_with_instances(vec![
            Box::new(StateTracker {
                notified: Arc::clone(&notified1),
            }),
            Box::new(StateTracker {
                notified: Arc::clone(&notified2),
            }),
        ]);

        chain.notify_state_change(ConnectionState::Play);
        assert!(notified1.load(Ordering::Relaxed));
        assert!(notified2.load(Ordering::Relaxed));
    }
}

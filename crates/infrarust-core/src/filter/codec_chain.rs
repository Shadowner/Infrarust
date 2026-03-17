//! Per-connection codec filter chain.

use std::net::{IpAddr, SocketAddr};

use infrarust_api::event::ConnectionState;
use infrarust_api::filter::{
    CodecContext, CodecFilterInstance, CodecSessionInit, CodecVerdict, ConnectionSide, FrameOutput,
};
use infrarust_api::types::{ProtocolVersion, RawPacket};

use super::codec_registry::CodecFilterRegistryImpl;

/// Result of processing a packet through the codec filter chain.
pub enum FilterResult {
    /// Packet passes through unchanged (or modified in place).
    Pass,
    /// Packet was dropped by a filter.
    Dropped,
    /// The original packet is replaced by injected frames.
    Replaced(FrameOutput),
    /// Packet passes through, but additional frames were injected.
    PassWithInjections(FrameOutput),
}

/// A chain of [`CodecFilterInstance`]s for one side of one connection.
pub struct CodecFilterChain {
    instances: Vec<Box<dyn CodecFilterInstance>>,
    context: CodecContext,
}

impl CodecFilterChain {
    /// Processes a packet through all filters sequentially.
    ///
    /// Returns the filter result indicating what happened to the packet.
    /// This is a sync operation — no `.await`.
    pub fn process(&mut self, packet: &mut RawPacket) -> FilterResult {
        if self.instances.is_empty() {
            return FilterResult::Pass;
        }

        let mut output = FrameOutput::new();

        for instance in &mut self.instances {
            match instance.filter(&self.context, packet, &mut output) {
                CodecVerdict::Pass => continue,
                CodecVerdict::Drop => return FilterResult::Dropped,
                CodecVerdict::Replace => return FilterResult::Replaced(output),
                CodecVerdict::Error(e) => {
                    tracing::warn!(error = %e, "CodecFilter error, passing frame through");
                    continue;
                }
            }
        }

        if output.has_injections() {
            FilterResult::PassWithInjections(output)
        } else {
            FilterResult::Pass
        }
    }

    /// Notifies all filter instances of a protocol state change.
    pub fn notify_state_change(&mut self, new_state: ConnectionState) {
        self.context.state = new_state;
        for instance in &mut self.instances {
            instance.on_state_change(new_state);
        }
    }

    /// Notifies all filter instances of a compression threshold change.
    pub fn notify_compression_change(&mut self, threshold: i32) {
        for instance in &mut self.instances {
            instance.on_compression_change(threshold);
        }
    }

    /// Calls `on_close()` on all filter instances.
    pub fn close(&mut self) {
        for instance in &mut self.instances {
            instance.on_close();
        }
    }

    /// Returns `true` if there are no filter instances.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
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

    let client_instances = registry.create_instances(&client_init);
    let server_instances = registry.create_instances(&server_init);

    let client_ctx = CodecContext {
        client_version,
        server_version: None,
        state: ConnectionState::Handshake,
        connection_id,
        side: ConnectionSide::ClientSide,
        player_info: None,
        is_proxy_consumed: false,
    };
    let server_ctx = CodecContext {
        side: ConnectionSide::ServerSide,
        ..client_ctx.clone()
    };

    (
        CodecFilterChain {
            instances: client_instances,
            context: client_ctx,
        },
        CodecFilterChain {
            instances: server_instances,
            context: server_ctx,
        },
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    use infrarust_api::filter::*;

    use super::*;

    // --- Mock infrastructure ---

    struct MockFactory {
        id: &'static str,
        priority: FilterPriority,
        create_count: Arc<AtomicU32>,
        verdict: CodecVerdict,
    }

    struct MockInstance {
        verdict_fn: Box<dyn FnMut(&mut RawPacket, &mut FrameOutput) -> CodecVerdict + Send>,
        call_count: Arc<AtomicU32>,
    }

    impl CodecFilterInstance for MockInstance {
        fn filter(
            &mut self,
            _ctx: &CodecContext,
            packet: &mut RawPacket,
            output: &mut FrameOutput,
        ) -> CodecVerdict {
            self.call_count.fetch_add(1, Ordering::Relaxed);
            (self.verdict_fn)(packet, output)
        }
    }

    impl CodecFilterFactory for MockFactory {
        fn metadata(&self) -> FilterMetadata {
            FilterMetadata {
                id: self.id,
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

    fn empty_chain(side: ConnectionSide) -> CodecFilterChain {
        CodecFilterChain {
            instances: vec![],
            context: CodecContext {
                client_version: ProtocolVersion::new(767),
                server_version: None,
                state: ConnectionState::Play,
                connection_id: 1,
                side,
                player_info: None,
                is_proxy_consumed: false,
            },
        }
    }

    fn chain_with_instances(
        instances: Vec<Box<dyn CodecFilterInstance>>,
    ) -> CodecFilterChain {
        CodecFilterChain {
            instances,
            context: CodecContext {
                client_version: ProtocolVersion::new(767),
                server_version: None,
                state: ConnectionState::Play,
                connection_id: 1,
                side: ConnectionSide::ClientSide,
                player_info: None,
                is_proxy_consumed: false,
            },
        }
    }

    // --- Tests ---

    #[test]
    fn test_empty_chain_passes() {
        let mut chain = empty_chain(ConnectionSide::ClientSide);
        let mut packet = RawPacket::new(0x1A, bytes::Bytes::from_static(b"test"));
        let result = chain.process(&mut packet);
        assert!(matches!(result, FilterResult::Pass));
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
        assert!(matches!(result, FilterResult::Pass));
        assert_eq!(&packet.data[..], b"modified");
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
            FilterResult::PassWithInjections(mut output) => {
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
        assert_eq!(count_b.load(Ordering::Relaxed), 0, "filter B should not be called after drop");
    }

    #[test]
    fn test_dual_pipeline_creates_two_instances() {
        let registry = CodecFilterRegistryImpl::new();
        let create_count = Arc::new(AtomicU32::new(0));

        registry.register(Box::new(MockFactory {
            id: "test",
            priority: FilterPriority::Normal,
            create_count: Arc::clone(&create_count),
            verdict: CodecVerdict::Pass,
        }));

        let (_client_chain, _server_chain) = build_codec_chains(
            &registry,
            ProtocolVersion::new(767),
            1,
            "127.0.0.1:12345".parse().unwrap(),
            None,
        );

        assert_eq!(create_count.load(Ordering::Relaxed), 2, "factory should be called twice (client + server)");
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
                _ctx: &CodecContext,
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
            Box::new(StateTracker { notified: Arc::clone(&notified1) }),
            Box::new(StateTracker { notified: Arc::clone(&notified2) }),
        ]);

        chain.notify_state_change(ConnectionState::Play);
        assert!(notified1.load(Ordering::Relaxed));
        assert!(notified2.load(Ordering::Relaxed));
    }
}

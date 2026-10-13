use std::time::Duration;

use bytes::Bytes;
use tokio::io::AsyncReadExt;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use infrarust_api::event::ResultedEvent;
use infrarust_api::events::claim::{RawConnectionEvent, RawConnectionResult};
use infrarust_api::events::handshake::RejectReason;
use infrarust_api::handoff::Handoff;

use crate::event_bus::CORE_OWNER;
use crate::handoff::ClaimMatch;
use crate::pipeline::admission;
use crate::pipeline::context::ConnectionContext;
use crate::services::ProxyServices;

pub const PARTIAL_MATCH_TIMEOUT: Duration = Duration::from_secs(5);
pub const NEED_MORE_TIMEOUT: Duration = Duration::from_secs(5);
pub const MAX_RAW_BYTES: usize = 4096;
pub const MAX_RAW_ROUNDS: usize = 8;

const READ_CHUNK: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawClaimLimits {
    pub partial_match: Duration,
    pub need_more: Duration,
    pub max_bytes: usize,
    pub max_rounds: usize,
}

impl Default for RawClaimLimits {
    fn default() -> Self {
        Self {
            partial_match: PARTIAL_MATCH_TIMEOUT,
            need_more: NEED_MORE_TIMEOUT,
            max_bytes: MAX_RAW_BYTES,
            max_rounds: MAX_RAW_ROUNDS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawClaim {
    Minecraft,
    Handoff(Handoff),
    Closed,
}

enum Read {
    Data,
    Ended,
    Shutdown,
}

pub async fn claim_raw(
    services: &ProxyServices,
    ctx: &mut ConnectionContext,
    limits: RawClaimLimits,
    shutdown: &CancellationToken,
) -> RawClaim {
    if filtered(services, ctx) {
        return RawClaim::Minecraft;
    }
    let claims = &services.connection_claims;
    let deadline = Instant::now() + limits.partial_match;
    let prefix = loop {
        match claims.matches(&ctx.buffered_data) {
            ClaimMatch::Match(prefix) => break prefix,
            ClaimMatch::NoMatchPossible => return RawClaim::Minecraft,
            ClaimMatch::NeedMoreBytes => {}
        }
        match read_more(ctx, deadline, limits.max_bytes, shutdown).await {
            Read::Data => {}
            Read::Ended => return RawClaim::Minecraft,
            Read::Shutdown => return RawClaim::Closed,
        }
    };

    let bus = &services.event_bus;
    if !bus.has_listeners::<RawConnectionEvent>() {
        return RawClaim::Minecraft;
    }
    let screened = tokio::select! {
        biased;
        () = shutdown.cancelled() => return RawClaim::Closed,
        refused = admission::refuse_handoff(&services.ban_manager, ctx) => refused,
    };
    if let Some(refusal) = screened {
        tracing::debug!(
            client = %ctx.client_addr(),
            reason = refusal.reason.as_str(),
            "raw connection refused before its claim"
        );
        admission::reject(bus, ctx.client_addr(), None, refusal.reason);
        return RawClaim::Closed;
    }

    let mut rounds = 0;
    loop {
        rounds += 1;
        let event = RawConnectionEvent::new(
            ctx.connection_id,
            ctx.client_addr(),
            ctx.local_addr,
            prefix.clone(),
            Bytes::copy_from_slice(&ctx.buffered_data),
        );
        let (event, decided_by) = tokio::select! {
            biased;
            () = shutdown.cancelled() => return RawClaim::Closed,
            fired = bus.fire_decided(event) => fired,
        };
        let wanted = match event.result() {
            RawConnectionResult::NeedMore(wanted) => usize::try_from(*wanted).unwrap_or(usize::MAX),
            RawConnectionResult::Handoff(handoff) => return RawClaim::Handoff(handoff.clone()),
            RawConnectionResult::Reject => {
                let plugin_id = decided_by
                    .filter(|owner| &**owner != CORE_OWNER)
                    .map(|owner| owner.to_string());
                tracing::debug!(client = %ctx.client_addr(), plugin = ?plugin_id, "raw connection rejected by a plugin");
                admission::reject(
                    bus,
                    ctx.client_addr(),
                    None,
                    RejectReason::Plugin { plugin_id },
                );
                return RawClaim::Closed;
            }
            _ => return RawClaim::Minecraft,
        };
        if rounds >= limits.max_rounds || wanted > limits.max_bytes {
            tracing::debug!(
                client = %ctx.client_addr(),
                wanted,
                rounds,
                "raw connection asked for more than the claim allows, closing it"
            );
            return RawClaim::Closed;
        }
        let deadline = Instant::now() + limits.need_more;
        while ctx.buffered_data.len() < wanted {
            match read_more(ctx, deadline, wanted, shutdown).await {
                Read::Data => {}
                Read::Shutdown => return RawClaim::Closed,
                Read::Ended => {
                    tracing::debug!(
                        client = %ctx.client_addr(),
                        wanted,
                        read = ctx.buffered_data.len(),
                        "raw connection did not send the bytes its claim waits for, closing it"
                    );
                    return RawClaim::Closed;
                }
            }
        }
    }
}

fn filtered(services: &ProxyServices, ctx: &ConnectionContext) -> bool {
    services
        .config
        .ip_filter
        .as_ref()
        .is_some_and(|filter| !filter.is_allowed(&ctx.client_ip))
}

async fn read_more(
    ctx: &mut ConnectionContext,
    deadline: Instant,
    limit: usize,
    shutdown: &CancellationToken,
) -> Read {
    let room = limit
        .saturating_sub(ctx.buffered_data.len())
        .min(READ_CHUNK);
    if room == 0 {
        return Read::Ended;
    }
    let mut buf = [0u8; READ_CHUNK];
    let read = tokio::select! {
        biased;
        () = shutdown.cancelled() => return Read::Shutdown,
        read = tokio::time::timeout_at(deadline, ctx.stream_mut().read(&mut buf[..room])) => read,
    };
    match read {
        Ok(Ok(n)) if n > 0 => {
            ctx.buffered_data.extend_from_slice(&buf[..n]);
            Read::Data
        }
        _ => Read::Ended,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use std::net::IpAddr;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use infrarust_api::event::EventPriority;
    use infrarust_api::event::bus::{EventBus, EventBusExt};
    use infrarust_api::events::handshake::ConnectionRejectedEvent;
    use infrarust_api::handoff::HandoffTarget;
    use infrarust_api::services::ban_service::{BanRequest, BanTarget};
    use tokio::io::AsyncWriteExt;
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::Notify;

    use super::*;
    use crate::ban::{BanManager, FileBanStorage};
    use crate::limbo::test_helpers::test_proxy_services;
    use crate::session::connection_registry::ConnectionRegistry;

    const SHORT: RawClaimLimits = RawClaimLimits {
        partial_match: Duration::from_millis(200),
        need_more: Duration::from_millis(200),
        max_bytes: 64,
        max_rounds: 3,
    };

    type Seen = Arc<Mutex<Vec<RawConnectionEvent>>>;

    struct Peer {
        client: TcpStream,
        ctx: ConnectionContext,
    }

    async fn peer(sent: &[u8]) -> Peer {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local = listener.local_addr().unwrap();
        let mut client = TcpStream::connect(local).await.unwrap();
        let (stream, remote) = listener.accept().await.unwrap();
        client.write_all(sent).await.unwrap();
        Peer {
            client,
            ctx: ConnectionContext::new_for_test(stream, remote, remote.ip(), local),
        }
    }

    fn claiming(prefixes: &[&[u8]]) -> ProxyServices {
        let services = test_proxy_services();
        for prefix in prefixes {
            services
                .connection_claims
                .register("bridge", prefix)
                .unwrap();
        }
        services
    }

    fn on_raw(
        services: &ProxyServices,
        decide: impl Fn(&mut RawConnectionEvent) + Send + Sync + 'static,
    ) -> Seen {
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&seen);
        let listeners: &dyn EventBus = &*services.event_bus;
        listeners.subscribe::<RawConnectionEvent, _>(EventPriority::NORMAL, move |event| {
            record.lock().unwrap().push(RawConnectionEvent::new(
                event.connection_id,
                event.remote_addr,
                event.local_addr,
                event.prefix.clone(),
                event.data.clone(),
            ));
            decide(event);
        });
        seen
    }

    fn data(seen: &Seen) -> Vec<Bytes> {
        seen.lock()
            .unwrap()
            .iter()
            .map(|event| event.data.clone())
            .collect()
    }

    async fn refused(services: &ProxyServices, peer: &mut Peer) -> Vec<RejectReason> {
        let reasons = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&reasons);
        let listeners: &dyn EventBus = &*services.event_bus;
        listeners.subscribe::<ConnectionRejectedEvent, _>(EventPriority::NORMAL, move |event| {
            record
                .lock()
                .unwrap()
                .push((event.virtual_host.clone(), event.reason.clone()));
        });
        assert_eq!(claim(services, peer, SHORT).await, RawClaim::Closed);
        services.event_bus.flush().await;
        let reasons = reasons.lock().unwrap().clone();
        assert!(
            reasons.iter().all(|(host, _)| host.is_none()),
            "{reasons:?}"
        );
        reasons.into_iter().map(|(_, reason)| reason).collect()
    }

    async fn claim(services: &ProxyServices, peer: &mut Peer, limits: RawClaimLimits) -> RawClaim {
        claim_raw(services, &mut peer.ctx, limits, &CancellationToken::new()).await
    }

    #[tokio::test]
    async fn bytes_that_cannot_match_go_on_as_minecraft_untouched() {
        let services = claiming(&[b"XFER"]);
        let seen = on_raw(&services, RawConnectionEvent::reject);
        let handshake = [0x10, 0x00, 0xFF, 0x05, 0x09, b'l', b'o', b'b', b'b', b'y'];
        let mut peer = peer(&handshake).await;

        let started = Instant::now();
        let claimed = claim(&services, &mut peer, RawClaimLimits::default()).await;

        assert_eq!(claimed, RawClaim::Minecraft);
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(peer.ctx.buffered_data.as_ref(), &handshake[..]);
        assert!(seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn bytes_already_buffered_decide_without_a_read() {
        let services = claiming(&[b"XFER"]);
        let seen = on_raw(&services, RawConnectionEvent::pass);
        let mut peer = peer(b"").await;
        peer.ctx.buffered_data.extend_from_slice(b"XFER!");

        assert_eq!(
            claim(&services, &mut peer, SHORT).await,
            RawClaim::Minecraft
        );
        assert_eq!(data(&seen), [Bytes::from_static(b"XFER!")]);
    }

    #[tokio::test]
    async fn a_partial_prefix_that_stalls_goes_on_as_minecraft() {
        let services = claiming(&[b"XFER"]);
        let seen = on_raw(&services, RawConnectionEvent::reject);
        let mut peer = peer(b"XF").await;

        let started = Instant::now();
        assert_eq!(
            claim(&services, &mut peer, SHORT).await,
            RawClaim::Minecraft
        );

        assert!(started.elapsed() >= SHORT.partial_match);
        assert_eq!(peer.ctx.buffered_data.as_ref(), b"XF");
        assert!(seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_partial_prefix_then_another_byte_goes_on_as_minecraft() {
        let services = claiming(&[b"XFER"]);
        let seen = on_raw(&services, RawConnectionEvent::reject);
        let Peer {
            mut client,
            mut ctx,
        } = peer(b"XF").await;
        let writer = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            client.write_all(b"X").await.unwrap();
            client
        });

        let claimed = claim_raw(
            &services,
            &mut ctx,
            RawClaimLimits::default(),
            &CancellationToken::new(),
        )
        .await;

        assert_eq!(claimed, RawClaim::Minecraft);
        assert_eq!(ctx.buffered_data.as_ref(), b"XFX");
        assert!(seen.lock().unwrap().is_empty());
        drop(writer.await.unwrap());
    }

    #[tokio::test]
    async fn a_match_nobody_listens_to_goes_on_as_minecraft() {
        let services = claiming(&[b"XFER"]);
        let mut peer = peer(b"XFER!").await;
        assert_eq!(
            claim(&services, &mut peer, SHORT).await,
            RawClaim::Minecraft
        );
        assert_eq!(peer.ctx.buffered_data.as_ref(), b"XFER!");
    }

    #[tokio::test]
    async fn a_pass_keeps_every_byte_in_front_of_the_stream() {
        let services = claiming(&[b"XFER", b"XFOK"]);
        let seen = on_raw(&services, RawConnectionEvent::pass);
        let mut peer = peer(b"XFER\x00\x01data").await;

        assert_eq!(
            claim(&services, &mut peer, SHORT).await,
            RawClaim::Minecraft
        );

        assert_eq!(peer.ctx.buffered_data.as_ref(), b"XFER\x00\x01data");
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].connection_id, peer.ctx.connection_id);
        assert_eq!(seen[0].remote_addr, peer.ctx.client_addr());
        assert_eq!(seen[0].local_addr, peer.ctx.local_addr);
        assert_eq!(seen[0].prefix, Bytes::from_static(b"XFER"));
        assert_eq!(seen[0].data, Bytes::from_static(b"XFER\x00\x01data"));
    }

    #[tokio::test]
    async fn need_more_reads_until_the_listener_has_enough() {
        let services = claiming(&[b"XFER"]);
        let asked = Arc::new(Notify::new());
        let ask = Arc::clone(&asked);
        let handoff = Handoff::to(HandoffTarget::address("127.0.0.1", 30037));
        let decided = handoff.clone();
        let seen = on_raw(&services, move |event| {
            if event.data.len() < 10 {
                ask.notify_one();
                event.need_more(10);
            } else {
                event.handoff(decided.clone());
            }
        });
        let Peer {
            mut client,
            mut ctx,
        } = peer(b"XFER").await;
        let writer = tokio::spawn(async move {
            asked.notified().await;
            client.write_all(b"0123").await.unwrap();
            tokio::time::sleep(Duration::from_millis(30)).await;
            client.write_all(b"456789tail").await.unwrap();
            client
        });

        let claimed = claim_raw(&services, &mut ctx, SHORT, &CancellationToken::new()).await;

        assert_eq!(claimed, RawClaim::Handoff(handoff));
        assert_eq!(
            data(&seen),
            [
                Bytes::from_static(b"XFER"),
                Bytes::from_static(b"XFER012345")
            ]
        );
        assert_eq!(ctx.buffered_data.as_ref(), b"XFER012345");
        drop(writer.await.unwrap());
    }

    #[tokio::test]
    async fn need_more_closes_a_client_that_does_not_send_in_time() {
        let services = claiming(&[b"XFER"]);
        let seen = on_raw(&services, |event| event.need_more(10));
        let mut peer = peer(b"XFER").await;

        let started = Instant::now();
        assert_eq!(claim(&services, &mut peer, SHORT).await, RawClaim::Closed);
        assert!(started.elapsed() >= SHORT.need_more);
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn need_more_is_bounded_by_bytes_and_by_rounds() {
        let services = claiming(&[b"XFER"]);
        let too_much = u32::try_from(SHORT.max_bytes + 1).unwrap();
        let seen = on_raw(&services, move |event| event.need_more(too_much));
        let mut greedy = peer(b"XFER").await;
        assert_eq!(claim(&services, &mut greedy, SHORT).await, RawClaim::Closed);
        assert_eq!(seen.lock().unwrap().len(), 1);

        let services = claiming(&[b"XFER"]);
        let fired = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&fired);
        on_raw(&services, move |event| {
            count.fetch_add(1, Ordering::SeqCst);
            event.need_more(4);
        });
        let mut stuck = peer(b"XFER").await;
        assert_eq!(claim(&services, &mut stuck, SHORT).await, RawClaim::Closed);
        assert_eq!(fired.load(Ordering::SeqCst), SHORT.max_rounds);
    }

    #[tokio::test]
    async fn a_rejected_claim_is_closed_and_reported() {
        let services = claiming(&[b"XFER"]);
        on_raw(&services, RawConnectionEvent::reject);
        let mut peer = peer(b"XFER").await;
        assert_eq!(
            refused(&services, &mut peer).await,
            [RejectReason::Plugin { plugin_id: None }]
        );
    }

    #[tokio::test]
    async fn a_filtered_address_is_left_to_the_pipeline_without_a_peek() {
        let mut services = claiming(&[b"XFER"]);
        services.config =
            Arc::new(toml::from_str("[ip_filter]\nblacklist = [\"127.0.0.1/32\"]\n").unwrap());
        let seen = on_raw(&services, RawConnectionEvent::pass);
        let rejected = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&rejected);
        let listeners: &dyn EventBus = &*services.event_bus;
        listeners.subscribe::<ConnectionRejectedEvent, _>(EventPriority::NORMAL, move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
        });

        for sent in [&b""[..], b"XF", b"XFER"] {
            let mut peer = peer(sent).await;
            let started = Instant::now();
            let claimed = claim(&services, &mut peer, RawClaimLimits::default()).await;
            assert_eq!(claimed, RawClaim::Minecraft, "{sent:?}");
            assert!(started.elapsed() < Duration::from_secs(1), "{sent:?}");
            assert!(peer.ctx.buffered_data.is_empty(), "{sent:?}");
        }

        services.event_bus.flush().await;
        assert_eq!(rejected.load(Ordering::SeqCst), 0);
        assert!(seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_ban_check_that_cannot_answer_closes_the_claim() {
        let mut services = claiming(&[b"XFER"]);
        services.ban_manager = Arc::new(BanManager::plugin(
            "absent",
            Arc::new(ConnectionRegistry::new()),
            Arc::new(crate::event_bus::EventBusImpl::new()),
        ));
        let seen = on_raw(&services, |event| {
            event.handoff(Handoff::to(HandoffTarget::address("127.0.0.1", 30037)));
        });
        let mut peer = peer(b"XFER").await;

        assert_eq!(refused(&services, &mut peer).await, [RejectReason::Banned]);
        assert!(seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_banned_address_is_never_claimed() {
        let dir = tempfile::tempdir().unwrap();
        let mut services = claiming(&[b"XFER"]);
        let bans = Arc::new(BanManager::builtin(
            Arc::new(FileBanStorage::new(dir.path().join("bans.json"))),
            Arc::new(ConnectionRegistry::new()),
            Arc::new(crate::event_bus::EventBusImpl::new()),
        ));
        bans.issue(BanRequest::new(BanTarget::Ip(IpAddr::from([127, 0, 0, 1]))))
            .await
            .unwrap();
        services.ban_manager = bans;
        let seen = on_raw(&services, RawConnectionEvent::pass);
        let mut peer = peer(b"XFER").await;

        assert_eq!(
            refused(&services, &mut peer).await,
            [RejectReason::IpBanned]
        );
        assert!(seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn shutdown_closes_a_connection_still_sending_its_prefix() {
        let services = claiming(&[b"XFER"]);
        let mut peer = peer(b"XF").await;
        let shutdown = CancellationToken::new();
        let cancel = shutdown.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cancel.cancel();
        });

        let started = Instant::now();
        let claimed = claim_raw(
            &services,
            &mut peer.ctx,
            RawClaimLimits::default(),
            &shutdown,
        )
        .await;

        assert_eq!(claimed, RawClaim::Closed);
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}

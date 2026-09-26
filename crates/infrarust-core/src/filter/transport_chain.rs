//! Transport filter chain.

use std::net::SocketAddr;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Duration;

use futures_util::FutureExt;
use infrarust_api::events::handshake::RejectReason;
use infrarust_api::filter::{FilterVerdict, TransportContext, TransportFilter};

use super::registry_base::FilterOwner;
use crate::event_bus::diagnostic::panic_message;

#[derive(Clone)]
pub struct ChainedFilter {
    pub id: String,
    pub owner: FilterOwner,
    pub filter: Arc<dyn TransportFilter>,
}

/// A chain of [`TransportFilter`]s applied to each connection.
///
/// Filters are shared (`Arc`) and the chain is cloned per-connection.
#[derive(Clone)]
pub struct TransportFilterChain {
    filters: Arc<Vec<ChainedFilter>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportFault {
    Panicked(String),
    TimedOut(Duration),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportRejection {
    pub filter_id: String,
    pub owner: FilterOwner,
    pub fault: Option<TransportFault>,
}

impl TransportRejection {
    pub fn reason(&self) -> RejectReason {
        let plugin_id = match &self.owner {
            FilterOwner::Plugin(plugin_id) => Some(plugin_id.clone()),
            FilterOwner::Proxy => None,
        };
        RejectReason::Plugin { plugin_id }
    }

    pub fn log(&self, peer: SocketAddr) {
        let filter = self.filter_id.as_str();
        let plugin = self.owner.name();
        match &self.fault {
            None => {
                tracing::debug!(%peer, filter, plugin, "connection rejected by a transport filter")
            }
            Some(TransportFault::Panicked(panic)) => tracing::warn!(
                %peer,
                filter,
                plugin,
                %panic,
                "transport filter panicked, rejecting the connection"
            ),
            Some(TransportFault::TimedOut(after)) => tracing::warn!(
                %peer,
                filter,
                plugin,
                timeout = ?after,
                "transport filter timed out, rejecting the connection"
            ),
        }
    }
}

pub struct TransportSession {
    ctx: TransportContext,
    accepted: Vec<ChainedFilter>,
}

impl TransportSession {
    pub const fn context(&self) -> &TransportContext {
        &self.ctx
    }
}

impl Drop for TransportSession {
    fn drop(&mut self) {
        for link in self.accepted.iter().rev() {
            let closed = catch_unwind(AssertUnwindSafe(|| link.filter.on_close(&self.ctx)));
            if let Err(payload) = closed {
                tracing::warn!(
                    filter = link.id.as_str(),
                    plugin = link.owner.name(),
                    panic = %panic_message(payload.as_ref()),
                    "transport filter panicked in on_close"
                );
            }
        }
    }
}

impl TransportFilterChain {
    pub fn new(filters: Vec<ChainedFilter>) -> Self {
        Self {
            filters: Arc::new(filters),
        }
    }

    /// Creates an empty chain that accepts everything.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            filters: Arc::new(Vec::new()),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.filters.is_empty()
    }

    pub async fn open(
        &self,
        ctx: TransportContext,
        timeout: Duration,
    ) -> Result<TransportSession, TransportRejection> {
        let mut session = TransportSession {
            ctx,
            accepted: Vec::with_capacity(self.filters.len()),
        };
        for link in self.filters.iter() {
            match accept(link, &mut session.ctx, timeout).await {
                Ok(FilterVerdict::Continue) => session.accepted.push(link.clone()),
                Ok(FilterVerdict::Reject) => return Err(rejection(link, None)),
                Err(fault) => return Err(rejection(link, Some(fault))),
            }
        }
        Ok(session)
    }
}

fn rejection(link: &ChainedFilter, fault: Option<TransportFault>) -> TransportRejection {
    TransportRejection {
        filter_id: link.id.clone(),
        owner: link.owner.clone(),
        fault,
    }
}

async fn accept(
    link: &ChainedFilter,
    ctx: &mut TransportContext,
    timeout: Duration,
) -> Result<FilterVerdict, TransportFault> {
    let filter = link.filter.as_ref();
    let verdict = catch_unwind(AssertUnwindSafe(move || {
        let ctx = ctx;
        filter.on_accept(ctx)
    }))
    .map_err(|payload| TransportFault::Panicked(panic_message(payload.as_ref())))?;
    match tokio::time::timeout(timeout, AssertUnwindSafe(verdict).catch_unwind()).await {
        Ok(Ok(verdict)) => Ok(verdict),
        Ok(Err(payload)) => Err(TransportFault::Panicked(panic_message(payload.as_ref()))),
        Err(_) => Err(TransportFault::TimedOut(timeout)),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use std::net::SocketAddr;
    use std::sync::Mutex;
    use std::time::Instant;

    use infrarust_api::event::BoxFuture;
    use infrarust_api::filter::*;
    use infrarust_api::types::Extensions;

    use super::*;

    const TIMEOUT: Duration = Duration::from_secs(5);

    #[derive(Clone, Copy)]
    enum Act {
        Pass,
        Refuse,
        PanicNow,
        PanicLater,
        Stall,
        PanicOnClose,
    }

    type Journal = Arc<Mutex<Vec<String>>>;

    struct Scripted {
        id: &'static str,
        act: Act,
        journal: Journal,
    }

    impl TransportFilter for Scripted {
        fn metadata(&self) -> FilterMetadata {
            FilterMetadata::new(self.id)
        }

        fn on_accept<'a>(&'a self, ctx: &'a mut TransportContext) -> BoxFuture<'a, FilterVerdict> {
            self.journal
                .lock()
                .unwrap()
                .push(format!("accept {}", self.id));
            ctx.extensions.insert(self.id);
            match self.act {
                Act::PanicNow => panic!("{} refuses to build a future", self.id),
                Act::PanicLater => Box::pin(async move { panic!("{} fails", self.id) }),
                Act::Stall => Box::pin(std::future::pending()),
                Act::Refuse => Box::pin(async { FilterVerdict::Reject }),
                Act::Pass | Act::PanicOnClose => Box::pin(async { FilterVerdict::Continue }),
            }
        }

        fn on_close(&self, ctx: &TransportContext) {
            self.journal.lock().unwrap().push(format!(
                "close {} id={} ext={:?}",
                self.id,
                ctx.connection_id,
                ctx.extensions.get::<&'static str>()
            ));
            if matches!(self.act, Act::PanicOnClose) {
                panic!("{} fails to close", self.id);
            }
        }
    }

    fn chain(journal: &Journal, filters: &[(&'static str, Act)]) -> TransportFilterChain {
        TransportFilterChain::new(
            filters
                .iter()
                .map(|(id, act)| ChainedFilter {
                    id: (*id).to_string(),
                    owner: FilterOwner::plugin("owner"),
                    filter: Arc::new(Scripted {
                        id,
                        act: *act,
                        journal: Arc::clone(journal),
                    }),
                })
                .collect(),
        )
    }

    fn test_ctx() -> TransportContext {
        TransportContext {
            remote_addr: "127.0.0.1:12345".parse::<SocketAddr>().unwrap(),
            local_addr: "0.0.0.0:25565".parse::<SocketAddr>().unwrap(),
            real_ip: None,
            connection_time: Instant::now(),
            connection_id: 7,
            extensions: Extensions::new(),
        }
    }

    fn entries(journal: &Journal) -> Vec<String> {
        journal.lock().unwrap().clone()
    }

    #[tokio::test]
    async fn filters_accept_in_order_and_close_in_reverse_once_the_session_ends() {
        let journal = Journal::default();
        let chain = chain(&journal, &[("first", Act::Pass), ("second", Act::Pass)]);

        let session = chain.open(test_ctx(), TIMEOUT).await.unwrap();
        assert_eq!(entries(&journal), ["accept first", "accept second"]);
        assert_eq!(session.context().connection_id, 7);

        drop(session);
        assert_eq!(
            entries(&journal),
            [
                "accept first",
                "accept second",
                "close second id=7 ext=Some(\"second\")",
                "close first id=7 ext=Some(\"second\")",
            ]
        );
    }

    #[tokio::test]
    async fn a_rejection_closes_the_filters_that_accepted_and_skips_the_rest() {
        let journal = Journal::default();
        let chain = chain(
            &journal,
            &[
                ("first", Act::Pass),
                ("gate", Act::Refuse),
                ("last", Act::Pass),
            ],
        );

        let rejection = chain.open(test_ctx(), TIMEOUT).await.err().unwrap();

        assert_eq!(
            rejection,
            TransportRejection {
                filter_id: "gate".into(),
                owner: FilterOwner::plugin("owner"),
                fault: None,
            }
        );
        assert_eq!(
            rejection.reason(),
            RejectReason::Plugin {
                plugin_id: Some("owner".into())
            }
        );
        assert_eq!(
            entries(&journal),
            [
                "accept first",
                "accept gate",
                "close first id=7 ext=Some(\"gate\")"
            ]
        );
    }

    #[tokio::test]
    async fn a_panic_building_the_future_rejects() {
        let journal = Journal::default();
        let chain = chain(&journal, &[("first", Act::Pass), ("boom", Act::PanicNow)]);

        let rejection = chain.open(test_ctx(), TIMEOUT).await.err().unwrap();

        assert_eq!(rejection.filter_id, "boom");
        assert_eq!(
            rejection.fault,
            Some(TransportFault::Panicked(
                "boom refuses to build a future".into()
            ))
        );
        assert_eq!(
            entries(&journal).last().unwrap(),
            "close first id=7 ext=Some(\"boom\")"
        );
    }

    #[tokio::test]
    async fn a_panic_inside_the_future_rejects() {
        let journal = Journal::default();
        let chain = chain(&journal, &[("boom", Act::PanicLater)]);

        let rejection = chain.open(test_ctx(), TIMEOUT).await.err().unwrap();

        assert_eq!(
            rejection.fault,
            Some(TransportFault::Panicked("boom fails".into()))
        );
        assert_eq!(entries(&journal), ["accept boom"]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_filter_that_never_answers_times_out_and_rejects() {
        let journal = Journal::default();
        let chain = chain(&journal, &[("first", Act::Pass), ("stuck", Act::Stall)]);
        let timeout = Duration::from_millis(250);

        let rejection = chain.open(test_ctx(), timeout).await.err().unwrap();

        assert_eq!(rejection.filter_id, "stuck");
        assert_eq!(rejection.fault, Some(TransportFault::TimedOut(timeout)));
        assert_eq!(
            entries(&journal),
            [
                "accept first",
                "accept stuck",
                "close first id=7 ext=Some(\"stuck\")"
            ]
        );
    }

    #[tokio::test]
    async fn a_panic_in_on_close_does_not_skip_the_other_filters() {
        let journal = Journal::default();
        let chain = chain(
            &journal,
            &[("first", Act::Pass), ("clumsy", Act::PanicOnClose)],
        );

        drop(chain.open(test_ctx(), TIMEOUT).await.unwrap());

        assert_eq!(
            entries(&journal),
            [
                "accept first",
                "accept clumsy",
                "close clumsy id=7 ext=Some(\"clumsy\")",
                "close first id=7 ext=Some(\"clumsy\")",
            ]
        );
    }

    #[tokio::test]
    async fn an_empty_chain_opens_a_session_that_closes_nothing() {
        let session = TransportFilterChain::empty()
            .open(test_ctx(), TIMEOUT)
            .await
            .unwrap();
        assert!(TransportFilterChain::empty().is_empty());
        drop(session);
    }
}

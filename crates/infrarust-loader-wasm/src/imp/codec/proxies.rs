use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use infrarust_api::event::ConnectionState;
use infrarust_api::filter::{
    CodecFilterError, CodecFilterFactory, CodecFilterInstance, CodecSessionInit, CodecVerdict,
    FilterMetadata, FrameOutput,
};
use infrarust_api::types::RawPacket;

use super::quarantine::{Admission, Quarantine, Ticket, Trip};
use super::{CodecInstantiator, CreateFailure, Live, Outcome, cause, convert};
use crate::error::WasmLoaderError;
use crate::rate_limit::SharedRateLimit;

const REPORT_INTERVAL: Duration = Duration::from_secs(60);
const REPORT_BURST: u32 = 10;

struct Shared {
    instantiator: Arc<CodecInstantiator>,
    filter_id: String,
    required: bool,
    quarantine: Option<Quarantine>,
    faults: SharedRateLimit,
    create_failures: SharedRateLimit,
    quarantines: SharedRateLimit,
}

impl Shared {
    fn plugin_id(&self) -> &str {
        self.instantiator.plugin_id()
    }

    fn action(&self) -> &'static str {
        if self.required {
            "closing the connection"
        } else {
            "passing through for this connection-side"
        }
    }

    fn report_fault(&self, op: &str, ip: IpAddr, cause: &str) {
        let Some(suppressed) = self.faults.admit(Instant::now()) else {
            return;
        };
        tracing::warn!(
            plugin = %self.plugin_id(),
            filter = %self.filter_id,
            op,
            client_ip = %ip,
            cause,
            required = self.required,
            suppressed,
            "wasm codec filter trapped; {}",
            self.action()
        );
    }

    fn report_create_failure(&self, ip: IpAddr, error: &WasmLoaderError) {
        let Some(suppressed) = self.create_failures.admit(Instant::now()) else {
            return;
        };
        tracing::error!(
            plugin = %self.plugin_id(),
            filter = %self.filter_id,
            client_ip = %ip,
            error = %error,
            required = self.required,
            suppressed,
            "codec instance create failed; {}",
            self.action()
        );
    }

    fn report_trip(&self, ip: IpAddr, trip: Trip, cause: &str) {
        let Some(suppressed) = self.quarantines.admit(Instant::now()) else {
            return;
        };
        let consequence = if self.required {
            "its connections are refused"
        } else {
            "its connections pass through unfiltered"
        };
        tracing::warn!(
            plugin = %self.plugin_id(),
            filter = %self.filter_id,
            client_ip = %ip,
            faults = trip.faults,
            window = ?trip.window,
            retry_in = ?trip.backoff,
            cause,
            suppressed,
            "wasm codec filter quarantined for a client address; {consequence}"
        );
    }

    fn fault(&self, ticket: Option<&Ticket>, ip: IpAddr, cause: &str) {
        if let (Some(quarantine), Some(ticket)) = (&self.quarantine, ticket)
            && let Some(trip) = quarantine.fault(ticket, Instant::now())
        {
            self.report_trip(ip, trip, cause);
        }
    }

    fn unavailable(&self, reason: String) -> Box<dyn CodecFilterInstance> {
        if self.required {
            Box::new(ClosedInstance { reason })
        } else {
            Box::new(PassthroughInstance)
        }
    }

    fn closing(&self, what: &str) -> String {
        format!(
            "required codec filter `{}` of plugin `{}` {what}",
            self.filter_id,
            self.plugin_id()
        )
    }
}

pub(crate) struct WasmCodecFilterFactory {
    shared: Arc<Shared>,
    factory_id: u64,
    metadata: FilterMetadata,
}

impl WasmCodecFilterFactory {
    pub(crate) fn new(
        instantiator: Arc<CodecInstantiator>,
        factory_id: u64,
        metadata: FilterMetadata,
        required: bool,
    ) -> Self {
        let quarantine = Quarantine::new(instantiator.quarantine());
        Self {
            shared: Arc::new(Shared {
                instantiator,
                filter_id: metadata.id.clone(),
                required,
                quarantine,
                faults: SharedRateLimit::new(REPORT_INTERVAL, REPORT_BURST),
                create_failures: SharedRateLimit::new(REPORT_INTERVAL, REPORT_BURST),
                quarantines: SharedRateLimit::new(REPORT_INTERVAL, REPORT_BURST),
            }),
            factory_id,
            metadata,
        }
    }
}

impl CodecFilterFactory for WasmCodecFilterFactory {
    fn metadata(&self) -> FilterMetadata {
        self.metadata.clone()
    }

    fn create(&self, init: &CodecSessionInit) -> Box<dyn CodecFilterInstance> {
        let shared = &self.shared;
        let ip = init.real_ip.unwrap_or_else(|| init.remote_addr.ip());
        let ticket = match &shared.quarantine {
            Some(quarantine) => match quarantine.admit(ip, Instant::now()) {
                Admission::Open(ticket) => Some(ticket),
                Admission::Quarantined { retry_in } => {
                    return shared.unavailable(shared.closing(&format!(
                        "is quarantined for this client address (retry in {retry_in:?})"
                    )));
                }
            },
            None => None,
        };
        match shared.instantiator.create_live(self.factory_id, init) {
            Ok(live) => Box::new(WasmCodecFilterInstance {
                live: Some(live),
                shared: Arc::clone(shared),
                ticket,
                ip,
                closing: None,
            }),
            Err(CreateFailure::Host(error)) => {
                shared.report_create_failure(ip, &error);
                shared.unavailable(shared.closing("could not be created"))
            }
            Err(CreateFailure::Guest(cause)) => {
                shared.report_fault("create", ip, &cause);
                shared.fault(ticket.as_ref(), ip, &cause);
                shared.unavailable(shared.closing(&cause))
            }
        }
    }
}

pub(crate) struct WasmCodecFilterInstance {
    live: Option<Live>,
    shared: Arc<Shared>,
    ticket: Option<Ticket>,
    ip: IpAddr,
    closing: Option<String>,
}

impl WasmCodecFilterInstance {
    fn usable(&mut self) -> Option<&mut Live> {
        if self.live.is_some() && self.ticket.as_ref().is_some_and(Ticket::is_cut) {
            self.live = None;
            if self.shared.required {
                self.closing = Some(
                    self.shared
                        .closing("is quarantined for this client address"),
                );
            }
        }
        self.live.as_mut()
    }

    fn fail(&mut self, op: &str, error: &wasmtime::Error) {
        let panic = self
            .live
            .take()
            .and_then(|mut live| live.take_guest_panic());
        let cause = cause(error, self.shared.instantiator.budget(), panic);
        self.shared.report_fault(op, self.ip, &cause);
        if self.shared.required {
            self.closing = Some(self.shared.closing(&cause));
        }
        self.shared.fault(self.ticket.as_ref(), self.ip, &cause);
    }

    fn unfiltered(&self) -> CodecVerdict {
        match &self.closing {
            Some(reason) => CodecVerdict::Error(CodecFilterError::Internal(reason.clone())),
            None => CodecVerdict::Pass,
        }
    }

    fn notify(&mut self, op: &str, call: impl FnOnce(&mut Live) -> wasmtime::Result<()>) {
        let Some(live) = self.usable() else {
            return;
        };
        if let Err(error) = call(live) {
            self.fail(op, &error);
        }
    }
}

impl CodecFilterInstance for WasmCodecFilterInstance {
    fn filter(&mut self, packet: &mut RawPacket, output: &mut FrameOutput) -> CodecVerdict {
        let Some(live) = self.usable() else {
            return self.unfiltered();
        };
        match live.filter(packet.packet_id, &packet.data) {
            Ok(Outcome::Pass) => CodecVerdict::Pass,
            Ok(Outcome::Drop) => CodecVerdict::Drop,
            Ok(Outcome::Output(out)) => convert::apply_filter_output(out, packet, output),
            Err(error) => {
                self.fail("filter", &error);
                self.unfiltered()
            }
        }
    }

    fn close_reason(&self) -> Option<&str> {
        self.closing.as_deref()
    }

    fn on_state_change(&mut self, new_state: ConnectionState) {
        let wit_state = convert::connection_state_to_wit(new_state);
        self.notify("on-state-change", |live| live.on_state_change(wit_state));
    }

    fn on_compression_change(&mut self, threshold: i32) {
        self.notify("on-compression-change", |live| {
            live.on_compression_change(threshold)
        });
    }

    fn on_encryption_enabled(&mut self) {
        self.notify("on-encryption-enabled", Live::on_encryption_enabled);
    }

    fn on_close(&mut self) {
        self.notify("on-close", Live::on_close);
        if let Some(live) = self.live.take() {
            live.release();
        }
    }
}

struct PassthroughInstance;

impl CodecFilterInstance for PassthroughInstance {
    fn filter(&mut self, _packet: &mut RawPacket, _output: &mut FrameOutput) -> CodecVerdict {
        CodecVerdict::Pass
    }
}

struct ClosedInstance {
    reason: String,
}

impl CodecFilterInstance for ClosedInstance {
    fn filter(&mut self, _packet: &mut RawPacket, _output: &mut FrameOutput) -> CodecVerdict {
        CodecVerdict::Error(CodecFilterError::Internal(self.reason.clone()))
    }

    fn close_reason(&self) -> Option<&str> {
        Some(&self.reason)
    }
}

pub mod exporter;
pub mod metrics;
pub mod tracing;

pub use exporter::configure_otlp_exporter;
pub use metrics::{init_meter_provider, MeterProviderGuard};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate};
pub use tracing::{init_tracer_provider, TracerProviderGuard};

pub use opentelemetry::global;

use lazy_static::lazy_static;
use metrics::InfrarustMetrics;
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

lazy_static! {
    pub static ref TELEMETRY: InfrarustMetrics = InfrarustMetrics::new();
}

#[derive(Debug, Clone, Copy)]
pub enum Direction {
    Incoming,
    Outgoing,
    Internal,
}

pub fn start_system_metrics_collection() {
    tokio::spawn(async move {
        let mut sys = sysinfo::System::new_all();
        let pid = sysinfo::Pid::from(std::process::id() as usize);
        let cpu_count = sys.cpus().len() as f64;

        loop {
            tokio::time::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL).await;
            sys.refresh_processes_specifics(
                ProcessesToUpdate::All,
                true,
                ProcessRefreshKind::nothing().with_cpu(),
            );
            if let Some(process) = sys.process(pid) {
                let cpu = process.cpu_usage() as f64;
                let memory = process.memory() as f64;

                TELEMETRY.update_system_metrics(
                    f64::from(process.cpu_usage()) / cpu_count,
                    process.memory() as f64,
                    process.tasks().unwrap_or(&HashSet::new()).len() as i64,
                );
            } else {
                TELEMETRY.internal_errors.add(1, &[]);
            }
        }
    });
}

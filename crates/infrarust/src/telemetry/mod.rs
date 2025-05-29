pub mod infrarust_fmt_formatter;
pub mod log_filter;
pub mod log_type_layer;
pub mod tracing;

#[cfg(feature = "telemetry")]
pub mod exporter;
#[cfg(feature = "telemetry")]
pub mod metrics;

#[cfg(feature = "telemetry")]
pub use metrics::{MeterProviderGuard, init_meter_provider};
#[cfg(feature = "telemetry")]
pub use tracing::TracerProviderGuard;

#[cfg(feature = "telemetry")]
pub use opentelemetry::global;

#[cfg(feature = "telemetry")]
use lazy_static::lazy_static;
#[cfg(feature = "telemetry")]
use metrics::InfrarustMetrics;
#[cfg(feature = "telemetry")]
use std::collections::HashSet;

#[cfg(feature = "telemetry")]
lazy_static! {
    pub static ref TELEMETRY: InfrarustMetrics = InfrarustMetrics::new();
}

#[derive(Debug, Clone, Copy)]
pub enum Direction {
    Incoming,
    Outgoing,
    Internal,
}

#[cfg(feature = "telemetry")]
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

#[cfg(not(feature = "telemetry"))]
pub fn start_system_metrics_collection() {
    // No-op when telemetry is disabled
}

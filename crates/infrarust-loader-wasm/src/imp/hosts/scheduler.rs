use std::time::Duration;

use infrarust_api::permissions::Capability;
use infrarust_api::services::scheduler::TaskHandle;

use crate::actor::CallKind;
use crate::bindings::infrarust::plugin::scheduler as wsched;
use crate::host_error::HostResult;
use crate::proxies;
use crate::store_state::PluginStoreState;

impl wsched::Host for PluginStoreState {
    async fn delay(&mut self, after: u64, handler: u64) -> wasmtime::Result<HostResult<u64>> {
        Ok((|| {
            self.check(Capability::Scheduler, "scheduler.delay")?;
            let ctx = self.services()?;
            let instance = self.instance_ref(CallKind::Callback);
            let handle = ctx.scheduler().delay(
                Duration::from_millis(after),
                Box::new(move || proxies::dispatch_scheduled_task(instance, handler)),
            );
            self.record_task(handle.as_u64());
            Ok(handle.as_u64())
        })())
    }

    async fn interval(
        &mut self,
        period: u64,
        initial_delay: Option<u64>,
        handler: u64,
    ) -> wasmtime::Result<HostResult<u64>> {
        Ok((|| {
            self.check(Capability::Scheduler, "scheduler.interval")?;
            let ctx = self.services()?;
            let instance = self.instance_ref(CallKind::Callback);
            let task =
                Box::new(move || proxies::dispatch_scheduled_task(instance.clone(), handler));
            let period = Duration::from_millis(period);
            let handle = match initial_delay {
                Some(delay) => {
                    ctx.scheduler()
                        .interval_with_delay(period, Duration::from_millis(delay), task)
                }
                None => ctx.scheduler().interval(period, task),
            };
            self.record_task(handle.as_u64());
            Ok(handle.as_u64())
        })())
    }

    async fn cancel(&mut self, handle: u64) -> wasmtime::Result<HostResult<()>> {
        Ok((|| {
            self.check(Capability::Scheduler, "scheduler.cancel")?;
            self.forget_task(handle);
            if let Ok(ctx) = self.services() {
                ctx.scheduler().cancel(TaskHandle::new(handle));
            }
            Ok(())
        })())
    }
}

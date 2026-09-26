use std::time::Duration;

use infrarust_api::services::scheduler::TaskHandle;

use crate::actor::CallKind;
use crate::bindings::infrarust::plugin::scheduler as wsched;
use crate::host_error::HostResult;
use crate::proxies;
use crate::store_state::PluginStoreState;

impl wsched::Host for PluginStoreState {
    async fn delay(&mut self, after: u64, handler: u64) -> wasmtime::Result<HostResult<u64>> {
        Ok(self.schedule_delay(after, handler))
    }

    async fn interval(
        &mut self,
        period: u64,
        initial_delay: Option<u64>,
        handler: u64,
    ) -> wasmtime::Result<HostResult<u64>> {
        Ok(self.schedule_interval(period, initial_delay, handler))
    }

    async fn cancel(&mut self, handle: u64) -> wasmtime::Result<HostResult<()>> {
        Ok(self.cancel_task(handle))
    }
}

impl PluginStoreState {
    fn schedule_delay(&mut self, after: u64, handler: u64) -> HostResult<u64> {
        self.check("scheduler", "delay")?;
        let ctx = self.services()?;
        let instance = self.instance_ref(CallKind::Callback)?;
        let handle = ctx.scheduler().delay(
            Duration::from_millis(after),
            Box::new(move || {
                proxies::dispatch_scheduled_task(instance, handler);
                Box::pin(async {})
            }),
        );
        self.record_task(handle.as_u64());
        Ok(handle.as_u64())
    }

    fn schedule_interval(
        &mut self,
        period: u64,
        initial_delay: Option<u64>,
        handler: u64,
    ) -> HostResult<u64> {
        self.check("scheduler", "interval")?;
        let ctx = self.services()?;
        let instance = self.instance_ref(CallKind::Callback)?;
        let handle = ctx.scheduler().repeat(
            Duration::from_millis(period),
            initial_delay.map(Duration::from_millis),
            Box::new(move || {
                proxies::dispatch_scheduled_task(instance.clone(), handler);
                Box::pin(async {})
            }),
        );
        self.record_task(handle.as_u64());
        Ok(handle.as_u64())
    }

    fn cancel_task(&mut self, handle: u64) -> HostResult<()> {
        self.check("scheduler", "cancel")?;
        self.forget_task(handle);
        if let Ok(ctx) = self.services() {
            ctx.scheduler().cancel(TaskHandle::new(handle));
        }
        Ok(())
    }
}

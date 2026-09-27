use std::time::Duration;

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
        let (id, tasks) = self.reserve_task();
        let handle = ctx.scheduler().delay(
            Duration::from_millis(after),
            Box::new(move || {
                tasks.fired(id);
                proxies::dispatch_scheduled_task(instance, handler);
                Box::pin(async {})
            }),
        );
        self.bind_task(id, handle);
        Ok(id)
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
        let (id, _) = self.reserve_task();
        let handle = ctx.scheduler().repeat(
            Duration::from_millis(period),
            initial_delay.map(Duration::from_millis),
            Box::new(move || {
                proxies::dispatch_scheduled_task(instance.clone(), handler);
                Box::pin(async {})
            }),
        );
        self.bind_task(id, handle);
        Ok(id)
    }

    fn cancel_task(&mut self, id: u64) -> HostResult<()> {
        self.check("scheduler", "cancel")?;
        if let Some(handle) = self.take_task(id)
            && let Ok(ctx) = self.services()
        {
            ctx.scheduler().cancel(handle);
        }
        Ok(())
    }
}

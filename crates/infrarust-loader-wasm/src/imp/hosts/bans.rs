use std::time::Duration;

use infrarust_api::permissions::Capability;
use infrarust_api::services::ban_service::{BanQuery, BanRequest, BanSource, UnbanRequest};

use super::await_service;
use crate::bindings::infrarust::plugin::ban_service as wb;
use crate::convert;
use crate::host_error::HostResult;
use crate::store_state::PluginStoreState;

impl wb::Host for PluginStoreState {
    async fn ban(&mut self, request: wb::BanRequest) -> wasmtime::Result<HostResult<wb::BanEntry>> {
        Ok(self.ban_target(request).await)
    }

    async fn unban(
        &mut self,
        target: wb::BanTarget,
    ) -> wasmtime::Result<HostResult<Option<wb::BanEntry>>> {
        Ok(self.unban_target(target).await)
    }

    async fn get(
        &mut self,
        target: wb::BanTarget,
    ) -> wasmtime::Result<HostResult<Option<wb::BanEntry>>> {
        Ok(self.ban_of(target).await)
    }

    async fn list(
        &mut self,
        cursor: Option<String>,
        limit: u32,
    ) -> wasmtime::Result<HostResult<wb::BanPage>> {
        Ok(self.ban_page(cursor, limit).await)
    }
}

impl PluginStoreState {
    fn plugin_source(&self) -> BanSource {
        BanSource::Plugin(self.plugin_id.clone())
    }

    async fn ban_target(&mut self, request: wb::BanRequest) -> HostResult<wb::BanEntry> {
        self.check(Capability::Ban, "ban-service.ban")?;
        let ctx = self.services()?;
        let mut native = BanRequest::new(convert::ban_target_from_wit(request.target)?)
            .kick(request.kick)
            .silent(request.silent)
            .source(self.plugin_source());
        native.reason = request.reason;
        native.duration = request.duration_ms.map(Duration::from_millis);
        let entry = await_service(self.service_call_limit(), ctx.ban_service().ban(native)).await?;
        Ok(convert::ban_entry_to_wit(&entry))
    }

    async fn unban_target(&mut self, target: wb::BanTarget) -> HostResult<Option<wb::BanEntry>> {
        self.check(Capability::Ban, "ban-service.unban")?;
        let ctx = self.services()?;
        let request =
            UnbanRequest::new(convert::ban_target_from_wit(target)?).source(self.plugin_source());
        let removed =
            await_service(self.service_call_limit(), ctx.ban_service().unban(request)).await?;
        Ok(removed.as_ref().map(convert::ban_entry_to_wit))
    }

    async fn ban_of(&mut self, target: wb::BanTarget) -> HostResult<Option<wb::BanEntry>> {
        self.check(Capability::Ban, "ban-service.get")?;
        let ctx = self.services()?;
        let target = convert::ban_target_from_wit(target)?;
        let entry =
            await_service(self.service_call_limit(), ctx.ban_service().get(&target)).await?;
        Ok(entry.as_ref().map(convert::ban_entry_to_wit))
    }

    async fn ban_page(&mut self, cursor: Option<String>, limit: u32) -> HostResult<wb::BanPage> {
        self.check(Capability::Ban, "ban-service.list")?;
        let ctx = self.services()?;
        let mut query = BanQuery::new().limit(usize::try_from(limit).unwrap_or(usize::MAX));
        if let Some(cursor) = cursor {
            query = query.after(cursor);
        }
        let page = await_service(self.service_call_limit(), ctx.ban_service().list(query)).await?;
        Ok(wb::BanPage {
            entries: page.entries.iter().map(convert::ban_entry_to_wit).collect(),
            next_cursor: page.next_cursor,
        })
    }
}

use std::sync::Arc;

use infrarust_api::error::ServiceError;
use infrarust_api::event::BoxFuture;
use infrarust_api::services::ban_service::{
    BanEntry, BanFeatures, BanPage, BanQuery, BanRequest, BanService, BanSource, BanTarget,
    BanVerdict, LoginAttempt, UnbanRequest,
};

pub struct PluginBanService {
    inner: Arc<dyn BanService>,
    plugin_id: String,
}

impl PluginBanService {
    pub fn new(inner: Arc<dyn BanService>, plugin_id: impl Into<String>) -> Self {
        Self {
            inner,
            plugin_id: plugin_id.into(),
        }
    }

    fn source(&self) -> BanSource {
        BanSource::Plugin(self.plugin_id.clone())
    }
}

impl infrarust_api::services::ban_service::private::Sealed for PluginBanService {}

impl BanService for PluginBanService {
    fn check<'a>(
        &'a self,
        attempt: &'a LoginAttempt,
    ) -> BoxFuture<'a, Result<Option<BanVerdict>, ServiceError>> {
        self.inner.check(attempt)
    }

    fn ban(&self, mut request: BanRequest) -> BoxFuture<'_, Result<BanEntry, ServiceError>> {
        request.source.get_or_insert_with(|| self.source());
        self.inner.ban(request)
    }

    fn unban(
        &self,
        mut request: UnbanRequest,
    ) -> BoxFuture<'_, Result<Option<BanEntry>, ServiceError>> {
        request.source.get_or_insert_with(|| self.source());
        self.inner.unban(request)
    }

    fn get<'a>(
        &'a self,
        target: &'a BanTarget,
    ) -> BoxFuture<'a, Result<Option<BanEntry>, ServiceError>> {
        self.inner.get(target)
    }

    fn list(&self, query: BanQuery) -> BoxFuture<'_, Result<BanPage, ServiceError>> {
        self.inner.list(query)
    }

    fn features(&self) -> BanFeatures {
        self.inner.features()
    }
}

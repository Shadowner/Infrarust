use std::sync::Arc;

use infrarust_api::event::BoxFuture;
use infrarust_config::ServerConfig;

use crate::provider::RoutesListener;
use crate::routing::DomainRouter;
use crate::status::{FaviconCache, StatusCache};

pub(super) struct StatusCacheRefresh {
    pub(super) router: Arc<DomainRouter>,
    pub(super) status_cache: Arc<StatusCache>,
    pub(super) favicon_cache: Arc<FaviconCache>,
}

impl RoutesListener for StatusCacheRefresh {
    fn routes_changed(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.status_cache.invalidate_all();

            let favicon_configs: Vec<(String, Arc<ServerConfig>)> = self
                .router
                .list_all()
                .into_iter()
                .map(|(_pid, cfg)| (cfg.effective_id(), cfg))
                .collect();
            if let Err(e) = self.favicon_cache.reload(&favicon_configs, None).await {
                tracing::warn!(error = %e, "failed to reload favicons after config change");
            }
        })
    }
}

//! Ban storage trait definition.

use infrarust_api::event::BoxFuture;
use infrarust_api::services::ban_service::{BanPage, BanQuery, LoginAttempt};

use crate::ban::types::{BanEntry, BanSource, BanTarget};
use crate::error::CoreError;

pub trait BanStorage: Send + Sync {
    fn add_ban(&self, entry: BanEntry) -> BoxFuture<'_, Result<BanEntry, CoreError>>;

    fn remove_ban<'a>(
        &'a self,
        target: &'a BanTarget,
        source: &'a BanSource,
    ) -> BoxFuture<'a, Result<Option<BanEntry>, CoreError>>;

    fn get_ban<'a>(
        &'a self,
        target: &'a BanTarget,
    ) -> BoxFuture<'a, Result<Option<BanEntry>, CoreError>>;

    fn check<'a>(
        &'a self,
        attempt: &'a LoginAttempt,
    ) -> BoxFuture<'a, Result<Option<BanEntry>, CoreError>>;

    fn list(&self, query: BanQuery) -> BoxFuture<'_, Result<BanPage, CoreError>>;

    fn get_all_active(&self) -> BoxFuture<'_, Result<Vec<BanEntry>, CoreError>>;

    fn purge_expired(&self) -> BoxFuture<'_, Result<usize, CoreError>>;

    fn load(&self) -> BoxFuture<'_, Result<(), CoreError>>;

    fn save(&self) -> BoxFuture<'_, Result<(), CoreError>>;
}

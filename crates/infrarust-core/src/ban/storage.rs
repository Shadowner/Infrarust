//! Ban storage trait definition.

use std::future::Future;
use std::pin::Pin;

use infrarust_api::services::ban_service::{BanPage, BanQuery, LoginAttempt};

use crate::ban::types::{BanEntry, BanSource, BanTarget};
use crate::error::CoreError;

pub type StorageFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, CoreError>> + Send + 'a>>;

pub trait BanStorage: Send + Sync {
    fn add_ban(&self, entry: BanEntry) -> StorageFuture<'_, BanEntry>;

    fn remove_ban<'a>(
        &'a self,
        target: &'a BanTarget,
        source: &'a BanSource,
    ) -> StorageFuture<'a, Option<BanEntry>>;

    fn get_ban<'a>(&'a self, target: &'a BanTarget) -> StorageFuture<'a, Option<BanEntry>>;

    fn check<'a>(&'a self, attempt: &'a LoginAttempt) -> StorageFuture<'a, Option<BanEntry>>;

    fn list(&self, query: BanQuery) -> StorageFuture<'_, BanPage>;

    fn get_all_active(&self) -> StorageFuture<'_, Vec<BanEntry>>;

    fn purge_expired(&self) -> StorageFuture<'_, usize>;

    fn load(&self) -> StorageFuture<'_, ()>;

    fn save(&self) -> StorageFuture<'_, ()>;
}

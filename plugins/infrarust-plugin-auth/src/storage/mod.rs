pub mod json;

use std::net::IpAddr;

use chrono::{DateTime, Utc};
use infrarust_api::event::BoxFuture;

use crate::account::{AuthAccount, PasswordHash, PremiumInfo, Username};
use crate::error::AuthStorageError;

pub trait AuthStorage: Send + Sync {
    fn has_account<'a>(
        &'a self,
        username: &'a Username,
    ) -> BoxFuture<'a, Result<bool, AuthStorageError>>;

    fn get_account<'a>(
        &'a self,
        username: &'a Username,
    ) -> BoxFuture<'a, Result<Option<AuthAccount>, AuthStorageError>>;

    fn create_account<'a>(
        &'a self,
        account: &'a AuthAccount,
    ) -> BoxFuture<'a, Result<(), AuthStorageError>>;

    fn update_password_hash<'a>(
        &'a self,
        username: &'a Username,
        new_hash: PasswordHash,
    ) -> BoxFuture<'a, Result<(), AuthStorageError>>;

    fn delete_account<'a>(
        &'a self,
        username: &'a Username,
    ) -> BoxFuture<'a, Result<bool, AuthStorageError>>;

    fn update_last_login<'a>(
        &'a self,
        username: &'a Username,
        ip: IpAddr,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<(), AuthStorageError>>;

    fn update_premium_info<'a>(
        &'a self,
        username: &'a Username,
        premium_info: Option<PremiumInfo>,
    ) -> BoxFuture<'a, Result<(), AuthStorageError>>;

    fn flush(&self) -> BoxFuture<'_, Result<(), AuthStorageError>>;

    fn get_account_blocking(
        &self,
        username: &Username,
    ) -> Result<Option<AuthAccount>, AuthStorageError>;

    fn has_account_blocking(&self, username: &Username) -> bool;

    fn is_force_cracked_blocking(&self, username: &Username) -> bool;
}

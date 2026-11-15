pub mod json;

use std::net::IpAddr;

use chrono::{DateTime, Utc};
use infrarust_api::event::BoxFuture;

use crate::account::{AuthAccount, PasswordHash, PremiumInfo, Username};
use crate::error::AuthStorageError;

pub trait AuthStorage: Send + Sync {
    fn has_account(&self, username: &Username) -> bool;

    fn get_account(&self, username: &Username) -> Result<Option<AuthAccount>, AuthStorageError>;

    fn is_force_cracked(&self, username: &Username) -> bool;

    fn create_account(&self, account: &AuthAccount) -> Result<(), AuthStorageError>;

    fn update_password_hash(
        &self,
        username: &Username,
        new_hash: PasswordHash,
    ) -> Result<(), AuthStorageError>;

    fn delete_account(&self, username: &Username) -> Result<bool, AuthStorageError>;

    fn update_last_login(
        &self,
        username: &Username,
        ip: IpAddr,
        now: DateTime<Utc>,
    ) -> Result<(), AuthStorageError>;

    fn update_premium_info(
        &self,
        username: &Username,
        premium_info: Option<PremiumInfo>,
    ) -> Result<(), AuthStorageError>;

    fn flush(&self) -> BoxFuture<'_, Result<(), AuthStorageError>>;
}

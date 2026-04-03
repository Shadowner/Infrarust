//! JSON file storage backend backed by `DashMap`.

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use infrarust_api::event::BoxFuture;

use crate::account::{AuthAccount, PasswordHash, PremiumInfo, Username};
use crate::error::AuthStorageError;

use super::AuthStorage;

pub struct JsonFileStorage {
    accounts: DashMap<Username, AuthAccount>,
    file_path: PathBuf,
    dirty: AtomicBool,
}

impl JsonFileStorage {
    pub async fn load_or_create(data_dir: &Path, filename: &str) -> Result<Self, AuthStorageError> {
        let file_path = data_dir.join(filename);

        let accounts = DashMap::new();

        if file_path.exists() {
            let content = tokio::fs::read_to_string(&file_path).await?;
            let loaded: Vec<AuthAccount> = serde_json::from_str(&content)
                .map_err(|e| AuthStorageError::Serialization(e.to_string()))?;
            for account in loaded {
                accounts.insert(account.username.clone(), account);
            }
            tracing::info!(
                count = accounts.len(),
                path = %file_path.display(),
                "Loaded auth accounts from disk"
            );
        } else {
            // Ensure the data directory exists
            tokio::fs::create_dir_all(data_dir).await?;
            tracing::info!(
                path = %file_path.display(),
                "No existing accounts file — starting fresh"
            );
        }

        Ok(Self {
            accounts,
            file_path,
            dirty: AtomicBool::new(false),
        })
    }

    fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Relaxed);
    }
}

impl AuthStorage for JsonFileStorage {
    fn has_account<'a>(
        &'a self,
        username: &'a Username,
    ) -> BoxFuture<'a, Result<bool, AuthStorageError>> {
        Box::pin(async move { Ok(self.accounts.contains_key(username)) })
    }

    fn get_account<'a>(
        &'a self,
        username: &'a Username,
    ) -> BoxFuture<'a, Result<Option<AuthAccount>, AuthStorageError>> {
        Box::pin(async move {
            Ok(self
                .accounts
                .get(username)
                .map(|entry| entry.value().clone()))
        })
    }

    fn create_account<'a>(
        &'a self,
        account: &'a AuthAccount,
    ) -> BoxFuture<'a, Result<(), AuthStorageError>> {
        Box::pin(async move {
            use dashmap::mapref::entry::Entry;

            match self.accounts.entry(account.username.clone()) {
                Entry::Occupied(_) => Err(AuthStorageError::AccountAlreadyExists {
                    username: account.username.to_string(),
                }),
                Entry::Vacant(entry) => {
                    entry.insert(account.clone());
                    self.mark_dirty();
                    Ok(())
                }
            }
        })
    }

    fn update_password_hash<'a>(
        &'a self,
        username: &'a Username,
        new_hash: PasswordHash,
    ) -> BoxFuture<'a, Result<(), AuthStorageError>> {
        Box::pin(async move {
            match self.accounts.get_mut(username) {
                Some(mut entry) => {
                    entry.password_hash = Some(new_hash);
                    self.mark_dirty();
                    Ok(())
                }
                None => Err(AuthStorageError::AccountNotFound {
                    username: username.to_string(),
                }),
            }
        })
    }

    fn delete_account<'a>(
        &'a self,
        username: &'a Username,
    ) -> BoxFuture<'a, Result<bool, AuthStorageError>> {
        Box::pin(async move {
            let existed = self.accounts.remove(username).is_some();
            if existed {
                self.mark_dirty();
            }
            Ok(existed)
        })
    }

    fn update_last_login<'a>(
        &'a self,
        username: &'a Username,
        ip: IpAddr,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<(), AuthStorageError>> {
        Box::pin(async move {
            if let Some(mut entry) = self.accounts.get_mut(username) {
                entry.last_login = Some(now);
                entry.last_ip = Some(ip);
                entry.login_count += 1;
                self.mark_dirty();
            }
            Ok(())
        })
    }

    fn flush(&self) -> BoxFuture<'_, Result<(), AuthStorageError>> {
        Box::pin(async move {
            if !self.dirty.swap(false, Ordering::Relaxed) {
                return Ok(());
            }

            let accounts: Vec<AuthAccount> = self
                .accounts
                .iter()
                .map(|entry| entry.value().clone())
                .collect();

            let json = serde_json::to_string_pretty(&accounts)
                .map_err(|e| AuthStorageError::Serialization(e.to_string()))?;

            let tmp_path = self.file_path.with_extension("json.tmp");
            tokio::fs::write(&tmp_path, json.as_bytes()).await?;
            tokio::fs::rename(&tmp_path, &self.file_path).await?;

            tracing::debug!(
                count = accounts.len(),
                path = %self.file_path.display(),
                "Flushed auth accounts to disk"
            );
            Ok(())
        })
    }

    fn get_account_blocking(
        &self,
        username: &Username,
    ) -> Result<Option<AuthAccount>, AuthStorageError> {
        Ok(self
            .accounts
            .get(username)
            .map(|entry| entry.value().clone()))
    }

    fn has_account_blocking(&self, username: &Username) -> bool {
        self.accounts.contains_key(username)
    }

    fn update_premium_info<'a>(
        &'a self,
        username: &'a Username,
        premium_info: Option<PremiumInfo>,
    ) -> BoxFuture<'a, Result<(), AuthStorageError>> {
        Box::pin(async move {
            match self.accounts.get_mut(username) {
                Some(mut entry) => {
                    entry.premium_info = premium_info;
                    self.mark_dirty();
                    Ok(())
                }
                None => Err(AuthStorageError::AccountNotFound {
                    username: username.to_string(),
                }),
            }
        })
    }

    fn is_force_cracked_blocking(&self, username: &Username) -> bool {
        self.accounts
            .get(username)
            .and_then(|entry| entry.premium_info.as_ref().map(|pi| pi.force_cracked))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::DisplayName;
    use tempfile::TempDir;

    fn test_account(name: &str) -> AuthAccount {
        AuthAccount {
            username: Username::new(name),
            display_name: DisplayName::new(name),
            password_hash: Some(PasswordHash::new("$argon2id$test")),
            registered_at: Utc::now(),
            last_login: None,
            last_ip: None,
            login_count: 0,
            premium_info: None,
        }
    }

    #[tokio::test]
    async fn create_and_get_account() {
        let dir = TempDir::new().unwrap();
        let storage = JsonFileStorage::load_or_create(dir.path(), "accounts.json")
            .await
            .unwrap();

        let account = test_account("TestPlayer");
        storage.create_account(&account).await.unwrap();

        let fetched = storage
            .get_account(&Username::new("testplayer"))
            .await
            .unwrap();
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().display_name.as_str(), "TestPlayer");
    }

    #[tokio::test]
    async fn create_duplicate_fails() {
        let dir = TempDir::new().unwrap();
        let storage = JsonFileStorage::load_or_create(dir.path(), "accounts.json")
            .await
            .unwrap();

        let account = test_account("DuplicateUser");
        storage.create_account(&account).await.unwrap();

        let result = storage.create_account(&account).await;
        assert!(matches!(
            result,
            Err(AuthStorageError::AccountAlreadyExists { .. })
        ));
    }

    #[tokio::test]
    async fn delete_account_returns_existed() {
        let dir = TempDir::new().unwrap();
        let storage = JsonFileStorage::load_or_create(dir.path(), "accounts.json")
            .await
            .unwrap();

        let account = test_account("DeleteMe");
        storage.create_account(&account).await.unwrap();

        assert!(
            storage
                .delete_account(&Username::new("deleteme"))
                .await
                .unwrap()
        );
        assert!(
            !storage
                .delete_account(&Username::new("deleteme"))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn flush_and_reload() {
        let dir = TempDir::new().unwrap();

        // Write
        {
            let storage = JsonFileStorage::load_or_create(dir.path(), "accounts.json")
                .await
                .unwrap();
            storage
                .create_account(&test_account("Persistent"))
                .await
                .unwrap();
            storage.flush().await.unwrap();
        }

        // Reload
        {
            let storage = JsonFileStorage::load_or_create(dir.path(), "accounts.json")
                .await
                .unwrap();
            assert!(storage.has_account_blocking(&Username::new("persistent")));
        }
    }

    #[tokio::test]
    async fn sync_accessors_work() {
        let dir = TempDir::new().unwrap();
        let storage = JsonFileStorage::load_or_create(dir.path(), "accounts.json")
            .await
            .unwrap();

        let account = test_account("SyncTest");
        storage.create_account(&account).await.unwrap();

        assert!(storage.has_account_blocking(&Username::new("synctest")));
        let fetched = storage
            .get_account_blocking(&Username::new("synctest"))
            .unwrap();
        assert!(fetched.is_some());
    }

    #[tokio::test]
    async fn update_password_hash_works() {
        let dir = TempDir::new().unwrap();
        let storage = JsonFileStorage::load_or_create(dir.path(), "accounts.json")
            .await
            .unwrap();

        storage
            .create_account(&test_account("HashUpdate"))
            .await
            .unwrap();

        let new_hash = PasswordHash::new("$argon2id$new-hash");
        storage
            .update_password_hash(&Username::new("hashupdate"), new_hash)
            .await
            .unwrap();

        let account = storage
            .get_account(&Username::new("hashupdate"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            account.password_hash.as_ref().map(|h| h.as_str()),
            Some("$argon2id$new-hash")
        );
    }
}

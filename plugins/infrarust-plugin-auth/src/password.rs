//! Password hashing and verification via `spawn_blocking`.

use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use rand_core::OsRng;

use crate::account::PasswordHash as AuthPasswordHash;
use crate::config::HashingConfig;
use crate::error::AuthError;

pub fn validate_hashing_config(config: &HashingConfig) -> Result<(), AuthError> {
    argon2::Params::new(
        config.argon2_memory_cost,
        config.argon2_time_cost,
        config.argon2_parallelism,
        None,
    )
    .map_err(|e| AuthError::Hashing(format!("invalid Argon2 parameters: {e}")))?;
    Ok(())
}

pub async fn hash_password(
    password: &str,
    config: &HashingConfig,
) -> Result<AuthPasswordHash, AuthError> {
    let password = password.to_string();
    let memory_cost = config.argon2_memory_cost;
    let time_cost = config.argon2_time_cost;
    let parallelism = config.argon2_parallelism;

    tokio::task::spawn_blocking(move || {
        let salt = SaltString::generate(&mut OsRng);
        let params = argon2::Params::new(memory_cost, time_cost, parallelism, None)
            .map_err(|e| AuthError::Hashing(e.to_string()))?;
        let argon2 = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
        let hash = argon2
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| AuthError::Hashing(e.to_string()))?;
        Ok(AuthPasswordHash::new(hash.to_string()))
    })
    .await
    .map_err(|e| AuthError::Hashing(format!("spawn_blocking failed: {e}")))?
}

pub async fn verify_password(
    password: &str,
    hash: &AuthPasswordHash,
) -> Result<bool, AuthError> {
    let password = password.to_string();
    let hash_str = hash.as_str().to_string();

    tokio::task::spawn_blocking(move || verify_password_sync(&password, &hash_str))
        .await
        .map_err(|e| AuthError::Hashing(format!("spawn_blocking failed: {e}")))?
}

/// Verify and optionally migrate a bcrypt hash to Argon2id on success.
pub async fn verify_and_migrate(
    password: &str,
    hash: &AuthPasswordHash,
    config: &HashingConfig,
) -> Result<(bool, Option<AuthPasswordHash>), AuthError> {
    let verified = verify_password(password, hash).await?;

    if verified && config.migrate_legacy_hashes && is_bcrypt_hash(hash.as_str()) {
        let new_hash = hash_password(password, config).await?;
        Ok((true, Some(new_hash)))
    } else {
        Ok((verified, None))
    }
}

/// Pre-generated hash used when verifying non-existent accounts (timing-attack resistance).
pub async fn generate_dummy_hash(config: &HashingConfig) -> Result<AuthPasswordHash, AuthError> {
    hash_password("dummy-timing-attack-resistance-value", config).await
}

fn verify_password_sync(password: &str, hash_str: &str) -> Result<bool, AuthError> {
    if hash_str.starts_with("$argon2") {
        let parsed = PasswordHash::new(hash_str)
            .map_err(|e| AuthError::Hashing(e.to_string()))?;
        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok())
    } else if is_bcrypt_hash(hash_str) {
        bcrypt::verify(password, hash_str).map_err(|e| AuthError::Hashing(e.to_string()))
    } else {
        let prefix_len = hash_str.len().min(8);
        Err(AuthError::Hashing(format!(
            "unsupported hash format: {}",
            &hash_str[..prefix_len]
        )))
    }
}

fn is_bcrypt_hash(hash: &str) -> bool {
    hash.starts_with("$2b$") || hash.starts_with("$2a$")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> HashingConfig {
        HashingConfig {
            argon2_memory_cost: 4096, // Low for tests
            argon2_time_cost: 1,
            argon2_parallelism: 1,
            migrate_legacy_hashes: true,
        }
    }

    #[tokio::test]
    async fn test_argon2id_hash_and_verify() {
        let config = test_config();
        let hash = hash_password("correct-password", &config).await.unwrap();
        assert!(hash.as_str().starts_with("$argon2id$"));

        let ok = verify_password("correct-password", &hash).await.unwrap();
        assert!(ok);
    }

    #[tokio::test]
    async fn test_argon2id_wrong_password_returns_false() {
        let config = test_config();
        let hash = hash_password("correct-password", &config).await.unwrap();

        let ok = verify_password("wrong-password", &hash).await.unwrap();
        assert!(!ok);
    }

    #[tokio::test]
    async fn test_bcrypt_verify() {
        let bcrypt_hash = bcrypt::hash("bcrypt-test", 4).unwrap();
        let hash = AuthPasswordHash::new(bcrypt_hash);

        let ok = verify_password("bcrypt-test", &hash).await.unwrap();
        assert!(ok);

        let bad = verify_password("wrong", &hash).await.unwrap();
        assert!(!bad);
    }

    #[tokio::test]
    async fn test_verify_and_migrate_bcrypt_to_argon2() {
        let config = test_config();
        let bcrypt_hash = bcrypt::hash("migrate-me", 4).unwrap();
        let hash = AuthPasswordHash::new(bcrypt_hash);

        let (verified, new_hash) = verify_and_migrate("migrate-me", &hash, &config)
            .await
            .unwrap();
        assert!(verified);
        assert!(new_hash.is_some());
        let new_hash = new_hash.unwrap();
        assert!(new_hash.as_str().starts_with("$argon2id$"));

        // Verify the migrated hash works
        let ok = verify_password("migrate-me", &new_hash).await.unwrap();
        assert!(ok);
    }

    #[tokio::test]
    async fn test_verify_and_migrate_argon2_no_migration() {
        let config = test_config();
        let hash = hash_password("already-argon2", &config).await.unwrap();

        let (verified, new_hash) = verify_and_migrate("already-argon2", &hash, &config)
            .await
            .unwrap();
        assert!(verified);
        assert!(new_hash.is_none()); // No migration needed
    }

    #[tokio::test]
    async fn test_dummy_hash_generation() {
        let config = test_config();
        let dummy = generate_dummy_hash(&config).await.unwrap();
        assert!(dummy.as_str().starts_with("$argon2id$"));
    }

    #[tokio::test]
    async fn test_unsupported_hash_format_returns_error() {
        let hash = AuthPasswordHash::new("$sha256$notsupported");
        let result = verify_password("test", &hash).await;
        assert!(result.is_err());
    }
}

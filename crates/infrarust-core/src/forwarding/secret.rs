//! Forwarding secret file management.

use std::io::Write;
use std::path::Path;

use rand::Rng;

const GENERATED_SECRET_LENGTH: usize = 12;

pub fn load_or_generate_secret(path: &Path) -> Result<Vec<u8>, std::io::Error> {
    if path.exists() {
        let content = std::fs::read_to_string(path)?;
        let trimmed = content.trim();
        if trimmed.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("forwarding secret file is empty: {}", path.display()),
            ));
        }
        Ok(trimmed.as_bytes().to_vec())
    } else {
        let secret: String = rand::thread_rng()
            .sample_iter(&rand::distributions::Alphanumeric)
            .take(GENERATED_SECRET_LENGTH)
            .map(char::from)
            .collect();

        if let Some(parent) = path.parent().filter(|p| !p.exists()) {
            std::fs::create_dir_all(parent)?;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)?;
            file.write_all(secret.as_bytes())?;
        }
        #[cfg(not(unix))]
        {
            std::fs::write(path, &secret)?;
        }
        tracing::info!(
            path = %path.display(),
            "Generated new forwarding secret. \
             Copy this file to your backend servers \
             (Paper: config/paper-global.yml → proxies.velocity.secret)"
        );
        Ok(secret.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use std::fs;

    #[test]
    fn test_secret_generation_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.secret");

        let secret = load_or_generate_secret(&path).unwrap();
        assert_eq!(secret.len(), 12);
        assert!(path.exists());

        let s = String::from_utf8(secret).unwrap();
        assert!(s.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn test_secret_loaded_from_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("existing.secret");
        fs::write(&path, "my-custom-secret").unwrap();

        let secret = load_or_generate_secret(&path).unwrap();
        assert_eq!(secret, b"my-custom-secret");
    }

    #[test]
    fn test_secret_trims_whitespace() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("whitespace.secret");
        fs::write(&path, "  secret-with-spaces  \n").unwrap();

        let secret = load_or_generate_secret(&path).unwrap();
        assert_eq!(secret, b"secret-with-spaces");
    }

    #[test]
    fn test_secret_empty_file_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.secret");
        fs::write(&path, "   \n").unwrap();

        let result = load_or_generate_secret(&path);
        assert!(result.is_err());
    }

    #[test]
    fn test_secret_idempotent_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("idem.secret");

        let secret1 = load_or_generate_secret(&path).unwrap();
        let secret2 = load_or_generate_secret(&path).unwrap();
        assert_eq!(secret1, secret2);
    }
}

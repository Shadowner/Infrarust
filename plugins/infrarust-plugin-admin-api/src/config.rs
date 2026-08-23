#[derive(Debug, Clone)]
pub struct ApiConfig {
    pub bind: String,
    pub api_key: String,
    pub cors_origins: Vec<String>,
    pub rate_limit: RateLimitConfig,
}

#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    pub requests_per_minute: u64,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            requests_per_minute: 60,
        }
    }
}

impl ApiConfig {
    /// Constant-time API key verification.
    /// Both values are right-padded to equal length to prevent length-leaking.
    /// An empty configured key never authenticates (defense-in-depth: the
    /// config layer should already refuse to start without a key).
    pub fn verify_api_key(&self, token: &str) -> bool {
        use subtle::ConstantTimeEq;

        if self.api_key.is_empty() {
            return false;
        }

        let key = self.api_key.as_bytes();
        let tok = token.as_bytes();
        let len = key.len().max(tok.len());

        let mut key_padded = vec![0u8; len];
        let mut tok_padded = vec![0u8; len];
        key_padded[..key.len()].copy_from_slice(key);
        tok_padded[..tok.len()].copy_from_slice(tok);

        let len_eq = key.len() == tok.len();
        let content_eq: bool = key_padded.ct_eq(&tok_padded).into();

        len_eq & content_eq
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(api_key: &str) -> ApiConfig {
        ApiConfig {
            bind: "127.0.0.1:0".into(),
            api_key: api_key.into(),
            cors_origins: vec![],
            rate_limit: RateLimitConfig::default(),
        }
    }

    #[test]
    fn verify_accepts_exact_key() {
        assert!(config("secret-key-1234").verify_api_key("secret-key-1234"));
    }

    #[test]
    fn verify_rejects_wrong_and_truncated_keys() {
        let cfg = config("secret-key-1234");
        assert!(!cfg.verify_api_key("wrong-key-12345"));
        assert!(!cfg.verify_api_key("secret-key-123"));
        assert!(!cfg.verify_api_key(""));
    }

    #[test]
    fn empty_configured_key_never_authenticates() {
        let cfg = config("");
        assert!(!cfg.verify_api_key(""));
        assert!(!cfg.verify_api_key("anything"));
    }
}

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
    pub fn verify_api_key(&self, token: &str) -> bool {
        use subtle::ConstantTimeEq;

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

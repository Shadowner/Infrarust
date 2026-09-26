use uuid::Uuid;

use crate::types::Component;

pub const MAX_RESOURCE_PACK_URL: usize = 32_767;

pub const RESOURCE_PACK_HASH_LENGTH: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ResourcePackError {
    #[error("the resource pack URL is {len} characters, over the {MAX_RESOURCE_PACK_URL} allowed")]
    UrlTooLong { len: usize },

    #[error("`{0}` is not a SHA-1 hash of {RESOURCE_PACK_HASH_LENGTH} hexadecimal characters")]
    InvalidHash(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResourcePackRequest {
    pub id: Uuid,
    pub url: String,
    pub hash: Option<String>,
    pub required: bool,
    pub prompt: Option<Component>,
}

impl ResourcePackRequest {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            url: url.into(),
            hash: None,
            required: false,
            prompt: None,
        }
    }

    #[must_use]
    pub const fn id(mut self, id: Uuid) -> Self {
        self.id = id;
        self
    }

    #[must_use]
    pub fn hash(mut self, sha1_hex: impl Into<String>) -> Self {
        self.hash = Some(sha1_hex.into());
        self
    }

    #[must_use]
    pub const fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    #[must_use]
    pub fn prompt(mut self, prompt: Component) -> Self {
        self.prompt = Some(prompt);
        self
    }

    pub fn validate(&self) -> Result<(), ResourcePackError> {
        let len = self.url.chars().count();
        if len > MAX_RESOURCE_PACK_URL {
            return Err(ResourcePackError::UrlTooLong { len });
        }
        match &self.hash {
            Some(hash)
                if hash.len() != RESOURCE_PACK_HASH_LENGTH
                    || !hash.chars().all(|c| c.is_ascii_hexdigit()) =>
            {
                Err(ResourcePackError::InvalidHash(hash.clone()))
            }
            _ => Ok(()),
        }
    }
}

pub use infrarust_plugin_common::enums::ResourcePackStatus;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_round_trip_their_protocol_ids() {
        for id in 0..8 {
            assert_eq!(ResourcePackStatus::from_id(id).id(), id);
        }
        assert_eq!(
            ResourcePackStatus::from_id(99),
            ResourcePackStatus::Unknown(99)
        );
        assert!(ResourcePackStatus::Declined.is_final());
        assert!(!ResourcePackStatus::Accepted.is_final());
    }

    #[test]
    fn requests_are_validated() {
        let good = ResourcePackRequest::new("https://example.com/pack.zip")
            .hash("0123456789abcdef0123456789ABCDEF01234567");
        assert_eq!(good.validate(), Ok(()));
        assert_eq!(
            ResourcePackRequest::new("https://example.com")
                .hash("abc")
                .validate(),
            Err(ResourcePackError::InvalidHash("abc".to_string()))
        );
        assert_eq!(
            ResourcePackRequest::new("x".repeat(MAX_RESOURCE_PACK_URL + 1)).validate(),
            Err(ResourcePackError::UrlTooLong {
                len: MAX_RESOURCE_PACK_URL + 1
            })
        );
    }
}

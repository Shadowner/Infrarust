use uuid::Uuid;

use crate::types::Component;

pub const MAX_RESOURCE_PACK_URL: usize = 32_767;

pub const RESOURCE_PACK_HASH_LENGTH: usize = 40;

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

    pub fn validate(&self) -> Result<(), String> {
        if self.url.chars().count() > MAX_RESOURCE_PACK_URL {
            return Err(format!(
                "the resource pack URL is over {MAX_RESOURCE_PACK_URL} characters"
            ));
        }
        match &self.hash {
            Some(hash)
                if hash.len() != RESOURCE_PACK_HASH_LENGTH
                    || !hash.chars().all(|c| c.is_ascii_hexdigit()) =>
            {
                Err(format!(
                    "`{hash}` is not a SHA-1 hash of {RESOURCE_PACK_HASH_LENGTH} hexadecimal characters"
                ))
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
        assert!(
            ResourcePackRequest::new("https://example.com")
                .hash("abc")
                .validate()
                .is_err()
        );
        assert!(
            ResourcePackRequest::new("x".repeat(MAX_RESOURCE_PACK_URL + 1))
                .validate()
                .is_err()
        );
    }
}

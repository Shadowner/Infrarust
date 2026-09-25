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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ResourcePackStatus {
    SuccessfullyLoaded,
    Declined,
    FailedDownload,
    Accepted,
    Downloaded,
    InvalidUrl,
    FailedReload,
    Discarded,
    Unknown(i32),
}

impl ResourcePackStatus {
    pub const fn from_id(id: i32) -> Self {
        match id {
            0 => Self::SuccessfullyLoaded,
            1 => Self::Declined,
            2 => Self::FailedDownload,
            3 => Self::Accepted,
            4 => Self::Downloaded,
            5 => Self::InvalidUrl,
            6 => Self::FailedReload,
            7 => Self::Discarded,
            other => Self::Unknown(other),
        }
    }

    pub const fn id(self) -> i32 {
        match self {
            Self::SuccessfullyLoaded => 0,
            Self::Declined => 1,
            Self::FailedDownload => 2,
            Self::Accepted => 3,
            Self::Downloaded => 4,
            Self::InvalidUrl => 5,
            Self::FailedReload => 6,
            Self::Discarded => 7,
            Self::Unknown(id) => id,
        }
    }

    pub const fn is_final(self) -> bool {
        !matches!(self, Self::Accepted | Self::Downloaded | Self::Unknown(_))
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SuccessfullyLoaded => "successfully_loaded",
            Self::Declined => "declined",
            Self::FailedDownload => "failed_download",
            Self::Accepted => "accepted",
            Self::Downloaded => "downloaded",
            Self::InvalidUrl => "invalid_url",
            Self::FailedReload => "failed_reload",
            Self::Discarded => "discarded",
            Self::Unknown(_) => "unknown",
        }
    }
}

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

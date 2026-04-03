//! Mojang API client for premium username lookups.

use std::num::NonZeroU32;

use governor::clock::DefaultClock;
use governor::state::InMemoryState;
use governor::{Quota, RateLimiter};
use uuid::Uuid;

const MOJANG_API_URL: &str = "https://api.mojang.com/users/profiles/minecraft";

#[derive(Debug)]
pub enum LookupError {
    RateLimited,
    Network(reqwest::Error),
    UnexpectedStatus(u16),
    MalformedResponse(String),
}

impl std::fmt::Display for LookupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RateLimited => write!(f, "Mojang API rate limited"),
            Self::Network(e) => write!(f, "Mojang API network error: {e}"),
            Self::UnexpectedStatus(s) => write!(f, "Mojang API returned status {s}"),
            Self::MalformedResponse(msg) => write!(f, "Mojang API malformed response: {msg}"),
        }
    }
}

impl From<reqwest::Error> for LookupError {
    fn from(e: reqwest::Error) -> Self {
        Self::Network(e)
    }
}

/// Response from `GET /users/profiles/minecraft/<username>`.
#[derive(serde::Deserialize)]
struct MojangProfile {
    id: String,
}

pub struct MojangApiLookup {
    http_client: reqwest::Client,
    rate_limiter: RateLimiter<governor::state::NotKeyed, InMemoryState, DefaultClock>,
}

impl MojangApiLookup {
    pub fn new(requests_per_second: u32) -> Self {
        let max = NonZeroU32::new(requests_per_second.max(1)).expect("max(1) is always non-zero");
        let quota = Quota::per_second(max);
        let rate_limiter = RateLimiter::direct(quota);

        Self {
            http_client: reqwest::Client::new(),
            rate_limiter,
        }
    }

    pub async fn lookup_username(&self, username: &str) -> Result<Option<Uuid>, LookupError> {
        if self.rate_limiter.check().is_err() {
            return Err(LookupError::RateLimited);
        }

        let url = format!("{MOJANG_API_URL}/{username}");
        let response = self.http_client.get(&url).send().await?;

        match response.status().as_u16() {
            200 => {
                let profile: MojangProfile = response.json().await?;
                let uuid = Uuid::parse_str(&profile.id).map_err(|e| {
                    LookupError::MalformedResponse(format!("invalid UUID '{}': {e}", profile.id))
                })?;
                Ok(Some(uuid))
            }
            404 => Ok(None),
            429 => Err(LookupError::RateLimited),
            status => Err(LookupError::UnexpectedStatus(status)),
        }
    }
}

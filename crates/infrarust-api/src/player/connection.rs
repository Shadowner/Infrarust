use crate::types::{Component, namespaced_key};

pub const MAX_COOKIE_SIZE: usize = 5120;

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ConnectionResult {
    Success,
    AlreadyConnected,
    Denied(Component),
    Failed(Component),
    Cancelled,
}

impl ConnectionResult {
    pub const fn is_success(&self) -> bool {
        matches!(self, Self::Success | Self::AlreadyConnected)
    }

    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::AlreadyConnected => "already_connected",
            Self::Denied(_) => "denied",
            Self::Failed(_) => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "`{0}` is not a cookie key: `namespace:path` in lowercase letters, digits, `_`, `-` and `.` (and `/` in the path)"
)]
pub struct CookieKeyError(pub String);

pub fn cookie_key(key: &str) -> Result<String, CookieKeyError> {
    let (namespace, path) = key.split_once(':').unwrap_or(("minecraft", key));
    if namespaced_key::is_valid(namespace, path) {
        Ok(format!("{namespace}:{path}"))
    } else {
        Err(CookieKeyError(key.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_keys_are_normalized_like_the_client_does() {
        assert_eq!(cookie_key("token").as_deref(), Ok("minecraft:token"));
        assert_eq!(
            cookie_key("infrarust:auth/session").as_deref(),
            Ok("infrarust:auth/session")
        );
        assert!(cookie_key("Upper:case").is_err());
        assert!(cookie_key("a:b:c").is_err());
        assert!(cookie_key(":x").is_err());
        assert!(cookie_key("x:").is_err());
    }

    #[test]
    fn only_success_and_already_connected_count_as_success() {
        assert!(ConnectionResult::Success.is_success());
        assert!(ConnectionResult::AlreadyConnected.is_success());
        assert!(!ConnectionResult::Cancelled.is_success());
        assert!(!ConnectionResult::Failed(Component::text("x")).is_success());
    }
}

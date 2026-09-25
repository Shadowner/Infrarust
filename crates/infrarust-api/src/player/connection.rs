use crate::types::Component;

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

fn is_namespace_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '.')
}

pub fn cookie_key(key: &str) -> Result<String, String> {
    let (namespace, path) = key.split_once(':').unwrap_or(("minecraft", key));
    let valid = !namespace.is_empty()
        && !path.is_empty()
        && namespace.chars().all(is_namespace_char)
        && path.chars().all(|c| is_namespace_char(c) || c == '/');
    if valid {
        Ok(format!("{namespace}:{path}"))
    } else {
        Err(format!(
            "`{key}` is not a cookie key: `namespace:path` in lowercase letters, digits, `_`, `-` and `.` (and `/` in the path)"
        ))
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

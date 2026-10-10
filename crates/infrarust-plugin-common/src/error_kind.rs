use std::fmt;

/// The class of a failed host call, as the `error-kind` enum of the WIT
/// contract spells it.
///
/// Proxy errors classify themselves with a `kind()` method, and WASM guests
/// receive this value next to the error message. The set is closed: a WIT
/// enum cannot grow without a new contract version, so this enum is not
/// `#[non_exhaustive]` and every conversion to and from it is an exhaustive
/// match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// An argument was malformed or outside its allowed range.
    InvalidArgument,
    /// The named player, server, filter or other entity does not exist.
    NotFound,
    /// The plugin lacks the capability or permission the call needs.
    PermissionDenied,
    /// The service or carrier the call needs is not reachable right now.
    Unavailable,
    /// The call did not finish before its deadline.
    Timeout,
    /// The player the call targets is no longer connected.
    PlayerGone,
    /// The name or resource is already held by someone else.
    Conflict,
    /// The target is not in a state that accepts the call.
    InvalidState,
    /// The proxy does not implement the call for this target.
    Unsupported,
    /// The proxy failed for a reason the caller cannot act on.
    Internal,
    /// A plugin quota was reached.
    LimitExceeded,
}

impl ErrorKind {
    /// The kebab-case name used by the WIT contract.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidArgument => "invalid-argument",
            Self::NotFound => "not-found",
            Self::PermissionDenied => "permission-denied",
            Self::Unavailable => "unavailable",
            Self::Timeout => "timeout",
            Self::PlayerGone => "player-gone",
            Self::Conflict => "conflict",
            Self::InvalidState => "invalid-state",
            Self::Unsupported => "unsupported",
            Self::Internal => "internal",
            Self::LimitExceeded => "limit-exceeded",
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

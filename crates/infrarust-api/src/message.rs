//! Proxy-branded chat message helpers.

use crate::types::Component;

/// Helper for building proxy-branded chat messages.
pub struct ProxyMessage;

impl ProxyMessage {
    pub fn success(text: &str) -> Component {
        Self::prefixed(Component::text(text).color("green"))
    }

    pub fn error(text: &str) -> Component {
        Self::prefixed(Component::text(text).color("red"))
    }

    pub fn info(text: &str) -> Component {
        Self::prefixed(Component::text(text).color("gray"))
    }

    pub fn detail(text: &str) -> Component {
        Component::text(text).color("gray")
    }

    pub fn prefixed(message: Component) -> Component {
        Component::text("[Infrarust] ")
            .color("gold")
            .append(message)
    }
}

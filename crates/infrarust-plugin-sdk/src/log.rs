//! Logging to the host. Prefer the [`info!`](crate::info) family of macros for
//! formatted messages; these take an already-formatted `&str`.

pub fn trace(message: &str) {
    crate::bindings::log::trace(message);
}

pub fn debug(message: &str) {
    crate::bindings::log::debug(message);
}

pub fn info(message: &str) {
    crate::bindings::log::info(message);
}

pub fn warn(message: &str) {
    crate::bindings::log::warn(message);
}

pub fn error(message: &str) {
    crate::bindings::log::error(message);
}

//! Logging to the host. Prefer the [`info!`](crate::info) family of macros for
//! formatted messages; these take an already-formatted `&str`.

use std::cell::Cell;

use crate::bindings::log as wl;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

impl Level {
    const fn from_wit(level: wl::Level) -> Self {
        match level {
            wl::Level::Error => Self::Error,
            wl::Level::Warn => Self::Warn,
            wl::Level::Info => Self::Info,
            wl::Level::Debug => Self::Debug,
            wl::Level::Trace => Self::Trace,
        }
    }
}

const UNREAD: u8 = u8::MAX;
const OFF: u8 = 0;

thread_local! {
    static MAX_LEVEL: Cell<u8> = const { Cell::new(UNREAD) };
}

fn cached_max_level() -> u8 {
    MAX_LEVEL.with(|cached| {
        let level = cached.get();
        if level != UNREAD {
            return level;
        }
        let read = crate::host::max_log_level().map_or(OFF, |level| Level::from_wit(level) as u8);
        cached.set(read);
        read
    })
}

#[must_use]
pub fn max_level() -> Option<Level> {
    match cached_max_level() {
        1 => Some(Level::Error),
        2 => Some(Level::Warn),
        3 => Some(Level::Info),
        4 => Some(Level::Debug),
        5 => Some(Level::Trace),
        _ => None,
    }
}

#[must_use]
pub fn enabled(level: Level) -> bool {
    level as u8 <= cached_max_level()
}

#[doc(hidden)]
pub fn emit(level: Level, message: &str) {
    match level {
        Level::Error => wl::error(message),
        Level::Warn => wl::warn(message),
        Level::Info => wl::info(message),
        Level::Debug => wl::debug(message),
        Level::Trace => wl::trace(message),
    }
}

fn log(level: Level, message: &str) {
    if enabled(level) {
        emit(level, message);
    }
}

pub fn trace(message: &str) {
    log(Level::Trace, message);
}

pub fn debug(message: &str) {
    log(Level::Debug, message);
}

pub fn info(message: &str) {
    log(Level::Info, message);
}

pub fn warn(message: &str) {
    log(Level::Warn, message);
}

pub fn error(message: &str) {
    log(Level::Error, message);
}

#[cfg(test)]
pub(crate) fn forget_max_level() {
    MAX_LEVEL.with(|cached| cached.set(UNREAD));
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::host::with_fake;

    fn read_with(level: Option<wl::Level>) -> Option<Level> {
        forget_max_level();
        with_fake(|host| host.log_level = level);
        max_level()
    }

    #[test]
    fn the_host_level_is_read_once_and_then_cached() {
        assert_eq!(read_with(Some(wl::Level::Info)), Some(Level::Info));
        assert!(enabled(Level::Error));
        assert!(enabled(Level::Info));
        assert!(!enabled(Level::Debug));
        assert!(!enabled(Level::Trace));
        with_fake(|host| host.log_level = Some(wl::Level::Trace));
        assert!(!enabled(Level::Trace), "the first answer is kept");
        forget_max_level();
        assert!(enabled(Level::Trace));
    }

    #[test]
    fn nothing_is_enabled_when_the_proxy_logs_nothing() {
        assert_eq!(read_with(None), None);
        assert!(!enabled(Level::Error));
    }

    #[test]
    fn a_disabled_level_does_not_format_its_message() {
        struct Counted<'a>(&'a Cell<u32>);
        impl std::fmt::Display for Counted<'_> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.set(self.0.get() + 1);
                f.write_str("x")
            }
        }
        let formatted = Cell::new(0);
        read_with(None);
        crate::trace!("{}", Counted(&formatted));
        crate::error!("{}", Counted(&formatted));
        assert_eq!(formatted.get(), 0);
    }
}

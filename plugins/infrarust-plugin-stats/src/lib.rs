//! Pure, transport-agnostic logic for the stats plugin.
//!
//! Both the native and the WASM adapter call into these functions so their
//! observable behavior is identical by construction.

pub const COMMAND_NAME: &str = "count";
pub const COMMAND_ALIASES: &[&str] = &["online"];
pub const COMMAND_DESCRIPTION: &str = "Show the number of online players";

#[must_use]
pub fn join_log(username: &str) -> String {
    format!("{username} joined")
}

#[must_use]
pub fn leave_log(username: &str) -> String {
    format!("{username} left")
}

#[must_use]
pub fn format_count(online: u32) -> String {
    format!("Online: {online}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_count() {
        assert_eq!(format_count(7), "Online: 7");
        assert_eq!(format_count(0), "Online: 0");
    }

    #[test]
    fn formats_join_leave() {
        assert_eq!(join_log("Steve"), "Steve joined");
        assert_eq!(leave_log("Alex"), "Alex left");
    }
}

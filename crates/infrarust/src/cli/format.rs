//! CLI formatting utilities for colorful output

/// ANSI color codes for terminal output
pub struct Colors;

impl Colors {
    pub const RESET: &'static str = "\x1B[0m";
    pub const BOLD: &'static str = "\x1B[1m";

    // Regular colors
    pub const BLACK: &'static str = "\x1B[30m";
    pub const RED: &'static str = "\x1B[31m";
    pub const GREEN: &'static str = "\x1B[32m";
    pub const YELLOW: &'static str = "\x1B[33m";
    pub const BLUE: &'static str = "\x1B[34m";
    pub const MAGENTA: &'static str = "\x1B[35m";
    pub const CYAN: &'static str = "\x1B[36m";
    pub const WHITE: &'static str = "\x1B[37m";
    pub const GRAY: &'static str = "\x1B[90m";

    // Bold colors
    pub const BOLD_RED: &'static str = "\x1B[1;31m";
    pub const BOLD_GREEN: &'static str = "\x1B[1;32m";
    pub const BOLD_YELLOW: &'static str = "\x1B[1;33m";
    pub const BOLD_BLUE: &'static str = "\x1B[1;34m";
    pub const BOLD_MAGENTA: &'static str = "\x1B[1;35m";
    pub const BOLD_CYAN: &'static str = "\x1B[1;36m";
    pub const BOLD_WHITE: &'static str = "\x1B[1;37m";
}

/// Colorizes text with the given color
pub fn colorize(text: &str, color: &str) -> String {
    format!("{}{}{}", color, text, Colors::RESET)
}

/// Formats a header with bold green
pub fn header(text: &str) -> String {
    colorize(&format!("=== {} ===", text), Colors::BOLD_GREEN)
}

/// Formats a sub-header with bold cyan
pub fn sub_header(text: &str) -> String {
    colorize(text, Colors::BOLD_CYAN)
}

/// Formats an entity name (player, server) with cyan
pub fn entity(text: &str) -> String {
    colorize(text, Colors::CYAN)
}

/// Formats a warning with yellow
pub fn warning(text: &str) -> String {
    colorize(text, Colors::YELLOW)
}

/// Formats an error with red
pub fn error(text: &str) -> String {
    colorize(text, Colors::RED)
}

/// Formats secondary information with gray
pub fn secondary(text: &str) -> String {
    colorize(text, Colors::GRAY)
}

/// Formats a success message with green
pub fn success(text: &str) -> String {
    colorize(text, Colors::GREEN)
}

/// Formats a field label with bold
pub fn label(text: &str) -> String {
    colorize(text, Colors::BOLD)
}

/// Formats a UUID or session ID with dimmed color
pub fn id(text: &str) -> String {
    colorize(text, Colors::GRAY)
}

/// Formats an info message with blue
pub fn info(text: &str) -> String {
    colorize(text, Colors::BLUE)
}

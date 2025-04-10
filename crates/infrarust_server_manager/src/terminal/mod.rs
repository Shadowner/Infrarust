mod command;
mod tests;
#[cfg(target_os = "linux")]
mod unix;
#[cfg(target_os = "windows")]
mod windows;

// Public API
pub use command::execute_command;

mod command;
#[cfg(target_os = "linux")]
mod unix;
#[cfg(target_os = "windows")]
mod windows;
mod tests;

pub use command::execute_command;

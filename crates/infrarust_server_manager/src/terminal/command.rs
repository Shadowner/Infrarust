use crate::error::ServerManagerError;

#[cfg(target_os = "windows")]
use super::windows;
#[cfg(target_os = "linux")]
use super::unix;

#[cfg(target_os = "windows")]
fn execute_platform_command(command: &str) -> std::io::Result<String> {
    windows::execute_command(command)
}

#[cfg(target_os = "linux")]
fn execute_platform_command(command: &str) -> std::io::Result<String> {
    unix::UnixTerminal::execute_command(command)
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn execute_platform_command(_command: &str) -> std::io::Result<String> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Other,
        "Unsupported operating system",
    ))
}

pub fn execute_command(command: &str) -> Result<String, ServerManagerError> {
    match execute_platform_command(command) {
        Ok(output) => Ok(output),
        Err(e) => Err(ServerManagerError::CommandError(e.to_string())),
    }
}
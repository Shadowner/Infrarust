use crate::error::ServerManagerError;

#[cfg(target_os = "linux")]
use super::unix;
#[cfg(target_os = "windows")]
use super::windows;

pub fn execute_command(command: &str) -> Result<String, ServerManagerError> {
    let result = {
        #[cfg(target_os = "windows")]
        windows::execute_command(command);
        #[cfg(target_os = "linux")]
        unix::UnixTerminal::execute_command(command)
    };

    match result {
        Ok(output) => Ok(output),
        Err(e) => Err(ServerManagerError::CommandError(e.to_string())),
    }
}

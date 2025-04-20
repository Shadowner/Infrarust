use std::io::Error as IoError;
use std::process::Command;

pub struct UnixTerminal {}

impl UnixTerminal {
    pub fn execute_command(command: &str) -> Result<String, IoError> {
        let output = Command::new("bash").arg("-c").arg(command).output()?;

        if output.status.success() {
            let result = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Ok(result)
        } else {
            let error = String::from_utf8_lossy(&output.stderr).to_string();
            Err(IoError::new(std::io::ErrorKind::Other, error))
        }
    }
}

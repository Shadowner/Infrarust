use std::error::Error;
use std::fmt;

#[derive(Debug)]
pub enum ServerManagerError {
    ApiError(String),
    CommandError(String),
    MonitoringError(String),
    IoError(std::io::Error),
}

impl fmt::Display for ServerManagerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServerManagerError::ApiError(msg) => write!(f, "API Error: {}", msg),
            ServerManagerError::CommandError(msg) => write!(f, "Command Error: {}", msg),
            ServerManagerError::MonitoringError(msg) => write!(f, "Monitoring Error: {}", msg),
            ServerManagerError::IoError(e) => write!(f, "I/O Error: {}", e),
        }
    }
}

impl Error for ServerManagerError {}

impl From<std::io::Error> for ServerManagerError {
    fn from(error: std::io::Error) -> Self {
        ServerManagerError::IoError(error)
    }
}

impl From<reqwest::Error> for ServerManagerError {
    fn from(error: reqwest::Error) -> Self {
        ServerManagerError::ApiError(error.to_string())
    }
}

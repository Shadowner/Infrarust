#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("storage error: {0}")]
    Storage(#[from] AuthStorageError),

    #[error("hashing error: {0}")]
    Hashing(String),

    #[error("config error: {0}")]
    Config(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("password validation failed: {reason}")]
    PasswordValidation { reason: String },
}

#[derive(Debug, thiserror::Error)]
pub enum AuthStorageError {
    #[error("account already exists: {username}")]
    AccountAlreadyExists { username: String },

    #[error("account not found: {username}")]
    AccountNotFound { username: String },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serialization(String),
}

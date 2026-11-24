#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("no trusted release key is built into this binary")]
    NoTrustedKey,
}

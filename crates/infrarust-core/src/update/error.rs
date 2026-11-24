use std::time::SystemTime;

use super::archive::ArchiveError;
use super::keys::SignatureError;
use super::manifest::ManifestError;

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("no trusted release key is built into this binary")]
    NoTrustedKey,
    #[error("the update url {url} cannot be used: {reason}")]
    InvalidSource { url: String, reason: String },
    #[error("request to {url} failed: {source}")]
    Network {
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("{url} answered HTTP {status}")]
    Status { url: String, status: u16 },
    #[error("{}", rate_limited(*.reset))]
    RateLimited { reset: Option<SystemTime> },
    #[error("{what} is larger than {limit} bytes")]
    TooLarge { what: String, limit: u64 },
    #[error("{what} cannot be read: {reason}")]
    Malformed { what: String, reason: String },
    #[error("release {tag} has no signed manifest")]
    NoManifest { tag: String },
    #[error("release {tag}: {source}")]
    Signature {
        tag: String,
        #[source]
        source: SignatureError,
    },
    #[error("release {tag}: {source}")]
    Manifest {
        tag: String,
        #[source]
        source: ManifestError,
    },
    #[error("no release {version} among the recent releases")]
    VersionNotFound { version: String },
    #[error("{file} has {actual} bytes, the manifest announces {expected}")]
    SizeMismatch {
        file: String,
        expected: u64,
        actual: u64,
    },
    #[error("{file} does not match the sha256 of the manifest")]
    ShaMismatch { file: String },
    #[error(transparent)]
    Archive(#[from] ArchiveError),
    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },
}

fn rate_limited(reset: Option<SystemTime>) -> String {
    match reset {
        Some(reset) => format!(
            "the update server rate limit is reached until {}",
            humantime::format_rfc3339_seconds(reset)
        ),
        None => "the update server rate limit is reached".to_string(),
    }
}

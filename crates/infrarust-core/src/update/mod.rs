pub mod channel;
mod error;
pub mod keys;
pub mod manifest;
#[cfg(test)]
mod testing;

pub use error::UpdateError;
pub use keys::{SignatureError, TrustedKeys, VerifyError};
pub use manifest::{Artifact, Manifest, ManifestError, VerifiedManifest};

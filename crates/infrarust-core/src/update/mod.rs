pub mod archive;
pub mod channel;
pub mod check;
pub mod download;
mod error;
pub mod keys;
pub mod manifest;
pub mod source;
#[cfg(test)]
mod testing;

pub use check::Candidate;
pub use error::UpdateError;
pub use keys::{SignatureError, TrustedKeys, VerifyError};
pub use manifest::{Artifact, Manifest, ManifestError, VerifiedManifest};
pub use source::{Asset, Release, ReleaseSource};

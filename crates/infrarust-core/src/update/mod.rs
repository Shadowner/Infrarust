pub mod archive;
pub mod channel;
pub mod check;
pub mod download;
mod error;
pub mod gate;
pub mod install;
pub mod keys;
pub mod manifest;
pub mod platform;
pub mod source;
#[cfg(test)]
mod testing;

pub use check::Candidate;
pub use error::UpdateError;
pub use gate::Refusal;
pub use install::{
    InstallObserver, InstallStep, InstallTarget, Installed, Installer, LogObserver, NoopObserver,
    RolledBack,
};
pub use keys::{SignatureError, TrustedKeys, VerifyError};
pub use manifest::{Artifact, Manifest, ManifestError, VerifiedManifest};
pub use platform::{BinaryPaths, Platform, SwapPlan, SwapStep};
pub use source::{Asset, Release, ReleaseSource};

use minisign_verify::{Error as MinisignError, PublicKey, Signature};

use super::error::UpdateError;
use super::manifest::{ManifestError, VerifiedManifest};

pub const RELEASE_KEYS: &[&str] = &[include_str!("keys/release-1.pub")];

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SignatureError {
    #[error("the signature cannot be decoded")]
    Malformed,
    #[error("the signature uses the legacy minisign format, which is refused")]
    Legacy,
    #[error("the signature does not match the manifest")]
    Invalid,
    #[error("the signature was made by a key this binary does not trust")]
    UnknownKey,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VerifyError {
    #[error(transparent)]
    Signature(#[from] SignatureError),
    #[error(transparent)]
    Manifest(#[from] ManifestError),
}

#[derive(Debug, Clone)]
pub struct TrustedKeys {
    keys: Vec<PublicKey>,
}

impl TrustedKeys {
    pub fn release() -> Result<Self, UpdateError> {
        Self::from_strs(RELEASE_KEYS)
    }

    pub fn from_strs<I, S>(keys: I) -> Result<Self, UpdateError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut parsed = Vec::new();
        for (index, text) in keys.into_iter().enumerate() {
            match PublicKey::decode(text.as_ref().trim()) {
                Ok(key) => parsed.push(key),
                Err(e) => tracing::error!(
                    key = index + 1,
                    error = %e,
                    "a trusted release key cannot be read and is ignored"
                ),
            }
        }
        if parsed.is_empty() {
            return Err(UpdateError::NoTrustedKey);
        }
        Ok(Self { keys: parsed })
    }

    pub const fn len(&self) -> usize {
        self.keys.len()
    }

    pub const fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn verify(
        &self,
        manifest: &[u8],
        signature: &str,
        release_tag: &str,
    ) -> Result<VerifiedManifest, VerifyError> {
        let signature = Signature::decode(signature).map_err(|_| SignatureError::Malformed)?;
        let mut failure = SignatureError::UnknownKey;
        for key in &self.keys {
            match key.verify(manifest, &signature, false) {
                Ok(()) => return Ok(VerifiedManifest::from_signed(manifest, release_tag)?),
                Err(MinisignError::UnexpectedKeyId) => {}
                Err(MinisignError::UnexpectedAlgorithm) => failure = SignatureError::Legacy,
                Err(_) => {
                    if failure != SignatureError::Legacy {
                        failure = SignatureError::Invalid;
                    }
                }
            }
        }
        Err(failure.into())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::super::testing::{SigningKey, manifest_json};
    use super::*;

    const TAG: &str = "v2.0.0-beta.6";

    fn manifest() -> String {
        manifest_json(
            "2.0.0-beta.6",
            &[("infrarust-linux-x86_64.tar.gz", "infrarust", b"bin")],
        )
    }

    #[test]
    fn a_valid_signature_yields_a_verified_manifest() {
        let key = SigningKey::generate();
        let keys = TrustedKeys::from_strs([key.public()]).unwrap();
        let manifest = manifest();
        let verified = keys
            .verify(manifest.as_bytes(), &key.sign(manifest.as_bytes()), TAG)
            .unwrap();
        assert_eq!(verified.tag(), TAG);
    }

    #[test]
    fn one_tampered_byte_breaks_the_signature() {
        let key = SigningKey::generate();
        let keys = TrustedKeys::from_strs([key.public()]).unwrap();
        let manifest = manifest();
        let signature = key.sign(manifest.as_bytes());
        let mut tampered = manifest.into_bytes();
        let at = tampered.len() / 2;
        tampered[at] ^= 0x01;
        assert_eq!(
            keys.verify(&tampered, &signature, TAG),
            Err(VerifyError::Signature(SignatureError::Invalid))
        );
    }

    #[test]
    fn a_tampered_trusted_comment_breaks_the_signature() {
        let key = SigningKey::generate();
        let keys = TrustedKeys::from_strs([key.public()]).unwrap();
        let manifest = manifest();
        let signature = key.sign(manifest.as_bytes()).replacen(
            "\ntrusted comment: ",
            "\ntrusted comment: x",
            1,
        );
        assert_eq!(
            keys.verify(manifest.as_bytes(), &signature, TAG),
            Err(VerifyError::Signature(SignatureError::Invalid))
        );
    }

    #[test]
    fn a_signature_from_another_key_is_refused() {
        let trusted = SigningKey::generate();
        let attacker = SigningKey::generate();
        let keys = TrustedKeys::from_strs([trusted.public()]).unwrap();
        let manifest = manifest();
        assert_eq!(
            keys.verify(
                manifest.as_bytes(),
                &attacker.sign(manifest.as_bytes()),
                TAG
            ),
            Err(VerifyError::Signature(SignatureError::UnknownKey))
        );
    }

    #[test]
    fn the_second_key_of_the_array_is_tried() {
        let old = SigningKey::generate();
        let new = SigningKey::generate();
        let keys = TrustedKeys::from_strs([old.public(), new.public()]).unwrap();
        assert_eq!(keys.len(), 2);
        let manifest = manifest();
        assert!(
            keys.verify(manifest.as_bytes(), &new.sign(manifest.as_bytes()), TAG)
                .is_ok()
        );
    }

    #[test]
    fn a_valid_legacy_signature_is_refused() {
        let public = "untrusted comment: minisign public key E7620F1842B4E81F\nRWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
        let legacy = "untrusted comment: signature from minisign secret key
RWQf6LRCGA9i59SLOFxz6NxvASXDJeRtuZykwQepbDEGt87ig1BNpWaVWuNrm73YiIiJbq71Wi+dP9eKL8OC351vwIasSSbXxwA=
trusted comment: timestamp:1555779966\tfile:test
QtKMXWyYcwdpZAlPF7tE2ENJkRd1ujvKjlj1m9RtHTBnZPa5WKU5uWRs5GoP5M/VqE81QFuMKI5k/SfNQUaOAA==";
        let key = PublicKey::decode(public).unwrap();
        let signature = Signature::decode(legacy).unwrap();
        assert!(key.verify(b"test", &signature, true).is_ok());

        let keys = TrustedKeys::from_strs([public]).unwrap();
        assert_eq!(
            keys.verify(b"test", legacy, TAG),
            Err(VerifyError::Signature(SignatureError::Legacy))
        );
    }

    #[test]
    fn a_garbage_signature_is_malformed() {
        let key = SigningKey::generate();
        let keys = TrustedKeys::from_strs([key.public()]).unwrap();
        for garbage in ["", "hello", "untrusted comment: x\nnot base64\n", "\0\0\0"] {
            assert_eq!(
                keys.verify(b"{}", garbage, TAG),
                Err(VerifyError::Signature(SignatureError::Malformed)),
                "{garbage:?}"
            );
        }
    }

    #[test]
    fn a_signed_but_invalid_manifest_is_refused() {
        let key = SigningKey::generate();
        let keys = TrustedKeys::from_strs([key.public()]).unwrap();
        let manifest = manifest();
        assert!(matches!(
            keys.verify(
                manifest.as_bytes(),
                &key.sign(manifest.as_bytes()),
                "v9.9.9"
            ),
            Err(VerifyError::Manifest(ManifestError::ReleaseTag { .. }))
        ));
    }

    #[test]
    fn unreadable_keys_are_skipped_and_none_left_is_an_error() {
        let key = SigningKey::generate();
        let keys = TrustedKeys::from_strs(["garbage".to_string(), key.public()]).unwrap();
        assert_eq!(keys.len(), 1);
        assert!(matches!(
            TrustedKeys::from_strs(["garbage", ""]),
            Err(UpdateError::NoTrustedKey)
        ));
        assert!(matches!(
            TrustedKeys::from_strs(Vec::<String>::new()),
            Err(UpdateError::NoTrustedKey)
        ));
    }
}

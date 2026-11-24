use std::collections::HashSet;

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::binary::HandoverSupport;

pub const MANIFEST_SCHEMA: u32 = 1;
pub const MAX_ARTIFACT_SIZE: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    pub version: String,
    pub tag: String,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub notes_url: Option<String>,
    #[serde(default)]
    pub security: bool,
    #[serde(default)]
    pub handover: Option<HandoverSupport>,
    pub artifacts: Vec<Artifact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artifact {
    pub targets: Vec<String>,
    pub file: String,
    pub binary: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManifestError {
    #[error("the manifest is not valid JSON: {0}")]
    Json(String),
    #[error("manifest schema {0} is not supported")]
    Schema(u32),
    #[error("manifest version {0} is not valid semver")]
    Version(String),
    #[error("manifest tag {tag} does not match its version {version}")]
    TagVersion { tag: String, version: String },
    #[error("manifest tag {manifest} does not match the release tag {release}")]
    ReleaseTag { manifest: String, release: String },
    #[error("artifact {file}: the sha256 must be 64 hexadecimal characters")]
    Sha256 { file: String },
    #[error("artifact {file}: size {size} is outside 1..={MAX_ARTIFACT_SIZE}")]
    Size { file: String, size: u64 },
    #[error("artifact name {0:?} is not a plain file name")]
    Name(String),
    #[error("target {0} is listed by more than one artifact")]
    DuplicateTarget(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedManifest {
    manifest: Manifest,
    version: Version,
}

impl VerifiedManifest {
    pub(super) fn from_signed(bytes: &[u8], release_tag: &str) -> Result<Self, ManifestError> {
        let manifest: Manifest =
            serde_json::from_slice(bytes).map_err(|e| ManifestError::Json(e.to_string()))?;
        let version = validate(&manifest, release_tag)?;
        Ok(Self { manifest, version })
    }

    pub const fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub const fn version(&self) -> &Version {
        &self.version
    }

    pub fn tag(&self) -> &str {
        &self.manifest.tag
    }

    pub const fn security(&self) -> bool {
        self.manifest.security
    }

    pub fn artifact_for(&self, target: &str) -> Option<&Artifact> {
        self.manifest
            .artifacts
            .iter()
            .find(|artifact| artifact.targets.iter().any(|t| t == target))
    }
}

fn validate(manifest: &Manifest, release_tag: &str) -> Result<Version, ManifestError> {
    if manifest.schema != MANIFEST_SCHEMA {
        return Err(ManifestError::Schema(manifest.schema));
    }
    let version = Version::parse(&manifest.version)
        .map_err(|_| ManifestError::Version(manifest.version.clone()))?;
    if manifest.tag != format!("v{}", manifest.version) {
        return Err(ManifestError::TagVersion {
            tag: manifest.tag.clone(),
            version: manifest.version.clone(),
        });
    }
    if manifest.tag != release_tag {
        return Err(ManifestError::ReleaseTag {
            manifest: manifest.tag.clone(),
            release: release_tag.to_string(),
        });
    }
    let mut targets = HashSet::new();
    for artifact in &manifest.artifacts {
        validate_artifact(artifact)?;
        for target in &artifact.targets {
            if !targets.insert(target.as_str()) {
                return Err(ManifestError::DuplicateTarget(target.clone()));
            }
        }
    }
    Ok(version)
}

fn validate_artifact(artifact: &Artifact) -> Result<(), ManifestError> {
    for name in [&artifact.file, &artifact.binary] {
        if !is_plain_name(name) {
            return Err(ManifestError::Name(name.clone()));
        }
    }
    if artifact.sha256.len() != 64 || !artifact.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ManifestError::Sha256 {
            file: artifact.file.clone(),
        });
    }
    if artifact.size == 0 || artifact.size > MAX_ARTIFACT_SIZE {
        return Err(ManifestError::Size {
            file: artifact.file.clone(),
            size: artifact.size,
        });
    }
    Ok(())
}

fn is_plain_name(name: &str) -> bool {
    !name.is_empty() && name != "." && !name.contains(['/', '\\', '\0']) && !name.contains("..")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::{Value, json};

    use super::*;

    const SHA: &str = "aa3f2a6ad9ad0a6c3c8e52e1f2a6a0b0c2a8f2e0a1b2c3d4e5f60718293a4b5c";

    fn manifest() -> Value {
        json!({
            "schema": 1,
            "version": "2.0.0-beta.6",
            "tag": "v2.0.0-beta.6",
            "published_at": "2026-11-24T10:00:00Z",
            "notes_url": "https://github.com/Shadowner/Infrarust/releases/tag/v2.0.0-beta.6",
            "security": false,
            "handover": {"protocol": 1, "accepts": [1]},
            "artifacts": [
                {
                    "targets": ["x86_64-unknown-linux-musl", "x86_64-unknown-linux-gnu"],
                    "file": "infrarust-linux-x86_64.tar.gz",
                    "binary": "infrarust",
                    "sha256": SHA,
                    "size": 123
                },
                {
                    "targets": ["x86_64-pc-windows-msvc"],
                    "file": "infrarust-windows-x86_64.zip",
                    "binary": "infrarust.exe",
                    "sha256": SHA,
                    "size": 456
                }
            ]
        })
    }

    fn parse(value: &Value) -> Result<VerifiedManifest, ManifestError> {
        VerifiedManifest::from_signed(value.to_string().as_bytes(), "v2.0.0-beta.6")
    }

    fn with(edit: impl FnOnce(&mut Value)) -> Result<VerifiedManifest, ManifestError> {
        let mut value = manifest();
        edit(&mut value);
        parse(&value)
    }

    #[test]
    fn a_well_formed_manifest_is_accepted() {
        let verified = parse(&manifest()).unwrap();
        assert_eq!(verified.version(), &Version::parse("2.0.0-beta.6").unwrap());
        assert_eq!(verified.tag(), "v2.0.0-beta.6");
        assert!(!verified.security());
        assert_eq!(
            verified.manifest().handover,
            Some(HandoverSupport {
                protocol: 1,
                accepts: vec![1]
            })
        );
    }

    #[test]
    fn unknown_fields_are_ignored_and_handover_is_optional() {
        let verified = with(|m| {
            m["future_field"] = json!({"anything": true});
            m["artifacts"][0]["signature_url"] = json!("x");
            m.as_object_mut().unwrap().remove("handover");
        })
        .unwrap();
        assert!(verified.manifest().handover.is_none());
        let verified = with(|m| m["handover"] = Value::Null).unwrap();
        assert!(verified.manifest().handover.is_none());
    }

    #[test]
    fn the_schema_must_be_one() {
        assert_eq!(
            with(|m| m["schema"] = json!(2)),
            Err(ManifestError::Schema(2))
        );
    }

    #[test]
    fn the_version_must_be_semver() {
        assert!(matches!(
            with(|m| {
                m["version"] = json!("next");
                m["tag"] = json!("vnext");
            }),
            Err(ManifestError::Version(_))
        ));
    }

    #[test]
    fn the_tag_must_be_v_and_the_version() {
        assert!(matches!(
            with(|m| m["tag"] = json!("2.0.0-beta.6")),
            Err(ManifestError::TagVersion { .. })
        ));
        assert!(matches!(
            with(|m| m["tag"] = json!("v2.0.0-beta.7")),
            Err(ManifestError::TagVersion { .. })
        ));
    }

    #[test]
    fn the_tag_must_match_the_release() {
        let bytes = manifest().to_string();
        assert!(matches!(
            VerifiedManifest::from_signed(bytes.as_bytes(), "v2.0.0-beta.7"),
            Err(ManifestError::ReleaseTag { .. })
        ));
    }

    #[test]
    fn the_sha256_must_be_64_hex_characters() {
        for bad in [
            &SHA[1..],
            "zz3f2a6ad9ad0a6c3c8e52e1f2a6a0b0c2a8f2e0a1b2c3d4e5f60718293a4b5c",
            "",
        ] {
            assert!(
                matches!(
                    with(|m| m["artifacts"][0]["sha256"] = json!(bad)),
                    Err(ManifestError::Sha256 { .. })
                ),
                "{bad}"
            );
        }
        assert!(with(|m| m["artifacts"][0]["sha256"] = json!(SHA.to_uppercase())).is_ok());
    }

    #[test]
    fn the_size_must_be_positive_and_at_most_256_mib() {
        assert!(matches!(
            with(|m| m["artifacts"][0]["size"] = json!(0)),
            Err(ManifestError::Size { size: 0, .. })
        ));
        assert!(matches!(
            with(|m| m["artifacts"][0]["size"] = json!(MAX_ARTIFACT_SIZE + 1)),
            Err(ManifestError::Size { .. })
        ));
        assert!(with(|m| m["artifacts"][0]["size"] = json!(MAX_ARTIFACT_SIZE)).is_ok());
    }

    #[test]
    fn names_must_be_plain_file_names() {
        for field in ["file", "binary"] {
            for bad in [
                "../infrarust",
                "bin/infrarust",
                "bin\\infrarust",
                "a..b",
                "nul\0",
                "",
                ".",
            ] {
                assert!(
                    matches!(
                        with(|m| m["artifacts"][0][field] = json!(bad)),
                        Err(ManifestError::Name(_))
                    ),
                    "{field} = {bad:?}"
                );
            }
        }
    }

    #[test]
    fn a_target_listed_twice_is_rejected() {
        assert!(matches!(
            with(|m| m["artifacts"][1]["targets"] = json!(["x86_64-unknown-linux-gnu"])),
            Err(ManifestError::DuplicateTarget(_))
        ));
    }

    #[test]
    fn garbage_is_not_a_manifest() {
        assert!(matches!(
            VerifiedManifest::from_signed(b"not json", "v2.0.0-beta.6"),
            Err(ManifestError::Json(_))
        ));
        assert!(matches!(
            with(|m| {
                m.as_object_mut().unwrap().remove("artifacts");
            }),
            Err(ManifestError::Json(_))
        ));
    }

    #[test]
    fn artifacts_match_the_exact_target_triple() {
        let verified = parse(&manifest()).unwrap();
        assert_eq!(
            verified
                .artifact_for("x86_64-unknown-linux-gnu")
                .map(|a| a.file.as_str()),
            Some("infrarust-linux-x86_64.tar.gz")
        );
        assert_eq!(
            verified
                .artifact_for("x86_64-pc-windows-msvc")
                .map(|a| a.binary.as_str()),
            Some("infrarust.exe")
        );
        assert!(verified.artifact_for("x86_64-unknown-linux").is_none());
        assert!(
            verified
                .artifact_for("aarch64-unknown-linux-musl")
                .is_none()
        );
        assert!(verified.artifact_for("X86_64-unknown-linux-musl").is_none());
    }
}

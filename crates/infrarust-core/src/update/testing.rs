#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Cursor;

use sha2::{Digest, Sha256};

pub(crate) const TEST_TARGET: &str = "x86_64-unknown-linux-musl";

pub(crate) struct SigningKey {
    public: minisign::PublicKey,
    secret: minisign::SecretKey,
}

impl SigningKey {
    pub(crate) fn generate() -> Self {
        let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        Self {
            public: pair.pk,
            secret: pair.sk,
        }
    }

    pub(crate) fn public(&self) -> String {
        self.public.to_box().unwrap().into_string()
    }

    pub(crate) fn sign(&self, data: &[u8]) -> String {
        minisign::sign(
            Some(&self.public),
            &self.secret,
            Cursor::new(data),
            Some("file:manifest.json"),
            None,
        )
        .unwrap()
        .into_string()
    }
}

pub(crate) fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) fn manifest_json(version: &str, artifacts: &[(&str, &str, &[u8])]) -> String {
    let artifacts: Vec<serde_json::Value> = artifacts
        .iter()
        .map(|(file, binary, archive)| {
            serde_json::json!({
                "targets": [TEST_TARGET],
                "file": file,
                "binary": binary,
                "sha256": sha256_hex(archive),
                "size": archive.len(),
            })
        })
        .collect();
    serde_json::json!({
        "schema": 1,
        "version": version,
        "tag": format!("v{version}"),
        "published_at": "2026-11-24T10:00:00Z",
        "notes_url": format!("https://example.invalid/notes/{version}"),
        "security": false,
        "handover": null,
        "artifacts": artifacts,
    })
    .to_string()
}

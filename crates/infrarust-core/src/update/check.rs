use semver::Version;

use super::channel::{ChannelFilter, newer_first, parse_version};
use super::error::UpdateError;
use super::keys::{SignatureError, TrustedKeys, VerifyError};
use super::manifest::VerifiedManifest;
use super::source::{
    MANIFEST_ASSET, MANIFEST_LIMIT, Release, ReleaseSource, SIGNATURE_ASSET, SIGNATURE_LIMIT,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub release: Release,
    pub manifest: VerifiedManifest,
}

impl Candidate {
    pub const fn version(&self) -> &Version {
        self.manifest.version()
    }
}

pub async fn newest(
    source: &ReleaseSource,
    keys: &TrustedKeys,
    filter: ChannelFilter,
    current: &Version,
) -> Result<Option<Candidate>, UpdateError> {
    let releases = source.releases().await?;
    let items = releases
        .iter()
        .filter(|release| !release.draft)
        .filter_map(|release| parse_version(&release.tag_name).map(|version| (version, release)));
    for (_, release) in newer_first(items, filter, current) {
        match load(source, keys, release).await {
            Ok(candidate) => return Ok(Some(candidate)),
            Err(UpdateError::NoManifest { tag }) => {
                tracing::debug!(%tag, "release has no signed manifest; skipped");
            }
            Err(
                e @ (UpdateError::RateLimited { .. }
                | UpdateError::Network { .. }
                | UpdateError::Status {
                    status: 500..=599, ..
                }),
            ) => {
                return Err(e);
            }
            Err(e) => {
                tracing::warn!(tag = %release.tag_name, error = %e, "release skipped");
            }
        }
    }
    Ok(None)
}

pub async fn pinned(
    source: &ReleaseSource,
    keys: &TrustedKeys,
    version: &Version,
) -> Result<Candidate, UpdateError> {
    let releases = source.releases().await?;
    let release = releases
        .iter()
        .filter(|release| !release.draft)
        .find(|release| parse_version(&release.tag_name).as_ref() == Some(version))
        .ok_or_else(|| UpdateError::VersionNotFound {
            version: version.to_string(),
        })?;
    load(source, keys, release).await
}

async fn load(
    source: &ReleaseSource,
    keys: &TrustedKeys,
    release: &Release,
) -> Result<Candidate, UpdateError> {
    let tag = release.tag_name.clone();
    let (Some(manifest), Some(signature)) = (
        release.asset(MANIFEST_ASSET),
        release.asset(SIGNATURE_ASSET),
    ) else {
        return Err(UpdateError::NoManifest { tag });
    };
    let bytes = source
        .fetch(
            &manifest.browser_download_url,
            "the manifest",
            MANIFEST_LIMIT,
        )
        .await?;
    let signature = source
        .fetch(
            &signature.browser_download_url,
            "the manifest signature",
            SIGNATURE_LIMIT,
        )
        .await?;
    let Ok(signature) = String::from_utf8(signature) else {
        return Err(UpdateError::Signature {
            tag,
            source: SignatureError::Malformed,
        });
    };
    let manifest = keys
        .verify(&bytes, &signature, &release.tag_name)
        .map_err(|e| match e {
            VerifyError::Signature(source) => UpdateError::Signature {
                tag: tag.clone(),
                source,
            },
            VerifyError::Manifest(source) => UpdateError::Manifest {
                tag: tag.clone(),
                source,
            },
        })?;
    Ok(Candidate {
        release: release.clone(),
        manifest,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use infrarust_config::UpdateChannel;

    use super::super::keys::SignatureError;
    use super::super::testing::{ARCHIVE, HttpServer, Reply, SigningKey, manifest_json};
    use super::*;

    fn v(text: &str) -> Version {
        parse_version(text).unwrap()
    }

    struct Fixture {
        server: HttpServer,
        source: ReleaseSource,
        keys: TrustedKeys,
    }

    async fn fixture(
        build: impl FnOnce(&HttpServer, &SigningKey) -> Vec<serde_json::Value>,
    ) -> Fixture {
        let key = SigningKey::generate();
        let server = HttpServer::routed().await;
        let releases = build(&server, &key);
        server.list(&releases);
        let source = ReleaseSource::new(&server.url("/releases"), "2.0.0-beta.5").unwrap();
        let keys = TrustedKeys::from_strs([key.public()]).unwrap();
        Fixture {
            server,
            source,
            keys,
        }
    }

    fn forged(server: &HttpServer, version: &str) -> serde_json::Value {
        let attacker = SigningKey::generate();
        server.publish_signed(&attacker, version, b"evil")
    }

    #[tokio::test]
    async fn the_newest_valid_release_wins_and_broken_ones_are_skipped() {
        let f = fixture(|server, key| {
            vec![
                server.publish_signed(key, "2.0.0-beta.7", b"seven"),
                forged(server, "2.0.0-beta.10"),
                server.publish("v2.0.0-beta.9", &[(ARCHIVE, b"no manifest")]),
                server.publish_signed(key, "2.0.0-beta.8", b"eight"),
                server.publish("nightly", &[]),
            ]
        })
        .await;
        let filter = ChannelFilter::new(UpdateChannel::Auto, &v("2.0.0-beta.5"));
        let candidate = newest(&f.source, &f.keys, filter, &v("2.0.0-beta.5"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(candidate.version(), &v("2.0.0-beta.8"));
        assert_eq!(candidate.release.tag_name, "v2.0.0-beta.8");
        assert!(
            candidate
                .manifest
                .artifact_for("x86_64-unknown-linux-musl")
                .is_some()
        );

        let fetched: Vec<String> = f.server.requests().into_iter().map(|r| r.path).collect();
        assert!(!fetched.iter().any(|path| path.contains("beta.7")));
    }

    #[tokio::test]
    async fn a_release_whose_manifest_lies_about_its_tag_is_skipped() {
        let f = fixture(|server, key| {
            let manifest = manifest_json("2.0.0-beta.9", &[(ARCHIVE, "infrarust", b"x")]);
            let signature = key.sign(manifest.as_bytes());
            vec![
                server.publish(
                    "v2.0.0-beta.10",
                    &[
                        ("manifest.json", manifest.as_bytes()),
                        ("manifest.json.minisig", signature.as_bytes()),
                    ],
                ),
                server.publish_signed(key, "2.0.0-beta.6", b"six"),
            ]
        })
        .await;
        let filter = ChannelFilter::new(UpdateChannel::Auto, &v("2.0.0-beta.5"));
        let candidate = newest(&f.source, &f.keys, filter, &v("2.0.0-beta.5"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(candidate.version(), &v("2.0.0-beta.6"));
    }

    #[tokio::test]
    async fn nothing_newer_means_no_candidate() {
        let f = fixture(|server, key| {
            vec![
                server.publish_signed(key, "2.0.0-beta.5", b"same"),
                server.publish_signed(key, "2.0.0-beta.4", b"older"),
            ]
        })
        .await;
        let filter = ChannelFilter::new(UpdateChannel::Prerelease, &v("2.0.0-beta.5"));
        assert!(
            newest(&f.source, &f.keys, filter, &v("2.0.0-beta.5"))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn a_rate_limited_asset_stops_the_check() {
        let f =
            fixture(|server, key| vec![server.publish_signed(key, "2.0.0-beta.6", b"six")]).await;
        f.server
            .route("/assets/v2.0.0-beta.6/manifest.json", Reply::status(429));
        let filter = ChannelFilter::new(UpdateChannel::Auto, &v("2.0.0-beta.5"));
        assert!(matches!(
            newest(&f.source, &f.keys, filter, &v("2.0.0-beta.5")).await,
            Err(UpdateError::RateLimited { .. })
        ));
    }

    #[tokio::test]
    async fn a_pinned_version_may_be_older_and_reports_its_own_failure() {
        let f = fixture(|server, key| {
            vec![
                server.publish_signed(key, "2.0.0-beta.3", b"three"),
                forged(server, "2.0.0-beta.6"),
                server.publish("v2.0.0-beta.7", &[]),
            ]
        })
        .await;
        let older = pinned(&f.source, &f.keys, &v("2.0.0-beta.3"))
            .await
            .unwrap();
        assert_eq!(older.version(), &v("2.0.0-beta.3"));
        assert!(matches!(
            pinned(&f.source, &f.keys, &v("2.0.0-beta.6")).await,
            Err(UpdateError::Signature {
                source: SignatureError::UnknownKey,
                ..
            })
        ));
        assert!(matches!(
            pinned(&f.source, &f.keys, &v("2.0.0-beta.7")).await,
            Err(UpdateError::NoManifest { .. })
        ));
        assert!(matches!(
            pinned(&f.source, &f.keys, &v("9.9.9")).await,
            Err(UpdateError::VersionNotFound { .. })
        ));
    }
}

use std::path::Path;

use bytes::Bytes;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use super::error::UpdateError;
use super::manifest::Artifact;
use super::source::{ReleaseSource, network};

pub(crate) trait ChunkSource {
    async fn next_chunk(&mut self) -> Result<Option<Bytes>, UpdateError>;
}

struct Body<'a> {
    url: &'a str,
    response: reqwest::Response,
}

impl ChunkSource for Body<'_> {
    async fn next_chunk(&mut self) -> Result<Option<Bytes>, UpdateError> {
        self.response
            .chunk()
            .await
            .map_err(|e| network(self.url, e))
    }
}

pub async fn download(
    source: &ReleaseSource,
    url: &str,
    dest: &Path,
    artifact: &Artifact,
    progress: &(dyn Fn(u64, u64) + Sync),
) -> Result<(), UpdateError> {
    let response = source.download(url).await?;
    if response
        .content_length()
        .is_some_and(|len| len > artifact.size)
    {
        return Err(UpdateError::TooLarge {
            what: artifact.file.clone(),
            limit: artifact.size,
        });
    }
    write_verified(Body { url, response }, dest, artifact, progress).await
}

pub(crate) async fn write_verified(
    mut body: impl ChunkSource,
    dest: &Path,
    artifact: &Artifact,
    progress: &(dyn Fn(u64, u64) + Sync),
) -> Result<(), UpdateError> {
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dest)
        .await
        .map_err(|source| UpdateError::Io {
            context: format!("cannot create {}", dest.display()),
            source,
        })?;
    let result = stream(&mut body, &mut file, dest, artifact, progress).await;
    drop(file);
    if result.is_err() {
        let _ = tokio::fs::remove_file(dest).await;
    }
    result
}

async fn stream(
    body: &mut impl ChunkSource,
    file: &mut tokio::fs::File,
    dest: &Path,
    artifact: &Artifact,
    progress: &(dyn Fn(u64, u64) + Sync),
) -> Result<(), UpdateError> {
    let write_error = |source| UpdateError::Io {
        context: format!("cannot write {}", dest.display()),
        source,
    };
    let mut hasher = Sha256::new();
    let mut received: u64 = 0;
    while let Some(chunk) = body.next_chunk().await? {
        received = received.saturating_add(chunk.len() as u64);
        if received > artifact.size {
            return Err(UpdateError::TooLarge {
                what: artifact.file.clone(),
                limit: artifact.size,
            });
        }
        hasher.update(&chunk);
        file.write_all(&chunk).await.map_err(write_error)?;
        progress(received, artifact.size);
    }
    if received != artifact.size {
        return Err(UpdateError::SizeMismatch {
            file: artifact.file.clone(),
            expected: artifact.size,
            actual: received,
        });
    }
    file.flush().await.map_err(write_error)?;
    file.sync_all().await.map_err(write_error)?;
    let digest: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if !digest.eq_ignore_ascii_case(&artifact.sha256) {
        return Err(UpdateError::ShaMismatch {
            file: artifact.file.clone(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::VecDeque;

    use super::super::testing::sha256_hex;
    use super::*;

    struct Chunks {
        queue: VecDeque<Bytes>,
        served: usize,
    }

    impl Chunks {
        fn new(chunks: &[&[u8]]) -> Self {
            Self {
                queue: chunks.iter().map(|c| Bytes::copy_from_slice(c)).collect(),
                served: 0,
            }
        }
    }

    impl ChunkSource for &mut Chunks {
        async fn next_chunk(&mut self) -> Result<Option<Bytes>, UpdateError> {
            let next = self.queue.pop_front();
            if next.is_some() {
                self.served += 1;
            }
            Ok(next)
        }
    }

    fn artifact(content: &[u8]) -> Artifact {
        Artifact {
            targets: vec!["x86_64-unknown-linux-musl".to_string()],
            file: "infrarust-linux-x86_64.tar.gz".to_string(),
            binary: "infrarust".to_string(),
            sha256: sha256_hex(content),
            size: content.len() as u64,
        }
    }

    fn ignore(_: u64, _: u64) {}

    #[tokio::test]
    async fn a_matching_stream_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("archive");
        let mut chunks = Chunks::new(&[b"hello ", b"world"]);
        write_verified(&mut chunks, &dest, &artifact(b"hello world"), &ignore)
            .await
            .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"hello world");
    }

    #[tokio::test]
    async fn a_sha_mismatch_deletes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("archive");
        let mut chunks = Chunks::new(&[b"hello wurld"]);
        let result = write_verified(&mut chunks, &dest, &artifact(b"hello world"), &ignore).await;
        assert!(
            matches!(result, Err(UpdateError::ShaMismatch { .. })),
            "{result:?}"
        );
        assert!(!dest.exists());
    }

    #[tokio::test]
    async fn an_oversized_stream_is_cut_at_the_announced_size() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("archive");
        let mut chunks = Chunks::new(&[b"hello", b" world", b"!!!", b"never read"]);
        let result = write_verified(&mut chunks, &dest, &artifact(b"hello worl"), &ignore).await;
        assert!(
            matches!(result, Err(UpdateError::TooLarge { .. })),
            "{result:?}"
        );
        assert_eq!(chunks.served, 2);
        assert!(!dest.exists());
    }

    #[tokio::test]
    async fn a_short_body_deletes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("archive");
        let mut chunks = Chunks::new(&[b"hello"]);
        let result = write_verified(&mut chunks, &dest, &artifact(b"hello world"), &ignore).await;
        assert!(
            matches!(
                result,
                Err(UpdateError::SizeMismatch {
                    expected: 11,
                    actual: 5,
                    ..
                })
            ),
            "{result:?}"
        );
        assert!(!dest.exists());
    }

    #[tokio::test]
    async fn an_existing_file_is_never_overwritten_or_removed() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("archive");
        std::fs::write(&dest, b"someone else's").unwrap();
        let mut chunks = Chunks::new(&[b"hello"]);
        let result = write_verified(&mut chunks, &dest, &artifact(b"hello"), &ignore).await;
        assert!(matches!(result, Err(UpdateError::Io { .. })), "{result:?}");
        assert_eq!(std::fs::read(&dest).unwrap(), b"someone else's");
    }

    #[tokio::test]
    async fn progress_reports_every_chunk() {
        use std::sync::Mutex;

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("archive");
        let seen = Mutex::new(Vec::new());
        let record = |received, total| seen.lock().unwrap().push((received, total));
        let mut chunks = Chunks::new(&[b"ab", b"cd"]);
        write_verified(&mut chunks, &dest, &artifact(b"abcd"), &record)
            .await
            .unwrap();
        assert_eq!(*seen.lock().unwrap(), [(2, 4), (4, 4)]);
    }
}

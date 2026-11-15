use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::io::AsyncWriteExt;

#[derive(Debug, thiserror::Error)]
#[error("{path}: {source}")]
pub struct WriteError {
    pub path: PathBuf,
    pub source: std::io::Error,
}

pub async fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), WriteError> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".{}.tmp", SEQUENCE.fetch_add(1, Ordering::Relaxed)));
    let tmp = PathBuf::from(name);

    let written = match write_synced(&tmp, bytes).await {
        Ok(()) => tokio::fs::rename(&tmp, path)
            .await
            .map_err(|source| WriteError {
                path: path.to_path_buf(),
                source,
            }),
        Err(source) => Err(WriteError {
            path: tmp.clone(),
            source,
        }),
    };
    if written.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
    }
    written
}

async fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = tokio::fs::File::create(path).await?;
    file.write_all(bytes).await?;
    file.sync_all().await
}

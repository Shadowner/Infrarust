use std::path::{Path, PathBuf};

/// Writes through a sibling temporary file so a crash mid-write cannot leave
/// a truncated file behind.
///
/// The rename replaces the destination's inode, so the scratch file is created
/// private and takes on the destination's permissions before it lands: a
/// file an operator restricted stays restricted.
pub(crate) fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;

    let mut scratch = ScratchFile::create_beside(path)?;
    scratch.file.write_all(text.as_bytes())?;
    scratch.inherit_permissions(path)?;
    scratch.persist(path)?;

    // Best effort: fsync on a directory handle is not portable, and the
    // rename already landed.
    if let Some(dir) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        let _ = std::fs::File::open(dir).and_then(|d| d.sync_all());
    }
    Ok(())
}

/// How many names a scratch file tries before giving up.
const SCRATCH_ATTEMPTS: u32 = 32;

/// A file that removes itself unless it is [persisted](ScratchFile::persist),
/// under a name no concurrent writer can be using.
struct ScratchFile {
    path: PathBuf,
    file: std::fs::File,
    persisted: bool,
}

impl ScratchFile {
    fn create_beside(destination: &Path) -> std::io::Result<Self> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NONCE: AtomicU64 = AtomicU64::new(0);

        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }

        for _ in 0..SCRATCH_ATTEMPTS {
            let mut name = destination.file_name().unwrap_or_default().to_os_string();
            name.push(format!(
                ".{}.{}.tmp",
                std::process::id(),
                NONCE.fetch_add(1, Ordering::Relaxed)
            ));
            let path = destination.with_file_name(name);

            match options.open(&path) {
                Ok(file) => {
                    return Ok(Self {
                        path,
                        file,
                        persisted: false,
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }

        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!(
                "no free scratch file name next to {}",
                destination.display()
            ),
        ))
    }

    #[cfg(unix)]
    fn inherit_permissions(&self, destination: &Path) -> std::io::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let Ok(metadata) = std::fs::metadata(destination) else {
            return Ok(());
        };
        let mode = metadata.permissions().mode() & 0o7777;
        self.file
            .set_permissions(std::fs::Permissions::from_mode(mode))
    }

    #[cfg(not(unix))]
    fn inherit_permissions(&self, _destination: &Path) -> std::io::Result<()> {
        Ok(())
    }

    fn persist(&mut self, destination: &Path) -> std::io::Result<()> {
        self.file.sync_all()?;
        std::fs::rename(&self.path, destination)?;
        self.persisted = true;
        Ok(())
    }
}

impl Drop for ScratchFile {
    fn drop(&mut self) {
        if !self.persisted {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn a_failed_write_leaves_no_scratch_file_behind() {
        let root = tempfile::tempdir().unwrap();
        let occupied = root.path().join("infrarust.toml");
        std::fs::create_dir(&occupied).unwrap();

        let error = write_atomic(&occupied, "bind = \"0.0.0.0:25565\"\n").unwrap_err();

        assert!(error.kind() != std::io::ErrorKind::NotFound);
        let leftovers: Vec<_> = std::fs::read_dir(root.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name())
            .filter(|name| name.to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "left behind {leftovers:?}");
    }

    #[test]
    fn two_writers_never_share_a_scratch_file() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("infrarust.toml");

        let first = ScratchFile::create_beside(&destination).unwrap();
        let second = ScratchFile::create_beside(&destination).unwrap();

        assert_ne!(first.path, second.path);
    }
}

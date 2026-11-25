use std::fs::Metadata;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExeIdentity {
    dev: u64,
    ino: u64,
    len: u64,
    modified: Option<SystemTime>,
}

impl ExeIdentity {
    pub fn of(path: &Path) -> io::Result<Self> {
        std::fs::metadata(path).map(|meta| Self::from_metadata(&meta))
    }

    #[cfg(unix)]
    fn from_metadata(meta: &Metadata) -> Self {
        use std::os::unix::fs::MetadataExt;
        Self {
            dev: meta.dev(),
            ino: meta.ino(),
            len: meta.len(),
            modified: meta.modified().ok(),
        }
    }

    #[cfg(not(unix))]
    fn from_metadata(meta: &Metadata) -> Self {
        Self {
            dev: 0,
            ino: 0,
            len: meta.len(),
            modified: meta.modified().ok(),
        }
    }
}

#[derive(Debug)]
pub struct ExeSnapshot {
    path: PathBuf,
    identity: Option<ExeIdentity>,
    #[cfg(target_os = "linux")]
    image: Option<std::fs::File>,
}

impl ExeSnapshot {
    pub fn capture() -> io::Result<Self> {
        let path = std::env::current_exe()?;
        let path = path.canonicalize().unwrap_or(path);
        Ok(Self {
            identity: ExeIdentity::of(&path).ok(),
            path,
            #[cfg(target_os = "linux")]
            image: std::fs::File::open("/proc/self/exe").ok(),
        })
    }

    pub fn at(path: PathBuf) -> Self {
        Self {
            identity: ExeIdentity::of(&path).ok(),
            path,
            #[cfg(target_os = "linux")]
            image: None,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub const fn identity(&self) -> Option<ExeIdentity> {
        self.identity
    }

    pub fn changed_on_disk(&self) -> bool {
        match (self.identity, ExeIdentity::of(&self.path)) {
            (Some(before), Ok(now)) => before != now,
            (Some(_), Err(_)) => true,
            (None, _) => false,
        }
    }

    #[cfg(target_os = "linux")]
    pub const fn image(&self) -> Option<&std::fs::File> {
        self.image.as_ref()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_replaced_file_reads_as_changed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("infrarust");
        std::fs::write(&path, b"old").unwrap();
        let snapshot = ExeSnapshot::at(path.clone());
        assert!(!snapshot.changed_on_disk());

        let staged = dir.path().join(".infrarust.new");
        std::fs::write(&staged, b"new binary").unwrap();
        std::fs::rename(&staged, &path).unwrap();
        assert!(snapshot.changed_on_disk());
    }

    #[test]
    fn a_removed_file_reads_as_changed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("infrarust");
        std::fs::write(&path, b"old").unwrap();
        let snapshot = ExeSnapshot::at(path.clone());
        std::fs::remove_file(&path).unwrap();
        assert!(snapshot.changed_on_disk());
    }

    #[test]
    fn a_missing_file_has_no_identity_and_never_changes() {
        let snapshot = ExeSnapshot::at(PathBuf::from("/nonexistent/infrarust"));
        assert!(snapshot.identity().is_none());
        assert!(!snapshot.changed_on_disk());
    }
}

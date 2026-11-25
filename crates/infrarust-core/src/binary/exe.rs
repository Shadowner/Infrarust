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
        let path = path
            .canonicalize()
            .unwrap_or_else(|_| without_deleted_mark(path));
        Ok(Self {
            identity: running_identity(&path),
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

#[cfg(target_os = "linux")]
const DELETED_MARK: &[u8] = b" (deleted)";

#[cfg(target_os = "linux")]
fn without_deleted_mark(path: PathBuf) -> PathBuf {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    match path.as_os_str().as_bytes().strip_suffix(DELETED_MARK) {
        Some(launched) if !launched.is_empty() => {
            PathBuf::from(std::ffi::OsString::from_vec(launched.to_vec()))
        }
        _ => path,
    }
}

#[cfg(not(target_os = "linux"))]
fn without_deleted_mark(path: PathBuf) -> PathBuf {
    path
}

#[cfg(target_os = "linux")]
fn running_identity(path: &Path) -> Option<ExeIdentity> {
    ExeIdentity::of(Path::new("/proc/self/exe"))
        .or_else(|_| ExeIdentity::of(path))
        .ok()
}

#[cfg(not(target_os = "linux"))]
fn running_identity(path: &Path) -> Option<ExeIdentity> {
    ExeIdentity::of(path).ok()
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

    #[cfg(target_os = "linux")]
    #[test]
    fn an_image_whose_file_was_replaced_keeps_the_path_it_was_launched_from() {
        assert_eq!(
            without_deleted_mark(PathBuf::from("/opt/infrarust/infrarust (deleted)")),
            PathBuf::from("/opt/infrarust/infrarust")
        );
        assert_eq!(
            without_deleted_mark(PathBuf::from("/opt/infrarust/infrarust")),
            PathBuf::from("/opt/infrarust/infrarust")
        );
        assert_eq!(
            without_deleted_mark(PathBuf::from(" (deleted)")),
            PathBuf::from(" (deleted)")
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_captured_identity_is_the_one_of_the_running_image() {
        let snapshot = ExeSnapshot::capture().unwrap();
        assert_eq!(
            snapshot.identity(),
            ExeIdentity::of(Path::new("/proc/self/exe")).ok()
        );
        assert!(!snapshot.changed_on_disk());
    }

    #[test]
    fn a_missing_file_has_no_identity_and_never_changes() {
        let snapshot = ExeSnapshot::at(PathBuf::from("/nonexistent/infrarust"));
        assert!(snapshot.identity().is_none());
        assert!(!snapshot.changed_on_disk());
    }
}

use std::fs::File;
use std::io::{self, BufReader, Read, Write};
use std::path::Path;

pub const MAX_BINARY_SIZE: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    TarGz,
    Zip,
}

impl ArchiveFormat {
    pub fn for_target(target: &str) -> Self {
        if target.contains("-windows") {
            Self::Zip
        } else {
            Self::TarGz
        }
    }

    pub const fn extension(self) -> &'static str {
        match self {
            Self::TarGz => ".tar.gz",
            Self::Zip => ".zip",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("{file} is not a {expected} archive")]
    WrongFormat {
        file: String,
        expected: &'static str,
    },
    #[error("the archive cannot be read: {0}")]
    Corrupt(String),
    #[error("the archive has no regular file named {0}")]
    Missing(String),
    #[error("the archive holds {0} more than once")]
    Duplicate(String),
    #[error("the archive entry {0} is not a regular file")]
    NotRegular(String),
    #[error("the archive entry {name} is larger than {limit} bytes")]
    TooLarge { name: String, limit: u64 },
    #[error("cannot extract the binary: {0}")]
    Io(#[source] io::Error),
}

pub fn extract(
    archive: &Path,
    format: ArchiveFormat,
    binary: &str,
    out: &mut impl Write,
) -> Result<u64, ArchiveError> {
    let file = File::open(archive).map_err(ArchiveError::Io)?;
    let reader = BufReader::new(file);
    match format {
        ArchiveFormat::TarGz => extract_tar(reader, binary, out),
        ArchiveFormat::Zip => extract_zip(reader, binary, out),
    }
}

fn names_binary(raw: &[u8], binary: &str) -> bool {
    let name = raw.strip_prefix(b"./").unwrap_or(raw);
    name == binary.as_bytes()
}

fn extract_tar(reader: impl Read, binary: &str, out: &mut impl Write) -> Result<u64, ArchiveError> {
    let corrupt = |e: io::Error| ArchiveError::Corrupt(e.to_string());
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(reader));
    let mut written = None;
    for entry in archive.entries().map_err(corrupt)? {
        let mut entry = entry.map_err(corrupt)?;
        if !names_binary(&entry.path_bytes(), binary) {
            continue;
        }
        if written.is_some() {
            return Err(ArchiveError::Duplicate(binary.to_string()));
        }
        if entry.header().entry_type() != tar::EntryType::Regular {
            return Err(ArchiveError::NotRegular(binary.to_string()));
        }
        written = Some(copy_limited(&mut entry, binary, out)?);
    }
    written.ok_or_else(|| ArchiveError::Missing(binary.to_string()))
}

fn extract_zip(
    reader: impl Read + io::Seek,
    binary: &str,
    out: &mut impl Write,
) -> Result<u64, ArchiveError> {
    let corrupt = |e: zip::result::ZipError| ArchiveError::Corrupt(e.to_string());
    let mut archive = zip::ZipArchive::new(reader).map_err(corrupt)?;
    let mut found = None;
    for index in 0..archive.len() {
        let entry = archive.by_index_raw(index).map_err(corrupt)?;
        if !names_binary(entry.name_raw(), binary) {
            continue;
        }
        if found.is_some() {
            return Err(ArchiveError::Duplicate(binary.to_string()));
        }
        if !entry.is_file() {
            return Err(ArchiveError::NotRegular(binary.to_string()));
        }
        found = Some(index);
    }
    let index = found.ok_or_else(|| ArchiveError::Missing(binary.to_string()))?;
    let mut entry = archive.by_index(index).map_err(corrupt)?;
    copy_limited(&mut entry, binary, out)
}

fn copy_limited(
    reader: &mut impl Read,
    name: &str,
    out: &mut impl Write,
) -> Result<u64, ArchiveError> {
    let copied = io::copy(&mut reader.take(MAX_BINARY_SIZE + 1), out).map_err(|e| {
        if e.kind() == io::ErrorKind::InvalidData || e.kind() == io::ErrorKind::UnexpectedEof {
            ArchiveError::Corrupt(e.to_string())
        } else {
            ArchiveError::Io(e)
        }
    })?;
    if copied > MAX_BINARY_SIZE {
        return Err(ArchiveError::TooLarge {
            name: name.to_string(),
            limit: MAX_BINARY_SIZE,
        });
    }
    Ok(copied)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::path::PathBuf;

    use super::super::testing::{Entry, tar_gz, zip_archive};
    use super::*;

    fn run(
        format: ArchiveFormat,
        entries: &[Entry],
    ) -> (Result<u64, ArchiveError>, Vec<u8>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let bytes = match format {
            ArchiveFormat::TarGz => tar_gz(entries),
            ArchiveFormat::Zip => zip_archive(entries),
        };
        let nested = dir.path().join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        let path: PathBuf = nested.join(format!("archive{}", format.extension()));
        std::fs::write(&path, bytes).unwrap();
        let mut out = Vec::new();
        let result = extract(&path, format, "infrarust", &mut out);
        (result, out, dir)
    }

    fn both(check: impl Fn(ArchiveFormat)) {
        check(ArchiveFormat::TarGz);
        check(ArchiveFormat::Zip);
    }

    #[test]
    fn the_root_binary_is_extracted() {
        both(|format| {
            let (result, out, _dir) = run(
                format,
                &[
                    Entry::file("README.md", b"docs"),
                    Entry::file("infrarust", b"ELF"),
                ],
            );
            assert_eq!(result.unwrap(), 3, "{format:?}");
            assert_eq!(out, b"ELF");
        });
    }

    #[test]
    fn a_dot_slash_prefix_is_accepted() {
        both(|format| {
            let (result, out, _dir) = run(format, &[Entry::file("./infrarust", b"ELF")]);
            assert!(result.is_ok(), "{format:?}: {result:?}");
            assert_eq!(out, b"ELF");
        });
    }

    #[test]
    fn a_parent_path_is_never_extracted() {
        both(|format| {
            let (result, out, dir) = run(
                format,
                &[
                    Entry::file("../x", b"escape"),
                    Entry::file("infrarust", b"ELF"),
                ],
            );
            assert!(result.is_ok(), "{format:?}: {result:?}");
            assert_eq!(out, b"ELF");
            assert!(!dir.path().join("a").join("x").exists());

            let (result, _, _dir) = run(format, &[Entry::file("../infrarust", b"escape")]);
            assert!(
                matches!(result, Err(ArchiveError::Missing(_))),
                "{format:?}: {result:?}"
            );
        });
    }

    #[test]
    fn a_nested_binary_does_not_count() {
        both(|format| {
            let (result, _, _dir) = run(format, &[Entry::file("bin/infrarust", b"ELF")]);
            assert!(
                matches!(result, Err(ArchiveError::Missing(_))),
                "{format:?}: {result:?}"
            );
        });
    }

    #[test]
    fn a_symlink_named_like_the_binary_is_refused() {
        both(|format| {
            let (result, _, _dir) = run(format, &[Entry::symlink("infrarust", "/bin/sh")]);
            assert!(
                matches!(result, Err(ArchiveError::NotRegular(_))),
                "{format:?}: {result:?}"
            );
        });
    }

    #[test]
    fn a_directory_named_like_the_binary_is_refused() {
        both(|format| {
            let (result, _, _dir) = run(format, &[Entry::dir("infrarust")]);
            assert!(
                matches!(
                    result,
                    Err(ArchiveError::NotRegular(_) | ArchiveError::Missing(_))
                ),
                "{format:?}: {result:?}"
            );
        });
    }

    #[test]
    fn a_hardlink_named_like_the_binary_is_refused() {
        let (result, _, _dir) = run(
            ArchiveFormat::TarGz,
            &[
                Entry::file("other", b"ELF"),
                Entry::hardlink("infrarust", "other"),
            ],
        );
        assert!(
            matches!(result, Err(ArchiveError::NotRegular(_))),
            "{result:?}"
        );
    }

    #[test]
    fn other_links_are_ignored() {
        let (result, out, _dir) = run(
            ArchiveFormat::TarGz,
            &[
                Entry::symlink("sh", "/bin/sh"),
                Entry::hardlink("again", "infrarust"),
                Entry::file("infrarust", b"ELF"),
            ],
        );
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(out, b"ELF");
    }

    #[test]
    fn a_duplicate_entry_is_refused() {
        both(|format| {
            let (result, _, _dir) = run(
                format,
                &[
                    Entry::file("infrarust", b"one"),
                    Entry::file("./infrarust", b"two"),
                ],
            );
            assert!(
                matches!(result, Err(ArchiveError::Duplicate(_))),
                "{format:?}: {result:?}"
            );
        });
        let (result, _, _dir) = run(
            ArchiveFormat::TarGz,
            &[
                Entry::file("infrarust", b"one"),
                Entry::file("infrarust", b"two"),
            ],
        );
        assert!(
            matches!(result, Err(ArchiveError::Duplicate(_))),
            "{result:?}"
        );
    }

    #[test]
    fn a_missing_entry_is_reported() {
        both(|format| {
            let (result, _, _dir) = run(format, &[Entry::file("infrarust.exe", b"PE")]);
            assert!(
                matches!(result, Err(ArchiveError::Missing(_))),
                "{format:?}: {result:?}"
            );
        });
    }

    #[test]
    fn garbage_is_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("archive");
        std::fs::write(&path, b"definitely not an archive").unwrap();
        for format in [ArchiveFormat::TarGz, ArchiveFormat::Zip] {
            let result = extract(&path, format, "infrarust", &mut Vec::new());
            assert!(
                matches!(result, Err(ArchiveError::Corrupt(_))),
                "{format:?}: {result:?}"
            );
        }
    }

    #[test]
    fn the_format_follows_the_target() {
        assert_eq!(
            ArchiveFormat::for_target("x86_64-pc-windows-msvc"),
            ArchiveFormat::Zip
        );
        assert_eq!(
            ArchiveFormat::for_target("aarch64-apple-darwin"),
            ArchiveFormat::TarGz
        );
        assert_eq!(
            ArchiveFormat::for_target("x86_64-unknown-linux-musl"),
            ArchiveFormat::TarGz
        );
    }
}

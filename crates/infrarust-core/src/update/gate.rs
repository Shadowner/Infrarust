use std::fmt;
use std::fs::OpenOptions;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Once;

use crate::binary::BuildInfo;

use super::platform::BinaryPaths;

const DOCKERENV: &str = "/.dockerenv";
const RELEASES_PAGE: &str = "https://github.com/Shadowner/Infrarust/releases";
const IMAGE: &str = "ghcr.io/shadowner/infrarust";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    NotOfficial,
    Docker,
    NotWritable { dir: PathBuf, reason: String },
    UnsupportedTarget { target: String },
}

impl Refusal {
    pub fn instructions(&self, version: Option<&str>) -> String {
        match self {
            Self::NotOfficial => {
                let install = match version {
                    Some(version) => format!("cargo install infrarust --version {version}"),
                    None => "cargo install infrarust".to_string(),
                };
                format!(
                    "this build was not made by the release pipeline; install with `{install}` or download the release from {RELEASES_PAGE}"
                )
            }
            Self::Docker => format!(
                "this binary comes from a container image; update the image with `docker pull {IMAGE}:{}` and recreate the container",
                version.unwrap_or("latest")
            ),
            Self::NotWritable { dir, reason } => format!(
                "cannot write to {} ({reason}); download the archive for this platform from {RELEASES_PAGE}, extract it, replace the binary in that directory by hand, then restart the proxy",
                dir.display()
            ),
            Self::UnsupportedTarget { target } => {
                let install = match version {
                    Some(version) => format!("cargo install infrarust --version {version}"),
                    None => "cargo install infrarust".to_string(),
                };
                format!(
                    "no release binary is published for {target}; build it from source with `{install}`"
                )
            }
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.instructions(None))
    }
}

pub fn build_refusal(build: &BuildInfo) -> Option<Refusal> {
    if build.docker {
        Some(Refusal::Docker)
    } else if !build.official {
        Some(Refusal::NotOfficial)
    } else if !supported_target(build.target) {
        Some(Refusal::UnsupportedTarget {
            target: build.target.to_string(),
        })
    } else {
        None
    }
}

pub fn supported_target(target: &str) -> bool {
    let arch = target.starts_with("x86_64-") || target.starts_with("aarch64-");
    let os = target.contains("-linux-")
        || target.ends_with("-apple-darwin")
        || target.contains("-windows-");
    arch && os
}

pub fn paths_for(exe: &Path) -> Result<BinaryPaths, Refusal> {
    BinaryPaths::for_exe(exe).ok_or_else(|| Refusal::NotWritable {
        dir: exe.to_path_buf(),
        reason: "the executable path has no directory".to_string(),
    })
}

pub fn check_writable(paths: &BinaryPaths) -> Result<(), Refusal> {
    warn_if_container(Path::new(DOCKERENV));
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&paths.staged)
    {
        Ok(file) => {
            drop(file);
            let _ = std::fs::remove_file(&paths.staged);
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(Refusal::NotWritable {
            dir: paths.dir.clone(),
            reason: e.to_string(),
        }),
    }
}

#[cfg(test)]
fn evaluate(build: &BuildInfo, exe: &Path) -> Result<BinaryPaths, Refusal> {
    if let Some(refusal) = build_refusal(build) {
        return Err(refusal);
    }
    let paths = paths_for(exe)?;
    check_writable(&paths)?;
    Ok(paths)
}

fn warn_if_container(marker: &Path) {
    static WARNED: Once = Once::new();
    if marker.exists() {
        WARNED.call_once(|| {
            tracing::warn!(
                "{} exists: this proxy seems to run in a container, an updated binary is lost when the container is recreated",
                marker.display()
            );
        });
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    const OFFICIAL: BuildInfo = BuildInfo {
        version: "2.0.0-beta.5",
        target: "x86_64-unknown-linux-musl",
        official: true,
        docker: false,
    };

    #[test]
    fn a_build_outside_the_pipeline_is_refused() {
        let build = BuildInfo {
            official: false,
            ..OFFICIAL
        };
        let refusal = build_refusal(&build).unwrap();
        assert_eq!(refusal, Refusal::NotOfficial);
        let text = refusal.instructions(Some("2.0.0-beta.6"));
        assert!(text.contains("not made by the release pipeline"), "{text}");
        assert!(
            text.contains("`cargo install infrarust --version 2.0.0-beta.6`"),
            "{text}"
        );
    }

    #[test]
    fn a_docker_build_is_refused_with_a_pull_command() {
        let build = BuildInfo {
            docker: true,
            ..OFFICIAL
        };
        let refusal = build_refusal(&build).unwrap();
        assert_eq!(refusal, Refusal::Docker);
        assert!(
            refusal
                .instructions(Some("2.0.0-beta.6"))
                .contains("`docker pull ghcr.io/shadowner/infrarust:2.0.0-beta.6`")
        );
    }

    #[test]
    fn a_docker_image_is_told_to_pull_even_when_built_outside_the_pipeline() {
        let build = BuildInfo {
            official: false,
            docker: true,
            ..OFFICIAL
        };
        assert_eq!(build_refusal(&build), Some(Refusal::Docker));
    }

    #[test]
    fn an_unpublished_target_is_refused() {
        for target in [
            "riscv64gc-unknown-linux-gnu",
            "x86_64-unknown-freebsd",
            "unknown",
        ] {
            let build = BuildInfo { target, ..OFFICIAL };
            assert_eq!(
                build_refusal(&build),
                Some(Refusal::UnsupportedTarget {
                    target: target.to_string()
                })
            );
        }
        for target in [
            "x86_64-unknown-linux-musl",
            "aarch64-unknown-linux-musl",
            "x86_64-unknown-linux-gnu",
            "x86_64-apple-darwin",
            "aarch64-apple-darwin",
            "x86_64-pc-windows-msvc",
            "aarch64-pc-windows-msvc",
        ] {
            assert!(supported_target(target), "{target}");
        }
    }

    #[test]
    fn a_missing_directory_is_not_writable() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("gone").join("infrarust");
        match evaluate(&OFFICIAL, &exe) {
            Err(Refusal::NotWritable { dir: refused, .. }) => {
                assert_eq!(refused, dir.path().join("gone"));
            }
            other => panic!("expected NotWritable, got {other:?}"),
        }
        let text = Refusal::NotWritable {
            dir: PathBuf::from("/opt/infrarust"),
            reason: "permission denied".to_string(),
        }
        .instructions(None);
        assert!(text.contains("/opt/infrarust"), "{text}");
        assert!(text.contains("replace the binary"), "{text}");
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_directory_is_not_writable() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        std::fs::write(bin.join("infrarust"), b"exe").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o555)).unwrap();
        let writable_anyway = std::fs::write(bin.join("probe"), b"").is_ok();
        let result = evaluate(&OFFICIAL, &bin.join("infrarust"));
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        if writable_anyway {
            return;
        }
        assert!(
            matches!(result, Err(Refusal::NotWritable { .. })),
            "{result:?}"
        );
    }

    #[test]
    fn a_writable_directory_passes_and_keeps_no_probe_file() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("infrarust");
        std::fs::write(&exe, b"exe").unwrap();
        let paths = evaluate(&OFFICIAL, &exe).unwrap();
        assert!(!paths.staged.exists());
        let left: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(left, ["infrarust"]);
    }
}

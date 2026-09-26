use std::path::PathBuf;

use infrarust_api::permissions::{Capability, CapabilitySet};
use infrarust_config::WasmMount;
use wasmtime_wasi::{DirPerms, FilePerms};

use crate::error::WasmLoaderError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Mount {
    pub(crate) host: PathBuf,
    pub(crate) guest: String,
    pub(crate) read_only: bool,
}

impl Mount {
    pub(crate) fn perms(&self) -> (DirPerms, FilePerms) {
        if self.read_only {
            (DirPerms::READ, FilePerms::READ)
        } else {
            (DirPerms::all(), FilePerms::all())
        }
    }
}

pub(crate) fn resolve_mounts(
    plugin_id: &str,
    mounts: &[WasmMount],
    capabilities: &CapabilitySet,
) -> Result<Vec<Mount>, WasmLoaderError> {
    if mounts.is_empty() {
        return Ok(Vec::new());
    }
    if !capabilities.has(Capability::FilesystemExtended) {
        tracing::warn!(
            plugin = %plugin_id,
            mounts = mounts.len(),
            "plugins.{plugin_id}.wasm.mounts is ignored: the plugin lacks the `filesystem-extended` capability, so nothing is mounted"
        );
        return Ok(Vec::new());
    }
    let scope = format!("plugins.{plugin_id}.wasm.mounts");
    mounts
        .iter()
        .map(|mount| {
            let guest = mount
                .guest_path()
                .map_err(|reason| WasmLoaderError::Config(format!("{scope}: {reason}")))?;
            let host = std::fs::canonicalize(&mount.host).map_err(|error| {
                WasmLoaderError::Config(format!(
                    "{scope}: host directory {} for {guest} cannot be opened: {error}",
                    mount.host.display()
                ))
            })?;
            if !host.is_dir() {
                return Err(WasmLoaderError::Config(format!(
                    "{scope}: host path {} for {guest} is not a directory",
                    host.display()
                )));
            }
            tracing::info!(
                plugin = %plugin_id,
                host = %host.display(),
                guest = %guest,
                read_only = mount.read_only,
                "wasm plugin mount"
            );
            Ok(Mount {
                host,
                guest,
                read_only: mount.read_only,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mount(host: &std::path::Path, guest: &str, read_only: bool) -> WasmMount {
        WasmMount {
            host: host.to_path_buf(),
            guest: guest.to_owned(),
            read_only,
        }
    }

    #[test]
    fn mounts_are_canonicalised_and_carry_their_permissions() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("shared")).unwrap();
        let granted = CapabilitySet::baseline().with(Capability::FilesystemExtended);
        let resolved = resolve_mounts(
            "p",
            &[
                mount(&dir.path().join("shared/../shared"), "/shared/", true),
                mount(dir.path(), "/rw", false),
            ],
            &granted,
        )
        .unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        assert_eq!(resolved[0].host, root.join("shared"));
        assert_eq!(resolved[0].guest, "/shared");
        assert_eq!(resolved[0].perms(), (DirPerms::READ, FilePerms::READ));
        assert_eq!(resolved[1].host, root);
        assert_eq!(resolved[1].perms(), (DirPerms::all(), FilePerms::all()));
    }

    #[test]
    fn a_missing_host_directory_names_the_plugin_scope_and_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("gone");
        let granted = CapabilitySet::baseline().with(Capability::FilesystemExtended);
        let err = resolve_mounts("libertybans", &[mount(&missing, "/shared", true)], &granted)
            .unwrap_err()
            .into_loader_error("libertybans")
            .to_string();
        assert!(err.contains("libertybans"), "{err}");
        assert!(err.contains("plugins.libertybans.wasm.mounts"), "{err}");
        assert!(err.contains(&missing.display().to_string()), "{err}");

        let file = dir.path().join("file");
        std::fs::write(&file, "x").unwrap();
        let err = resolve_mounts("p", &[mount(&file, "/f", true)], &granted)
            .unwrap_err()
            .to_string();
        assert!(err.contains("is not a directory"), "{err}");
    }

    #[test]
    fn without_the_capability_nothing_is_mounted_and_nothing_is_checked() {
        let dir = tempfile::tempdir().unwrap();
        let resolved = resolve_mounts(
            "p",
            &[mount(&dir.path().join("gone"), "/shared", true)],
            &CapabilitySet::baseline(),
        )
        .unwrap();
        assert!(resolved.is_empty());
    }
}

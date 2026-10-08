//! The frozen WIT contract for Infrarust WASM plugins (`infrarust:plugin@0.3.0`).
//!
//! This crate owns the `wit/` directory so the host loader and the guest SDK
//! generate bindings from one source of truth.

pub mod arena;

pub const WORLD_VERSION: &str = "0.3.0";

pub const PACKAGE: &str = "infrarust:plugin";

pub const WIT_DIR: &str = "wit";

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    const FROZEN_WIT_HASH: u64 = 0xe76b_f153_9e51_eb30;

    fn fnv1a(hash: u64, bytes: &[u8]) -> u64 {
        bytes.iter().fold(hash, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
        })
    }

    #[test]
    fn the_frozen_contract_is_unchanged() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(super::WIT_DIR);
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        files.sort();
        let hash = files.iter().fold(0xcbf2_9ce4_8422_2325, |hash, path| {
            let name = path.file_name().unwrap().to_string_lossy();
            let text = std::fs::read_to_string(path).unwrap().replace('\r', "");
            fnv1a(fnv1a(hash, name.as_bytes()), text.as_bytes())
        });
        assert_eq!(
            hash,
            FROZEN_WIT_HASH,
            "wit/ changed but infrarust:plugin@{} is frozen: publish the change under a new \
             WORLD_VERSION, or set FROZEN_WIT_HASH to {hash:#x} if the edit leaves the contract \
             as it was (comments, formatting)",
            super::WORLD_VERSION
        );
    }

    #[test]
    fn world_version_matches_wit_package() {
        let world = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/wit/world.wit"));
        let decl = format!("package {}@{};", super::PACKAGE, super::WORLD_VERSION);
        assert!(
            world.contains(&decl),
            "WORLD_VERSION ({}) does not match the package declaration in wit/world.wit",
            super::WORLD_VERSION
        );
    }
}

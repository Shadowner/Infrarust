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

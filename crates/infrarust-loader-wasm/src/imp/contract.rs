use std::path::Path;

use infrarust_plugin_wit::PACKAGE;
use wasmtime::Engine;
use wasmtime::component::Component;

use crate::consts::WORLD_VERSION;
use crate::error::WasmLoaderError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Mismatch {
    NotAPlugin,
    Incompatible { found: String },
}

pub(crate) fn check(
    engine: &Engine,
    component: &Component,
    path: &Path,
) -> Result<(), WasmLoaderError> {
    let prefix = format!("{PACKAGE}/guest@");
    let found = component
        .component_type()
        .exports(engine)
        .find_map(|(name, _)| name.strip_prefix(&prefix).map(str::to_owned));
    verdict(found.as_deref(), WORLD_VERSION).map_err(|mismatch| match mismatch {
        Mismatch::NotAPlugin => WasmLoaderError::NotAPlugin {
            path: path.to_path_buf(),
        },
        Mismatch::Incompatible { found } => WasmLoaderError::WorldIncompatible {
            path: path.to_path_buf(),
            found: format!("{PACKAGE}@{found}"),
            expected: format!("{PACKAGE}@{}", supported(WORLD_VERSION)),
        },
    })
}

pub(crate) fn verdict(found: Option<&str>, host: &str) -> Result<(), Mismatch> {
    let found = found.ok_or(Mismatch::NotAPlugin)?;
    let compatible = match (parse(found), parse(host)) {
        (Some(guest), Some(host)) => guest.0 == host.0 && guest.1 == host.1 && guest.2 <= host.2,
        _ => false,
    };
    if compatible {
        Ok(())
    } else {
        Err(Mismatch::Incompatible {
            found: found.to_owned(),
        })
    }
}

fn supported(host: &str) -> String {
    match parse(host) {
        Some((major, minor, _)) => format!("{major}.{minor}.x"),
        None => host.to_owned(),
    }
}

fn parse(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.split('.');
    let mut next = || parts.next()?.parse::<u64>().ok();
    let parsed = (next()?, next()?, next()?);
    parts.next().is_none().then_some(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> Engine {
        let config: infrarust_config::ProxyConfig = toml::from_str("").unwrap();
        crate::engine::build_engine(&config).unwrap()
    }

    fn incompatible(found: &str) -> Result<(), Mismatch> {
        Err(Mismatch::Incompatible {
            found: found.to_owned(),
        })
    }

    #[test]
    fn the_version_table() {
        let host = "0.3.0";
        assert_eq!(verdict(Some("0.3.0"), host), Ok(()));
        assert_eq!(verdict(Some("0.3.1"), host), incompatible("0.3.1"));
        assert_eq!(verdict(Some("0.2.3"), host), incompatible("0.2.3"));
        assert_eq!(verdict(Some("0.4.0"), host), incompatible("0.4.0"));
        assert_eq!(verdict(Some("1.3.0"), host), incompatible("1.3.0"));
        assert_eq!(verdict(Some("0.3"), host), incompatible("0.3"));
        assert_eq!(
            verdict(Some("0.3.0-rc.1"), host),
            incompatible("0.3.0-rc.1")
        );
        assert_eq!(verdict(None, host), Err(Mismatch::NotAPlugin));

        assert_eq!(verdict(Some("0.3.0"), "0.3.2"), Ok(()));
        assert_eq!(verdict(Some("0.3.2"), "0.3.2"), Ok(()));
        assert_eq!(verdict(Some("0.3.3"), "0.3.2"), incompatible("0.3.3"));
    }

    #[test]
    fn the_host_version_is_accepted() {
        assert_eq!(verdict(Some(WORLD_VERSION), WORLD_VERSION), Ok(()));
        assert_eq!(supported("0.3.0"), "0.3.x");
    }

    fn exporting(name: &str) -> Component {
        Component::new(
            &engine(),
            format!(
                r#"(component
                    (instance $guest)
                    (export "{name}" (instance $guest))
                )"#
            ),
        )
        .unwrap()
    }

    #[test]
    fn an_older_world_is_rejected_with_a_rebuild_hint() {
        let engine = engine();
        let component = exporting("infrarust:plugin/guest@0.2.3");
        let err = check(&engine, &component, Path::new("old.wasm")).unwrap_err();
        let message = err.to_string();
        for needle in [
            "infrarust:plugin@0.2.3",
            "infrarust:plugin@0.3.x",
            "rebuild",
        ] {
            assert!(
                message.contains(needle),
                "{needle:?} missing from {message}"
            );
        }
    }

    #[test]
    fn a_component_without_the_guest_export_is_not_a_plugin() {
        let engine = engine();
        let component = exporting("example:other/guest@0.3.0");
        let err = check(&engine, &component, Path::new("other.wasm")).unwrap_err();
        assert!(matches!(err, WasmLoaderError::NotAPlugin { .. }), "{err:?}");
        assert!(
            err.to_string()
                .contains("not an Infrarust plugin component"),
            "{err}"
        );
    }

    #[test]
    fn the_current_world_passes() {
        let engine = engine();
        let component = exporting(&format!("infrarust:plugin/guest@{WORLD_VERSION}"));
        assert!(check(&engine, &component, Path::new("new.wasm")).is_ok());
    }
}

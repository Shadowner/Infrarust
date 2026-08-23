//! Every `.toml` shipped under `examples/` must keep parsing with the
//! production loaders: `examples/infrarust.toml` as a proxy config, files
//! under `examples/servers/` (and any other example) as server configs.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use infrarust_config::{ProxyConfig, ServerConfig, validate_server_config};

fn collect_tomls(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("examples dir must be readable") {
        let path = entry.expect("readable dir entry").path();
        if path.is_dir() {
            collect_tomls(&path, out);
        } else if path.extension().is_some_and(|e| e == "toml") {
            out.push(path);
        }
    }
}

#[test]
fn every_shipped_example_parses() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut files = Vec::new();
    collect_tomls(&examples, &mut files);

    let mut proxy_count = 0;
    let mut server_count = 0;
    let mut failures = Vec::new();

    for path in &files {
        let name = path.strip_prefix(&examples).unwrap_or(path).display();
        let content = std::fs::read_to_string(path).expect("readable example file");

        if path.file_name().is_some_and(|n| n == "infrarust.toml") {
            proxy_count += 1;
            if let Err(e) = toml::from_str::<ProxyConfig>(&content) {
                failures.push(format!("{name}: {e}"));
            }
        } else {
            server_count += 1;
            match toml::from_str::<ServerConfig>(&content) {
                Ok(config) => {
                    if let Err(e) = validate_server_config(&config) {
                        failures.push(format!("{name}: {e}"));
                    }
                }
                Err(e) => failures.push(format!("{name}: {e}")),
            }
        }
    }

    assert!(
        proxy_count >= 1 && server_count >= 1,
        "expected at least one proxy and one server example under {}",
        examples.display()
    );
    assert!(
        failures.is_empty(),
        "shipped example configs no longer parse:\n{}",
        failures.join("\n")
    );
}

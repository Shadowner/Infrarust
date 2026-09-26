# Contributing

Feel free to add or modify the source code. On GitHub the best way of doing this is by forking this repository, then cloning your fork with Git to your local system. After adding or modifying the source code, push it back to your fork and open a pull request in this repository.

## Tools Required

- Development
  - [Rust](https://www.rust-lang.org/tools/install) (latest stable version)
  - [Cargo](https://doc.rust-lang.org/cargo/) (comes with Rust)
  - [rustfmt](https://github.com/rust-lang/rustfmt) (for code formatting)
  - [clippy](https://github.com/rust-lang/rust-clippy) (for linting)
- Optional Tools
  - [Docker](https://www.docker.com/get-started/) (for containerization)
  - [rust-analyzer](https://rust-analyzer.github.io/) (recommended IDE plugin)

## Project Structure

Infrarust is a Cargo workspace. Each directory below is one crate unless noted.

```text
crates/
├── infrarust/                  # The proxy binary: CLI, config loading, telemetry, plugin bootstrap
├── infrarust-core/             # Proxy engine: pipeline, sessions, status, event bus, plugin manager
├── infrarust-api/              # The native plugin API (traits, events, types); `test-util` feature ships mocks
├── infrarust-plugin-common/    # Dependency-free types shared by the API and the WASM guest SDK
├── infrarust-plugin-sdk/       # Guest SDK for WASM plugins (infrarust:plugin@0.3)
├── infrarust-plugin-macros/    # Proc-macros used by the guest SDK (`#[plugin]`)
├── infrarust-plugin-wit/       # The frozen WIT contract shared by the host loader and the SDK
├── infrarust-loader-wasm/      # The wasmtime host that loads WASM plugins
├── infrarust_protocol/         # Minecraft protocol codec: packets, framing, compression, encryption
├── infrarust_config/           # Configuration types, validation, providers and migration
├── infrarust_server_manager/   # Managed backends: local process, Pterodactyl and Crafty providers
├── infrarust-transport/        # Listener, backend connector, PROXY protocol
└── infrarust-test-harness/     # In-process fake clients and backends for end-to-end tests
plugins/
├── infrarust-plugin-hello/     # Minimal example plugin
├── infrarust-plugin-auth/      # Offline-mode login and register, premium auto-login
├── infrarust-plugin-admin-api/ # HTTP REST administration API
├── infrarust-plugin-server-wake/ # Wakes managed backends on demand
└── infrarust-plugin-stats/     # Statistics plugin, built natively and as WASM
tools/
├── mc-bench/                   # Play-state load generator and mock backend for benchmarks
├── stress-test/                # Connection-flood stress tester
└── registry-extractor/         # Dumps configuration-phase registry data from a live server
templates/
└── plugin-template/            # `cargo generate` template for a new plugin
docs/
└── v2/                         # The documentation site (VitePress): guide, configuration, plugins
```

Integration tests live in each crate's `tests/` directory; the end-to-end suite that drives real Minecraft traffic through the proxy is in `crates/infrarust-test-harness/tests/`.

## Code Style

- Follow the official [Rust Style Guide](https://rust-lang.github.io/api-guidelines/)
- Use `cargo fmt --all` before committing to ensure consistent formatting
- Run `cargo clippy --workspace --all-targets --all-features` and keep it at zero warnings

Two conventions are enforced across the workspace:

- **No `unwrap` or `expect` in production code.** The workspace lints warn on `clippy::unwrap_used` and `clippy::expect_used`. Handle the error or return it; for a `std::sync::Mutex` or `RwLock`, use `crate::util::sync::{lock, read, write}` in `infrarust-core`, which recover from poisoning. Test modules start with `#![allow(clippy::unwrap_used, clippy::expect_used)]`, so tests may unwrap freely.
- **No comments in code.** Rust sources carry no `//`, `///` or `//!` comments: names and types are expected to explain the code, and anything that needs prose goes to `docs/v2`, next to the feature it documents. Public items of `infrarust-api` keep their doc comments since they are rendered by rustdoc for plugin authors.

## Commit Messages

When contributing to this project please follow the [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) specification.

Examples:

- `feat: add support for protocol version 1.19.4`
- `fix: handle compression threshold properly`
- `docs: update README with new configuration options`
- `test: add unit tests for packet handling`

> More example here <https://www.conventionalcommits.org/en/v1.0.0/#examples>

## Building and Testing

```bash
# Build the project
cargo build

# Run tests
cargo test

# Run with specific features
cargo run --bin infrarust -- --config-path custom_config.yaml --proxies-path proxies_path_foler
```

## Versioning

We follow [Semantic Versioning](https://semver.org/):

- MAJOR version for incompatible API changes
- MINOR version for new functionality in a backwards compatible manner
- PATCH version for backwards compatible bug fixes

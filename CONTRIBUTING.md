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

```C#
rust/
├── src/
│   ├── bin/           # Binary executables
│   ├── core/          # Core functionality
│   ├── network/       # Networking code
│   ├── protocol/      # Minecraft protocol implementation
│   └── proxy_modes/   # Different proxy mode implementations
├── tests/             # Integration tests // TODO
├── Cargo.toml         # Project dependencies and metadata
└── Cargo.lock         # Locked dependencies
```

> More detail will be added on the website documentation

## Code Style

- Follow the official [Rust Style Guide](https://rust-lang.github.io/api-guidelines/)
- Use `cargo fmt` before committing to ensure consistent formatting
- Run `cargo clippy` to catch common mistakes and improve code quality

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

# infrarust

The Infrarust binary: a high-performance Minecraft reverse proxy written in Rust.

Routes players to backend servers by the domain they connect with, with load
balancing, hot-reloaded configuration, status caching, a ban system, an
interactive console, and a plugin system (native and WebAssembly).

This crate wires the [`infrarust-core`](https://crates.io/crates/infrarust-core)
runtime, the bundled plugins, and the CLI into the shipped executable.

- Repository: <https://github.com/Shadowner/Infrarust>
- Documentation: <https://infrarust.dev>

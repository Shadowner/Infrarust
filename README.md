<div align="center" class="header-container">
  <div class="logo-wrapper">
    <img width="200" height="auto" src="docs/public/img/logo.svg" alt="Infrarust Logo" class="main-logo">
    <div class="logo-glow"></div>
  </div>
  
  <h1 class="title">Infrarust</h1>
  <h3 class="subtitle">High-Performance Minecraft Reverse Proxy in Rust</h3>
  
  <div class="badges-container">
    <a href="https://crates.io/crates/infrarust" class="badge-link">
      <img alt="Crates.io" src="https://img.shields.io/crates/v/infrarust?style=flat-square" />
    </a>
    <img alt="License" src="https://img.shields.io/badge/license-AGPL--3.0-blue?style=flat-square" />
  </div>
</div>

> [!WARNING]
> Infrarust is currently in active development. This project is a Rust implementation inspired by [Infrared](https://infrared.dev/), focusing on performance and enhanced features.

A blazing fast Minecraft reverse proxy that allows you to expose multiple Minecraft servers through a single port. It uses domain/subdomain-based routing to direct clients to specific Minecraft servers while providing advanced features for authentication and monitoring.

## Key Features

- [X] Efficient Reverse Proxy
  - [X] Wildcard Domain Support
  - [X] Multi-Domain Routing
  - [X] Direct IP Connection Support
- [X] Advanced Authentication Modes
  - [X] ClientOnly Mode with Mojang Authentication
  - [X] Passthrough Mode
  - [X] Offline Mode
- [X] Performance Optimizations
  - [X] Status Response Caching
  - [X] Connection Pooling
  - [ ] Proxy Protocol Support
- [X] Security Features
  - [X] Rate Limiting
  - [X] DDoS Protection
  - [X] IP Filtering

## Upcoming Features (#1)

- [ ] RESTful API for Dynamic Configuration
- [ ] Advanced Telemetry and Metrics
- [ ] Web Dashboard
- [ ] Hot Configuration Reload
- [ ] Plugin System
- [ ] Multi-version Support (BE/JE)

## Quick Start

### Prerequisites

- Rust 1.80+ and Cargo

### Installation

```bash
# From source
git clone https://github.com/shadowner/infrarust
cd infrarust
cargo build --release

# Or via cargo
cargo install infrarust
```

### Configuration

Create a `config.yaml` file:

```yaml
bind: "0.0.0.0:25565"
domains:
  - "*.minecraft.example.com"
```

And create your server configurations in the `proxies` directory:

```yaml
domains:
  - "hub.minecraft.example.com"
addresses:
  - "localhost:25566"
proxyMode: "passthrough"  # Options: passthrough, cllient-only, offline
```

## Documentation

- [Installation Guide](https://infrarust.dev/docs/installation)
- [Configuration Reference](https://infrarust.dev/docs/configuration)
- [Proxy Modes](https://infrarust.dev/docs/proxy-modes)
- [API Documentation](https://infrarust.dev/docs/api)
- [Performance Tuning](https://infrarust.dev/docs/performance)

## Performance

Infrarust is built in Rust with a focus on performance and reliability:

- Minimal memory footprint
- Low CPU utilization
- Efficient async I/O handling
- Zero-copy packet forwarding when possible

> [!NOTE]
> This project was initiated as a learning experience in advanced Rust programming, with continuous improvements and optimizations expected as development progresses.

## Contributing

Contributions are welcome! Check out our [Contributing Guidelines](CONTRIBUTING.md) to get started.

Feel free to join our [Discord](https://discord.gg/uzs5nZsWaB) if you have any question !

## Similar Projects

- [Infrared](https://github.com/haveachin/infrared) - The original inspiration, written in Go
- [MCRouter](https://github.com/itzg/mc-router)
- [Velocity](https://github.com/PaperMC/Velocity)

## License

Infrarust is licensed under the GNU Affero General Public License v3.0 - see the [LICENSE](LICENSE) file for details.

<br />
<p align="center">
  <img height="60" src="docs/public/img/agplv3_logo.svg"/>
</p>

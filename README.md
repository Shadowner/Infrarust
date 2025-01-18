<div align="center" class="header-container">
  <div class="logo-wrapper">
    <img width="200" height="auto" src="docs/public/img/logo.svg" alt="Infrarust Logo" class="main-logo">
    <div class="logo-glow"></div>
  </div>
  
  <h1 class="title">Infrarust</h1>
  <h3 class="subtitle">High-Performance Minecraft Reverse Proxy in Rust</h3>
  
  <div class="badges-container">
    <a href="https://github.com/shadowner/infrarust/actions" class="badge-link">
      <img alt="CI" src="https://github.com/shadowner/infrarust/actions/workflows/ci.yml/badge.svg" />
    </a>
    <a href="https://crates.io/crates/infrarust" class="badge-link">
      <img alt="Crates.io" src="https://img.shields.io/crates/v/infrarust?style=flat-square" />
    </a>
    <img alt="License" src="https://img.shields.io/badge/license-AGPL--3.0-blue?style=flat-square" />
  </div>
</div>

<style>
  .header-container {
    padding: 3rem 1.5rem;
    background: linear-gradient(180deg, rgba(230,126,34,0.1) 0%, rgba(0,0,0,0) 100%);
    border-radius: 16px;
    margin-bottom: 2rem;
  }

  .logo-wrapper {
    position: relative;
    width: 200px;
    height: 200px;
    margin: 0 auto;
  }

  .main-logo {
    position: relative;
    z-index: 2;
    transition: transform 0.3s ease;
  }

  .main-logo:hover {
    transform: scale(1.05);
  }

  .logo-glow {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: 180px;
    height: 180px;
    background: rgba(230,126,34,0.15);
    filter: blur(20px);
    border-radius: 50%;
    z-index: 1;
  }

  .title {
    font-size: 3.5rem;
    font-weight: 700;
    margin: 1.5rem 0 0.5rem;
    background: linear-gradient(135deg, #E67E22 0%, #D35400 100%);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    animation: titleGlow 4s ease-in-out infinite;
  }

  .subtitle {
    font-size: 1.5rem;
    font-weight: 500;
    color: #666;
    margin: 0 0 1.5rem;
    max-width: 600px;
    margin: 0 auto 2rem;
  }

  .badges-container {
    display: flex;
    gap: 0.75rem;
    justify-content: center;
    align-items: center;
  }

  .badge-link {
    transition: opacity 0.2s ease;
  }

  .badge-link:hover {
    opacity: 0.8;
  }

  @keyframes titleGlow {
    0%, 100% {
      filter: drop-shadow(0 0 2px rgba(230,126,34,0.3));
    }
    50% {
      filter: drop-shadow(0 0 5px rgba(230,126,34,0.5));
    }
  }
</style>
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

- Rust 1.75+ and Cargo

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
proxyMode: "passthrough"  # Options: passthrough, clientOnly, offline
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

## Similar Projects

- [Infrared](https://github.com/haveachin/infrared) - The original inspiration, written in Go
- [MCRouter](https://github.com/itzg/mc-router)
- [Waterfall](https://github.com/PaperMC/Velocity)

## License

Infrarust is licensed under the GNU Affero General Public License v3.0 - see the [LICENSE](LICENSE) file for details.

<br />
<p align="center">
  <img height="60" src="assets/agplv3_logo.svg"/>
</p>

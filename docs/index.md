---
layout: home

hero:
  name: "Infrarust"
  text: "Universal Minecraft Reverse Proxy"
  tagline: One proxy to rule them all
  image:
    src: /img/logo.svg
    alt: Infrarust Logo
  actions:
    - theme: brand
      text: Quick Start →
      link: /quickstart/
    - theme: alt
      text: Configuration
      link: /quickstart/configuration
    - theme: alt
      text: View on GitHub
      link: https://github.com/shadowner/infrarust

features:
  - icon: 🌈
    title: Universal Compatibility
    details: Works with any Minecraft version (1.7.10 to 1.20.4) and any mod loader (Forge, Fabric, Quilt, etc.)

  - icon: 🚀
    title: Native Performance
    details: Built in Rust for maximum efficiency, with minimal overhead and optimized resource usage

  - icon: 🔒
    title: Enhanced Security
    details: Protect your network with built-in DDoS protection, ban system, and filtering capabilities

  - icon: 🐋
    title: Docker Integration
    details: Automatically detect and proxy Minecraft containers with real-time configuration

  - icon: 🖥️
    title: Powerful CLI
    details: Manage your server with an intuitive command-line interface for player and ban management

  - icon: 🎮
    title: Modded & Plugin Support
    details: Seamlessly handle modded servers and clients without any special configuration
---

::: tip CURRENT VERSION
<span class="version-tag">v1.2.0</span> - CLI, Docker & Ban Update
:::

## 🎯 Why Infrarust?

Infrarust is a modern Minecraft reverse proxy that truly works with everything:

### Universal Compatibility - Passthrough Mode

- ✅ All Minecraft versions (1.7.10 to latest)
- ✅ Every mod loader (Forge, Fabric, Quilt)
- ✅ Vanilla and modded servers
- ✅ Premium and offline modes
- ✅ No special configuration needed

### Technical Stack

- 🚀 Written in Rust for native performance
- 🛡️ Built-in protection against attacks
- 🚫 Advanced ban system with IP, username, and UUID filtering
- 🐋 Seamless Docker container integration
- 📝 Simple YAML configuration
- 🔄 Hot-reload support
- 📊 Comprehensive monitoring

## 🚀 Quick Start

```bash
# Download and run
curl -LO https://github.com/Shadowner/Infrarust/releases/latest/download/infrarust
chmod +x infrarust
./infrarust

# Or install via cargo
cargo install infrarust
```

## 🔮 Key Features

| Feature | Description |
|---------|-------------|
| **Multiple Proxy Modes** | Support passthrough, client-only, offline, and server-only modes |
| **Ban System** | Ban players by IP, username, or UUID with temporary or permanent bans |
| **Docker Integration** | Automatically detect and proxy containers with label-based configuration |
| **Command-Line Interface** | Manage players, view connections, and handle bans in real-time |
| **Configuration Hot-Reload** | Change configuration without restarting the proxy |
| **DDoS Protection** | Built-in rate limiting and connection filtering |

## 💡 Perfect For

- **Local Hosting**: For those who don't want to expose all their ports
- **Network Owners**: Handle multiple server types from one proxy
- **Modpack Creators**: Route different modpack versions seamlessly
- **Server Admins**: Manage vanilla and modded servers together
- **Community Hosts**: Support any client version or mod loader
- **Container Deployments**: Seamlessly integrate with Docker environments

## 📊 Real-World Performance

| Metric | Value |
|--------|--------|
| Memory Usage | < 20MB base |
| CPU Usage | Minimal |
| Latency Overhead | < 1ms |
| Connection Handling | 10,000+ concurrent |

## 📚 Documentation Highlights

- [Complete Configuration Reference](/quickstart/configuration)
- [Docker Integration Guide](/features/docker)
- [Ban System Documentation](/features/ban-system)
- [CLI Command Reference](/features/cli/)

## 🗺️ Roadmap Highlights

| Feature | Status |
|---------|--------|
| Web Dashboard | 💡 Planned |
| Plugin API | 💭 Proposed |
| Version Translation | 💭 Proposed |
| Multi-Proxy Clustering | 💭 Proposed |

## 🤝 Community

Join our growing community:

- 📖 [Documentation](/quickstart/)
- 💬 [Discord](https://discord.gg/sqbJhZVSgG)
- 🐛 [GitHub Issues](https://github.com/shadowner/infrarust/issues)

<script>
// TODO: Look for another way with vitepress
if (typeof window !== 'undefined' && navigator.language.startsWith('fr') && !localStorage.getItem('redirected')) {
  window.location.replace('/fr' + window.location.pathname);
  localStorage.setItem('redirected', 'true');
}
</script>

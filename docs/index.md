---
layout: home

hero:
  name: "Infrarust"
  text: "Universal Minecraft Reverse Proxy"
  tagline: One proxy for all Minecraft versions and mod loaders
  image:
    src: /img/logo.svg
    alt: Infrarust Logo
  actions:
    - theme: brand
      text: Quick Start →
      link: /quickstart/
    - theme: alt
      text: Documentation
      link: /proxy/
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
    details: Protect your network with built-in DDoS protection and filtering systems
  
  - icon: 🎮
    title: Modded Support
    details: Seamlessly handle modded servers and clients without any special configuration
---

::: tip CURRENT VERSION
<span class="version-tag">v1.2.0</span> - Production Ready
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

## 💡 Perfect For

- **Local Hosting**: For those who doesn't want to expose all their ports
- **Network Owners**: Handle multiple server types from one proxy
- **Modpack Creators**: Route different modpack versions seamlessly
- **Server Admins**: Manage vanilla and modded servers together
- **Community Hosts**: Support any client version or mod loader

## 📊 Real-World Performance

| Metric | Value |
|--------|--------|
| Memory Usage | < 20MB base |
| CPU Usage | Minimal |
| Latency Overhead | < 1ms |
| Connection Handling | 10,000+ concurrent |

## 🗺️ Roadmap Highlights

| Feature | Status |
|---------|--------|
| Web Dashboard | 💡 Planned |
| Plugin API | 💭 Proposed |
| Version Translation | 💭 Proposed |
| Multi-Proxy Clustering | 💭 Proposed |

## 🤝 Community

Join our growing community:

- 📖 [Documentation](/docs/)
- 💬 [Discord](https://discord.gg/uzs5nZsWaB)
- 🐛 [GitHub Issues](https://github.com/shadowner/infrarust/issues)

<script>
// TODO: Look for another way with vitepress
if (typeof window !== 'undefined' && navigator.language.startsWith('fr') && !localStorage.getItem('redirected')) {
  window.location.replace('/fr' + window.location.pathname);
  localStorage.setItem('redirected', 'true');
}
</script>

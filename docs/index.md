---
# https://vitepress.dev/reference/default-theme-home-page
layout: home

hero:
  name: "Infrarust"
  text: "High-Performance Minecraft Reverse Proxy"
  tagline: Power your Minecraft servers with Rust's performance
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
  - icon: 🚀
    title: Performance
    details: Built in Rust for maximum efficiency, with minimal memory footprint and optimized CPU usage.
  
  - icon: 🔒
    title: Enhanced Security
    details: Built-in dynamic filtering system
  
  - icon: 🌐
    title: Smart Routing
    details: Wildcard domains and multi-domain routing support for maximum flexibility.
  
  - icon: 🔄
    title: Multiple Modes
    details: Several proxy modes (ClientOnly, Passthrough, Offline) to adapt to your needs.
  
---

::: tip CURRENT VERSION
<span class="version-tag">v1.0.1</span> - Under active development
<br>
:::

## 🎯 Why Infrarust?

Infrarust was born from the desire to create a high-performance Minecraft proxy by leveraging the power and security of Rust. Inspired by [Infrared](https://infrared.dev/), we redesigned the architecture to offer:

- **Maximum Performance**: Written in Rust for native performance
- **Enhanced Security**: Built-in protection against attacks
- **Ease of Use**: Intuitive YAML configuration
- **Total Flexibility**: Adapted to all configurations

## 🚀 Quick Installation

```bash
# Install from source
git clone https://github.com/shadowner/infrarust
cd infrarust
cargo build --release

# Via cargo
cargo install infrarust

# Or via binaries
https://github.com/Shadowner/Infrarust/releases/
```

## 🛣️ Roadmap

| Feature | Status |
|---------|--------|
| REST API | 💡 Proposed |
| Web Dashboard | 💡 Proposed |
| Multi-Version Support | 💡 Proposed |
| Desktop Version | 💡 Proposed |
| Plugin System | 💡 Proposed |

## 🤝 Join the Community

Infrarust is a growing open source project. We welcome all contributions!

- 📖 [Contribution Guide](/contributing)
- 💬 [Discord](https://discord.gg/uzs5nZsWaB)
- 🐛 [Report a Bug](https://github.com/shadowner/infrarust/issues)

<script>

// TODO: Look for another way with vitepress
if (typeof window !== 'undefined' && navigator.language.startsWith('fr') && !localStorage.getItem('redirected')) {
  window.location.replace('/fr' + window.location.pathname);
  localStorage.setItem('redirected', 'true');
}
</script>

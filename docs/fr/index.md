---
layout: home

hero:
  name: "Infrarust"
  text: "Proxy Minecraft de Haute Performance"
  tagline: Propulsez vos serveurs Minecraft avec la puissance de Rust
  image:
    src: /img/logo.svg
    alt: Infrarust Logo
  actions:
    - theme: brand
      text: Démarrage Rapide →
      link: /fr/quickstart
    - theme: alt
      text: Documentation
      link: /fr/proxy/
    - theme: alt
      text: Voir sur GitHub
      link: https://github.com/shadowner/infrarust

features:
  - icon: 🚀
    title: Performant
    details: Conçu en Rust pour une efficacité maximale, avec une empreinte mémoire minimale et une utilisation optimisée du CPU.
  
  - icon: 🔒
    title: Sécurité Renforcée
    details: Système de filtres dynamique intégré
  
  - icon: 🌐
    title: Routage Intelligent
    details: Support des domaines wildcards et routage multi-domaines pour une flexibilité maximale.
  
  - icon: 🔄
    title: Modes Multiples
    details: Plusieurs modes d'authentification (ClientOnly, Passthrough, Offline) pour s'adapter à vos besoins.
  
---

::: tip VERSION ACTUELLE
<span class="version-tag">v1.0.1</span> - En développement actif
<br>
:::

## 🎯 Pourquoi Infrarust ?

Infrarust est né de la volonté de créer un proxy Minecraft haute performance en tirant parti de la puissance et de la sécurité de Rust. Inspiré par [Infrared](https://infrared.dev/), nous avons repensé l'architecture pour offrir :

- **Performance maximale** : Écrit en Rust pour des performances natives
- **Sécurité renforcée** : Protection intégrée contre les attaques
- **Simplicité d'utilisation** : Configuration intuitive en YAML
- **Flexibilité totale** : Adapté à toutes les configurations

## 🚀 Installation Rapide

```bash
# Installation depuis les sources
git clone https://github.com/shadowner/infrarust
cd infrarust
cargo build --release

# via cargo
cargo install infrarust

# Ou par les binaires précompilé
https://github.com/Shadowner/Infrarust/releases/
```

## 🛣️ Feuille de Route

| Fonctionnalité | Statut |
|----------------|--------|
| API REST | 💡 Proposé |
| Dashboard Web | 💡 Proposé |
| Support Multi-Version | 💡 Proposé |
| Version Desktop | 💡 Proposé |
| Système de Plugins | 💡 Proposé |

## 🤝 Rejoignez la Communauté

Infrarust est un projet open source en pleine croissance. Nous accueillons toutes les contributions !

- 📖 [Guide de Contribution](/contributing/)
- 💬 [Discord](https://discord.gg/uzs5nZsWaB) #TODO
- 🐛 [Signaler un Bug](https://github.com/shadowner/infrarust/issues)

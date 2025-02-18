---
layout: home

hero:
  name: "Infrarust"
  text: "Proxy Inverse Minecraft Universel"
  tagline: Un proxy pour toutes les versions Minecraft et tous les mod loaders
  image:
    src: /img/logo.svg
    alt: Logo Infrarust
  actions:
    - theme: brand
      text: Démarrage Rapide →
      link: /fr/quickstart/
    - theme: alt
      text: Documentation
      link: /fr/proxy/
    - theme: alt
      text: Voir sur GitHub
      link: https://github.com/shadowner/infrarust

features:
  - icon: 🌈
    title: Compatibilité Universelle
    details: Fonctionne avec toute version Minecraft (1.7.10 à 1.20.4) et tout mod loader (Forge, Fabric, Quilt, etc.)
  
  - icon: 🚀
    title: Performance Native
    details: Développé en Rust pour une efficacité maximale, avec une surcharge minimale et une utilisation optimisée des ressources
  
  - icon: 🔒
    title: Sécurité Renforcée
    details: Protégez votre réseau avec des systèmes intégrés de protection DDoS et de filtrage
  
  - icon: 🎮
    title: Support des Mods
    details: Gérez les serveurs et clients moddés sans configuration particulière

---

::: tip VERSION ACTUELLE
<span class="version-tag">v1.2.0</span> - Prêt pour la Production
:::

## 🎯 Pourquoi Infrarust ?

Infrarust est un proxy inverse Minecraft moderne qui fonctionne réellement avec tout :

### Compatibilité Universelle - Mode Passthrough

- ✅ Toutes les versions Minecraft (1.7.10 à la dernière)
- ✅ Tous les mod loaders (Forge, Fabric, Quilt)
- ✅ Serveurs vanilla et moddés
- ✅ Modes premium et offline
- ✅ Aucune configuration spéciale requise

### Stack Technique

- 🚀 Écrit en Rust pour des performances natives
- 🛡️ Protection intégrée contre les attaques
- 📝 Configuration YAML simple
- 🔄 Support du rechargement à chaud
- 📊 Surveillance complète

## 🚀 Démarrage Rapide

```bash
# Télécharger et exécuter
curl -LO https://github.com/Shadowner/Infrarust/releases/latest/download/infrarust
chmod +x infrarust
./infrarust

# Ou installer via cargo
cargo install infrarust
```

## 💡 Parfait Pour

- **Hébergement Local** : Pour ceux qui ne veulent pas exposer tous leurs ports
- **Propriétaires de Réseaux** : Gérez plusieurs types de serveurs depuis un seul proxy
- **Créateurs de Modpacks** : Routez différentes versions de modpacks sans effort
- **Administrateurs de Serveurs** : Gérez ensemble serveurs vanilla et moddés
- **Hébergeurs Communautaires** : Supportez n'importe quelle version client ou mod loader

## 📊 Performances en Conditions Réelles

| Métrique | Valeur |
|----------|--------|
| Utilisation Mémoire | < 20MB base |
| Utilisation CPU | Minimale |
| Surcharge Latence | < 1ms |
| Gestion Connexions | 10,000+ simultanées |

## 🗺️ Points Clés de la Feuille de Route

| Fonctionnalité | Statut |
|----------------|--------|
| Tableau de Bord Web | 💡 Planifié |
| API Plugin | 💭 Proposé |
| Traduction de Version | 💭 Proposé |
| Clustering Multi-Proxy | 💭 Proposé |

## 🤝 Communauté

Rejoignez notre communauté grandissante :

- 📖 [Documentation](/fr/docs/)
- 💬 [Discord](https://discord.gg/uzs5nZsWaB)
- 🐛 [GitHub Issues](https://github.com/shadowner/infrarust/issues)

<script>
// TODO: Chercher une autre façon avec vitepress
if (typeof window !== 'undefined' && !navigator.language.startsWith('fr') && !localStorage.getItem('redirected')) {
  window.location.replace(window.location.pathname.replace('/fr/', '/'));
  localStorage.setItem('redirected', 'true');
}
</script>
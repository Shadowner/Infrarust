---
layout: home

hero:
  name: "Infrarust"
  text: "Reverse Proxy Universel pour Minecraft"
  tagline: Un seul proxy pour tous les gouverner
  image:
    src: /img/logo.svg
    alt: Logo Infrarust
  actions:
    - theme: brand
      text: Démarrage Rapide →
      link: /quickstart/
    - theme: alt
      text: Configuration
      link: /quickstart/configuration
    - theme: alt
      text: Voir sur GitHub
      link: https://github.com/shadowner/infrarust

features:
  - icon: 🌈
    title: Compatibilité Universelle
    details: Fonctionne avec toutes les versions de Minecraft (1.8 à la dernière) et tous les mod loaders (Forge, Fabric, Quilt, etc.)

  - icon: 🚀
    title: Performance Native
    details: Construit en Rust pour une efficacité maximale, avec une surcharge minimale et une utilisation optimisée des ressources

  - icon: 🔒
    title: Sécurité Renforcée
    details: Protégez votre réseau avec une protection DDoS intégrée, un système de bannissement et des capacités de filtrage

  - icon: 🐋
    title: Intégration Docker
    details: Détection et proxy automatiques des conteneurs Minecraft avec configuration en temps réel

  - icon: 🖥️
    title: CLI Puissante
    details: Gérez votre serveur avec une interface en ligne de commande intuitive pour la gestion des joueurs et des bannissements

  - icon: 🎮
    title: Support des Mods
    details: Gérez facilement les serveurs et clients moddés sans configuration spéciale
---

::: tip VERSION ACTUELLE
<span class="version-tag">v1.5.0</span> - Crafty integration Update update
:::

## 🎯 Pourquoi Infrarust ?

Infrarust est un proxy inverse moderne pour Minecraft qui fonctionne vraiment avec tout :

### Compatibilité Universelle - Mode Passthrough

- ✅ Toutes les versions de Minecraft (1.8 à la plus récente)
- ✅ Tous les mod loaders (Forge, Fabric, Quilt)
- ✅ Serveurs vanilla et moddés
- ✅ Modes premium et offline
- ✅ Aucune configuration spéciale nécessaire

### Stack Technique

- 🚀 Écrit en Rust pour des performances natives
- 🛡️ Protection intégrée contre les attaques
- 🚫 Système de bannissement avancé avec filtrage par IP, nom d'utilisateur et UUID
- 🐋 Intégration transparente des conteneurs Docker
- 📝 Configuration YAML simple
- 🔄 Support du rechargement à chaud
- 📊 Surveillance complète

## 🚀 Démarrage Rapide

```bash
# Télécharger et exécuter
curl -LO https://github.com/Shadowner/Infrarust/releases/
chmod +x infrarust
./infrarust

# Ou installer via cargo
cargo install infrarust
```

## 🔮 Fonctionnalités Clés

| Fonctionnalité | Description |
|---------|-------------|
| **Modes de Proxy Multiples** | Support des modes passthrough, client-only, offline et server-only |
| **Système de Bannissement** | Bannissement des joueurs par IP, nom d'utilisateur ou UUID avec bans temporaires ou permanents |
| **Intégration Docker** | Détection et proxy automatiques des conteneurs avec configuration basée sur les labels |
| **Interface en Ligne de Commande** | Gestion des joueurs, visualisation des connexions et gestion des bannissements en temps réel |
| **Rechargement de Configuration à Chaud** | Modification de la configuration sans redémarrer le proxy |
| **Protection DDoS** | Limitation de débit et filtrage de connexion intégrés |

## 💡 Parfait Pour

- **Hébergement Local** : Pour ceux qui ne veulent pas exposer tous leurs ports
- **Propriétaires de Réseaux** : Gérez plusieurs types de serveurs depuis un seul proxy
- **Créateurs de Modpacks** : Routez différentes versions de modpacks sans problème
- **Administrateurs de Serveurs** : Gérez ensemble des serveurs vanilla et moddés
- **Hébergeurs Communautaires** : Supportez n'importe quelle version cliente ou mod loader
- **Déploiements de Conteneurs** : Intégration transparente avec les environnements Docker

## 📊 Performance en Conditions Réelles

| Métrique | Valeur |
|--------|--------|
| Utilisation Mémoire | < 20MB base |
| Utilisation CPU | Minimale |
| Surcharge de Latence | < 1ms |
| Gestion de Connexions | 10 000+ simultanées |

## 📚 Points Forts de la Documentation

- [Référence Complète de Configuration](/fr/quickstart/configuration)
- [Guide d'Intégration Docker](/fr/features/docker)
- [Documentation du Système de Bannissement](/fr/features/ban-system)
- [Référence des Commandes CLI](/fr/features/cli/)

## 🗺️ Points Forts de la Feuille de Route

| Fonctionnalité | Statut |
|---------|--------|
| Tableau de Bord Web | 💡 Planifié |
| API de Plugin | 💭 Proposé |
| Traduction de Version | 💭 Proposé |
| Clustering Multi-Proxy | 💭 Proposé |

## 🤝 Communauté

Rejoignez notre communauté grandissante :

- 📖 [Documentation](/fr/quickstart/)
- 💬 [Discord](https://discord.gg/sqbJhZVSgG)
- 🐛 [Problèmes GitHub](https://github.com/shadowner/infrarust/issues)

<script>
// TODO: Chercher une autre méthode avec vitepress
if (typeof window !== 'undefined' && !navigator.language.startsWith('fr') && !localStorage.getItem('redirected')) {
  window.location.replace('/' + window.location.pathname);
  localStorage.setItem('redirected', 'true');
}
</script>

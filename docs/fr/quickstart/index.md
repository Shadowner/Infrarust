# Guide de Démarrage Rapide

Ce guide vous aidera à installer et configurer Infrarust pour votre première utilisation.

## Prérequis

Avant de commencer, assurez-vous d'avoir :

> Ces prérequis s'applique seulement si vous ne téléchargez pas la version Précompilés

- Rust 1.80 ou supérieur
- Cargo (gestionnaire de paquets Rust)
- Un serveur Minecraft existant
- Un domaine (optionnel, pour le routage basé sur les domaines)

## Installation

### Méthode 1 : Binaires Précompilés

Téléchargez la dernière version depuis la [page des releases](https://github.com/shadowner/infrarust/releases).

### Méthode 2 : Via Cargo (Recommandée)

```bash
cargo install infrarust
```

### Méthode 2 : Depuis les Sources

```bash
# Cloner le dépôt
git clone https://github.com/shadowner/infrarust
cd infrarust

# Compiler le projet
cargo build --release

# L'exécutable se trouve dans target/release/infrarust
```

## Configuration Rapide

1. Créez un fichier `config.yaml` dans votre répertoire de travail :

```yaml
# Configuration minimale
bind: "0.0.0.0:25565"  # Adresse d'écoute
keepAliveTimeout: 30s
filters:
  rateLimiter:
    requestLimit: 10
    windowLength: 1s
keepalive_timeout: 30s  # Timeout de keepalive
```

2. Créez un dossier `proxies` et ajoutez un fichier de configuration pour votre serveur :

```yaml
# proxies/my-server.yml
domains:
  - "hub.minecraft.example.com"  # Domaine spécifique
addresses:
  - "localhost:25566"  # Adresse du serveur Minecraft
proxyMode: "passthrough"  # Mode de proxy
```

## Structure des Dossiers

```
infrarust/
├── config.yaml          # Configuration principale
├── proxies/            # Configurations des serveurs
│   ├── hub.yml
│   ├── survival.yml
│   └── creative.yml
└── logs/               # Journaux (créé automatiquement) //TODO: Not implemented yet
```

## Premiers Pas

### 1. Démarrer Infrarust

```bash
# Si installé via cargo
infrarust --config-path "./custom_config_path/config.yaml" --proxies-path "./custom_proxies_path/" 

# Si compilé depuis les sources
./target/release/infrarust --config-path "./custom_config_path/config.yaml" --proxies-path "./custom_proxies_path/" 
```

:::note
:::note
Les arguments --config-path et --proxies-path sont nécessaires uniquement si l'exécutable n'est pas dans le même répertoire que la structure de dossiers présentée ci-dessus
:::

### 2. Vérifier le Fonctionnement

1. Lancez votre client Minecraft
2. Connectez-vous à votre domaine configuré
3. Vérifiez les logs pour confirmer la connexion

## Modes de Proxy Disponibles

Infrarust propose plusieurs modes de proxy pour différents cas d'utilisation :

| Mode | Description | Cas d'Utilisation |
|------|-------------|-------------------|
| `passthrough` | Transmission directe | Pas de fonction de plugin, juste un proxy |
| `client_only` | Auth côté client | Serveurs en `online_mode=false`, mais client prenium |
| `offline` | Sans authentification | Serveurs `online_mode=false` et client cracké |

> D'autres modes sont en cours de développement

## Configuration de Base

### Protection DDoS Simple

```yaml
# Dans config.yaml
filters:
  rateLimiter:
    requestLimit: 10
    windowLength: 1s
```

### Cache de Status

```yaml
# Dans config.yaml
statusCache:
  enabled: true
  ttl: 30s
```

## Prochaines Étapes

Une fois la configuration de base terminée, vous pouvez :

1. [Configurer les différents modes de proxy](/proxy/modes)
2. [Optimiser les performances](/proxy/performance)
3. [Mettre en place la sécurité](/proxy/security)
4. [Configurer le monitoring](/deployment/monitoring)

## Résolution des Problèmes Courants

### Le proxy ne démarre pas

- Vérifiez que le port n'est pas déjà utilisé
- Assurez-vous d'avoir les permissions nécessaires
- Vérifiez la syntaxe du fichier de configuration

### Les clients ne peuvent pas se connecter

- Vérifiez la configuration des domaines
- Assurez-vous que les serveurs de destination sont accessibles
- Vérifiez les logs pour des erreurs spécifiques
- Vérifiez que le mode est compatible avec votre serveur

### Problèmes de Performance

- Activez le cache de status
- Vérifiez la configuration du rate limiter
- Assurez-vous que votre serveur a assez de ressources

## Besoin d'Aide ?

- 📖 Consultez la [documentation complète](/guide/)
- 🐛 Signalez un bug sur [GitHub](https://github.com/shadowner/infrarust/issues)
- 💬 Rejoignez notre [Discord](https://discord.gg/uzs5nZsWaB) ``// TODO``
 
::: tip
Pensez à consulter régulièrement la documentation car Infrarust est en développement actif et de nouvelles fonctionnalités sont ajoutées régulièrement.
:::
:::
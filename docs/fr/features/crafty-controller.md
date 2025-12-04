# Intégration Crafty Controller

Infrarust inclut une intégration avec Crafty Controller, un panneau de gestion de serveurs Minecraft populaire basé sur le web. Cette intégration permet à Infrarust de surveiller l'état des serveurs et de les démarrer automatiquement lorsque des joueurs se connectent.

## Aperçu

Le fournisseur Crafty Controller dans Infrarust :
- Surveille l'état du serveur en temps réel (En cours d'exécution, Arrêté, Planté)
- Démarre automatiquement les serveurs lorsque des joueurs tentent de se connecter
- Prend en charge le contrôle à distance du serveur (démarrer, arrêter, redémarrer)
- Utilise l'API REST de Crafty Controller pour la communication

## Configuration

### Configuration du gestionnaire

Pour activer l'intégration Crafty Controller, ajoutez ce qui suit à votre `config.yaml` :

```yaml
managers_config:
  crafty:
    enabled: true
    api_key: "votre_cle_api_crafty"
    base_url: "https://crafty.example.com"
```

### Options de configuration

| Option | Description | Requis |
|--------|-------------|--------|
| `enabled` | Activer l'intégration Crafty Controller | Oui |
| `api_key` | Clé API pour l'authentification Bearer token | Oui |
| `base_url` | URL de base de votre instance Crafty Controller | Oui |

## Configuration du serveur

Pour configurer un proxy utilisant Crafty Controller pour la gestion du serveur, ajoutez la section `server_manager` à votre fichier de configuration proxy :

```yaml
domains:
  - "mc.example.com"
addresses:
  - "127.0.0.1:25565"
proxyMode: "passthrough"

server_manager:
  provider_name: Crafty
  server_id: "votre-uuid-serveur"
  empty_shutdown_time: 30

motds:
  online:
    text: "Serveur en ligne"
  offline:
    text: "Serveur hors ligne - La connexion démarrera le serveur"
```

### Options de configuration du serveur

| Option | Description | Requis |
|--------|-------------|--------|
| `provider_name` | Doit être `Crafty` pour Crafty Controller | Oui |
| `server_id` | L'UUID du serveur dans Crafty Controller | Oui |

## Exemple complet

### Configuration principale (`config.yaml`)

```yaml
bind: "0.0.0.0:25565"

file_provider:
  proxies_path:
    - "./proxies"
  watch: true

managers_config:
  crafty:
    enabled: true
    api_key: "votre_cle_api_crafty"
    base_url: "https://crafty.example.com"
```

### Configuration du proxy (`proxies/survival.yaml`)

```yaml
domains:
  - "survival.example.com"
addresses:
  - "127.0.0.1:25565"
proxyMode: "passthrough"

server_manager:
  provider_name: Crafty
  server_id: "550e8400-e29b-41d4-a716-446655440000"
  empty_shutdown_time: 300  # Arrêt après 5 minutes sans joueurs

motds:
  online:
    version_name: "Serveur Survie"
    text: "Bienvenue en Survie !"
  offline:
    version_name: "Démarrage du serveur..."
    text: "Le serveur est hors ligne. Rejoignez pour le démarrer !"
```

## États du serveur

L'intégration Crafty Controller reconnaît les états de serveur suivants :

| État | Description |
|------|-------------|
| `Running` | Le serveur est en ligne et accepte les connexions |
| `Stopped` | Le serveur est hors ligne |
| `Crashed` | Le serveur a planté et nécessite une attention |

## Fonction d'arrêt automatique

Lorsque `empty_shutdown_time` est configuré, Infrarust arrêtera automatiquement le serveur après le nombre de secondes spécifié lorsqu'aucun joueur n'est connecté. Cela permet d'économiser des ressources lorsque les serveurs ne sont pas utilisés.

```yaml
server_manager:
  provider_name: Pterodactyl
  server_id: "de0d8f2d"
  empty_shutdown_time: 300  # Arrêt après 5 minutes sans joueurs
```

## Capacités de l'API

L'intégration prend en charge les opérations suivantes via l'API de Crafty Controller :

- **Obtenir l'état du serveur** : Vérifier si le serveur est en cours d'exécution, arrêté ou planté
- **Démarrer le serveur** : Démarrer un serveur hors ligne
- **Arrêter le serveur** : Arrêter proprement un serveur en cours d'exécution
- **Redémarrer le serveur** : Redémarrer un serveur en cours d'exécution


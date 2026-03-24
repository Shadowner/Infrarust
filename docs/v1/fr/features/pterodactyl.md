# Intégration Pterodactyl

Infrarust inclut une intégration avec Pterodactyl, un panneau de gestion de serveurs de jeux open-source populaire. Cette intégration permet à Infrarust de surveiller l'état des serveurs et de les démarrer automatiquement lorsque des joueurs se connectent.

## Aperçu

Le fournisseur Pterodactyl dans Infrarust :
- Surveille l'état du serveur en temps réel (Démarrage, En cours d'exécution, Arrêt, Arrêté, Planté)
- Démarre automatiquement les serveurs lorsque des joueurs tentent de se connecter
- Prend en charge le contrôle à distance du serveur (démarrer, arrêter, redémarrer)
- Arrête automatiquement les serveurs vides après un délai configurable
- Utilise l'API Client de Pterodactyl pour la communication


## Configuration

### Configuration du gestionnaire

Pour activer l'intégration Pterodactyl, ajoutez ce qui suit à votre `config.yaml` :

```yaml
managers_config:
  pterodactyl:
    enabled: true
    api_key: "votre_cle_api_pterodactyl" # Ce doit être une clé qui commence par "ptlc_" c'est une clé client
    base_url: "https://panel.example.com"
```

### Options de configuration

| Option | Description | Requis |
|--------|-------------|--------|
| `enabled` | Activer l'intégration Pterodactyl | Oui |
| `api_key` | Clé API Client du panneau Pterodactyl | Oui |
| `base_url` | URL de base de votre panneau Pterodactyl | Oui |

## Configuration du serveur

Pour configurer un proxy utilisant Pterodactyl pour la gestion du serveur, ajoutez la section `server_manager` à votre fichier de configuration proxy :

```yaml
domains:
  - "mc.example.com"
addresses:
  - "192.168.1.100:25565"
proxyMode: "passthrough"

server_manager:
  provider_name: Pterodactyl
  server_id: "de0d8f2d"
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
| `provider_name` | Doit être `Pterodactyl` pour le panneau Pterodactyl | Oui |
| `server_id` | L'identifiant du serveur dans le panneau Pterodactyl | Oui |
| `empty_shutdown_time` | Secondes avant l'arrêt d'un serveur vide | Non |

## Exemple complet

### Configuration principale (`config.yaml`)

```yaml
bind: "0.0.0.0:25565"

file_provider:
  proxies_path:
    - "./proxies"
  watch: true

managers_config:
  pterodactyl:
    enabled: true
    api_key: "ptlc_xxxxxxxxxxxxxxxxxxxx"
    base_url: "https://panel.example.com"
```

### Configuration du proxy (`proxies/survival.yaml`)

```yaml
domains:
  - "survival.example.com"
  - "play.example.com"
addresses:
  - "192.168.1.100:25565"
sendProxyProtocol: false
proxyMode: "passthrough"

server_manager:
  provider_name: Pterodactyl
  server_id: "de0d8f2d"
  empty_shutdown_time: 300

motds:
  online:
    version_name: "Serveur Survie"
    text: "Bienvenue en Survie !"
  offline:
    version_name: "Démarrage du serveur..."
    text: "Le serveur est hors ligne. Rejoignez pour le démarrer !"
```

## États du serveur

L'intégration Pterodactyl reconnaît les états de serveur suivants :

| État | Description |
|------|-------------|
| `Starting` | Le serveur démarre |
| `Running` | Le serveur est en ligne et accepte les connexions |
| `Stopping` | Le serveur s'arrête |
| `Stopped` | Le serveur est hors ligne |
| `Crashed` | Le serveur a planté et peut nécessiter une attention |

## Fonction d'arrêt automatique

Lorsque `empty_shutdown_time` est configuré, Infrarust arrêtera automatiquement le serveur après le nombre de secondes spécifié lorsqu'aucun joueur n'est connecté. Cela permet d'économiser des ressources lorsque les serveurs ne sont pas utilisés.

```yaml
server_manager:
  provider_name: Pterodactyl
  server_id: "de0d8f2d"
  empty_shutdown_time: 300  # Arrêt après 5 minutes sans joueurs
```

## Capacités de l'API

L'intégration prend en charge les opérations suivantes via l'API Client de Pterodactyl :

- **Obtenir l'état du serveur** : Vérifier l'état actuel du serveur et l'utilisation des ressources
- **Démarrer le serveur** : Démarrer un serveur hors ligne
- **Arrêter le serveur** : Arrêter proprement un serveur en cours d'exécution
- **Redémarrer le serveur** : Redémarrer un serveur en cours d'exécution

### Points d'API utilisés

| Point d'accès | Méthode | But |
|---------------|---------|-----|
| `/api/client/servers/{id}` | GET | Obtenir les informations du serveur |
| `/api/client/servers/{id}/resources` | GET | Obtenir l'état et les ressources du serveur |
| `/api/client/servers/{id}/power` | POST | Contrôler l'état d'alimentation du serveur |

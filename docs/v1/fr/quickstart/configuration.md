# Référence de Configuration

Ce document détaille toutes les options de configuration disponibles dans Infrarust.

## Structure de Configuration

Infrarust utilise deux types de fichiers de configuration :

```
infrarust/
├── config.yaml         # Configuration globale
└── proxies/           # Configurations des serveurs
    ├── hub.yml
    ├── survival.yml
    └── creative.yml
```

## Configuration Principale (config.yaml)

Le fichier de configuration principal prend en charge les options suivantes :

```yaml
# Configuration de Base
bind: "0.0.0.0:25565"           # Adresse d'écoute du proxy
keepAliveTimeout: 30s           # Délai d'expiration de la connexion

# Configuration du Fournisseur de Fichiers
file_provider:
  proxies_path: ["./proxies"]   # Chemin vers les configurations de proxy
  file_type: "yaml"             # Type de fichier (seul yaml est supporté actuellement)
  watch: true                   # Activer le rechargement à chaud des configurations

# Configuration du Fournisseur Docker
docker_provider:
  docker_host: "unix:///var/run/docker.sock"  # Socket du démon Docker
  label_prefix: "infrarust"                   # Préfixe des étiquettes pour les conteneurs
  polling_interval: 10                        # Intervalle de sondage en secondes
  watch: true                                 # Surveiller les changements de conteneurs
  default_domains: []                         # Domaines par défaut pour les conteneurs

# Configuration des Gestionnaires de Serveurs
managers_config:
  pterodactyl:
    enabled: true
    api_key: "votre_clé_api"
    base_url: "https://pterodactyl.example.com"
  crafty:
    enabled: true
    api_key: "votre_clé_api"
    base_url: "https://crafty.example.com"

# Configuration du Protocole Proxy (Réception)
proxy_protocol:
  enabled: true                    # Activer le support du protocole proxy
  receive_enabled: true            # Accepter le protocole proxy entrant
  receive_timeout_secs: 5          # Délai d'attente pour la réception de l'en-tête
  receive_allowed_versions: [1, 2] # Versions autorisées du protocole proxy

# Configuration du Cache
cache:
  status_ttl_seconds: 30        # Durée de vie des entrées du cache de statut
  max_status_entries: 1000      # Nombre maximal d'entrées dans le cache de statut

# Configuration de la Télémétrie
telemetry:
  enabled: false               # Activer la collecte de télémétrie
  export_interval_seconds: 30  # Intervalle d'exportation
  export_url: "http://..."     # Destination d'exportation (optionnel)
  enable_metrics: false        # Activer la collecte de métriques
  enable_tracing: false        # Activer le traçage distribué

# Configuration des Journaux
logging:
  debug: true                  # Activer le mode débogage
  use_color: true              # Utiliser des couleurs dans la sortie console
  use_icons: true              # Utiliser des icônes dans la sortie console
  show_timestamp: true         # Afficher l'horodatage dans les journaux
  time_format: "%Y-%m-%d %H:%M:%S"  # Format d'horodatage
  show_target: true            # Afficher la cible du journal
  show_fields: true            # Afficher les champs du journal
  template: "{timestamp} [{level}] {message}"  # Modèle de journal
  regex_filter: "^(pattern)"   # Filtrer les journaux par expression régulière
  min_level: "info"            # Niveau de journal minimum global
  log_types:                   # Niveaux de journal par composant
    supervisor: "info"
    server_manager: "info"
    packet_processing: "debug"
    proxy_protocol: "debug"
    ban_system: "info"
    authentication: "info"
    filter: "info"
    config_provider: "info"
    cache: "debug"
    motd: "warn"
    telemetry: "error"
  exclude_types:               # Exclure les types de journaux bruyants
    - "tcp_connection"
    - "packet_processing"
    - "cache"

# Configuration des Filtres
filters:
  rate_limiter:
    enabled: true
    requests_per_minute: 600   # Requêtes maximales par minute
    burst_size: 10             # Taille de rafale pour la limitation

# Configuration MOTD par Défaut
motds:
  unreachable:
    version_name: "Infrarust Inaccessible"
    protocol_version: 760
    max_players: 100
    online_players: 0
    text: "Serveur Inaccessible"
    favicon: ""
```

## Configuration des Serveurs (proxies/*.yml)

Chaque fichier de configuration de serveur dans le répertoire proxies peut contenir :

```yaml
domains:
  - "play.example.com"      # Noms de domaine pour ce serveur
addresses:
  - "localhost:25566"       # Adresses des serveurs cibles

sendProxyProtocol: false    # Envoyer le protocole PROXY au backend
proxy_protocol_version: 2   # Version du protocole PROXY à utiliser (1 ou 2)

proxyMode: "passthrough"    # Mode proxy (passthrough/client_only/offline/server_only)

# Configuration du Gestionnaire de Serveur (optionnel)
server_manager:
  provider_name: Local       # Local | Pterodactyl | Crafty | Docker
  server_id: "mon_serveur"
  empty_shutdown_time: 300   # Arrêt après temps d'inactivité (secondes)
  local_provider:            # Uniquement pour le fournisseur Local
    executable: "java"
    working_dir: "/chemin/vers/serveur"
    args:
      - "-jar"
      - "server.jar"
    startup_string: 'For help, type "help"'

# Configuration MOTD (par serveur)
motds:
  online:
    enabled: true
    text: "Bienvenue sur notre serveur !"
    version_name: "Paper 1.20.4"
    max_players: 100
    online_players: 42
    protocol_version: 765
    favicon: "./icons/server.png"
    samples:
      - name: "Steve"
        id: "069a79f4-44e9-4726-a5be-fca90e38aaf5"
  offline:
    enabled: true
    text: "Serveur en veille - Connectez-vous pour le réveiller !"
  # Autres états : starting, stopping, shutting_down, crashed, unreachable, unable_status

# Configuration des Filtres par Serveur
filters:
  rate_limiter:
    enabled: true
    requests_per_minute: 600
    burst_size: 10
  ip_filter:
    enabled: true
    whitelist: ["127.0.0.1"]
    blacklist: []
  id_filter:
    enabled: true
    whitelist: ["uuid1", "uuid2"]
    blacklist: []
  name_filter:
    enabled: true
    whitelist: ["joueur1"]
    blacklist: []
  ban:
    enabled: true
    storage_type: "file"
    file_path: "bans.json"

# Configuration du Cache par Serveur
caches:
  status_ttl_seconds: 30
  max_status_entries: 1000
```

Pour des exemples complets de configuration de serveur, consultez les [config_examples sur GitHub](https://github.com/Shadowner/Infrarust/tree/main/config_examples/proxies).

## Référence des Fonctionnalités

### Modes de Proxy

| Mode | Description |
|------|-------------|
| `passthrough` | Proxy direct, compatible avec toutes les versions de Minecraft |
| `client_only` | Pour les clients premium se connectant à des serveurs offline |
| `server_only` | Pour les scénarios où l'authentification du serveur nécessite une gestion |
| `offline` | Pour les clients et serveurs offline |

### Gestionnaires de Serveurs

Infrarust peut automatiquement démarrer et arrêter les serveurs Minecraft en fonction de l'activité des joueurs.

#### Intégration Pterodactyl

```yaml
managers_config:
  pterodactyl:
    enabled: true
    api_key: "votre_clé_api"
    base_url: "https://pterodactyl.example.com"
```

Puis dans la configuration de votre serveur :

```yaml
server_manager:
  provider_name: Pterodactyl
  server_id: "votre_uuid_serveur"
  empty_shutdown_time: 300
```

#### Intégration Crafty Controller

```yaml
managers_config:
  crafty:
    enabled: true
    api_key: "votre_clé_api"
    base_url: "https://crafty.example.com"
```

Puis dans la configuration de votre serveur :

```yaml
server_manager:
  provider_name: Crafty
  server_id: "votre_uuid_serveur"
```

#### Gestion de Serveur Local

Pour les serveurs gérés localement :

```yaml
server_manager:
  provider_name: Local
  server_id: "serveur_local"
  empty_shutdown_time: 300
  local_provider:
    executable: "java"
    working_dir: "/chemin/vers/serveur"
    args:
      - "-jar"
      - "server.jar"
    startup_string: 'For help, type "help"'
```

### Intégration Docker

Infrarust peut automatiquement faire proxy des conteneurs Minecraft :

```yaml
docker_provider:
  docker_host: "unix:///var/run/docker.sock"
  label_prefix: "infrarust"
  polling_interval: 10
  watch: true
  default_domains: ["docker.local"]
```

La configuration des conteneurs se fait via les étiquettes Docker :
- `infrarust.enable=true` - Activer le proxy pour le conteneur
- `infrarust.domains=mc.example.com,mc2.example.com` - Domaines pour le conteneur
- `infrarust.port=25565` - Port Minecraft à l'intérieur du conteneur
- `infrarust.proxy_mode=passthrough` - Mode proxy
- `infrarust.proxy_protocol=true` - Activer le protocole PROXY

### Protocole Proxy

Configurer le protocole PROXY pour recevoir les informations client des load balancers :

```yaml
proxy_protocol:
  enabled: true
  receive_enabled: true
  receive_timeout_secs: 5
  receive_allowed_versions: [1, 2]
```

Pour envoyer le protocole PROXY aux serveurs backend, configurez dans la configuration du serveur :

```yaml
sendProxyProtocol: true
proxy_protocol_version: 2
```

### Télémétrie

La configuration de télémétrie permet la surveillance du proxy via OpenTelemetry :

```yaml
telemetry:
  enabled: true
  export_interval_seconds: 10
  export_url: "http://localhost:4317"
  enable_metrics: true
  enable_tracing: true
```

### Configuration MOTD

Configurer l'affichage de la liste des serveurs pour différents états :

| État | Description |
|------|-------------|
| `online` | Le serveur est en cours d'exécution et accessible |
| `offline` | Le serveur est en veille/arrêté |
| `starting` | Le serveur démarre |
| `stopping` | Le serveur s'arrête gracieusement |
| `shutting_down` | Compte à rebours avant l'arrêt (supporte le placeholder `${seconds_remaining}`) |
| `crashed` | Le serveur a planté |
| `unreachable` | Impossible d'atteindre le serveur |
| `unable_status` | Impossible d'obtenir le statut du serveur |

Champs MOTD :
- `enabled` - Activer cet état MOTD
- `text` - Description du serveur (supporte les codes de couleur Minecraft)
- `version_name` - Texte de version à afficher
- `protocol_version` - Numéro de version du protocole Minecraft
- `max_players` - Nombre maximal de joueurs
- `online_players` - Nombre actuel de joueurs
- `favicon` - Icône du serveur (PNG encodé en base64 ou chemin de fichier)
- `samples` - Échantillons de liste de joueurs (tableau de `{name, id}`)

Pour des exemples complets de MOTD, voir [local-server.yaml](https://github.com/Shadowner/Infrarust/blob/main/config_examples/proxies/local-server.yaml).

### Configuration du Cache

Configurer la mise en cache des statuts :

```yaml
cache:
  status_ttl_seconds: 30    # Durée de vie des entrées du cache de statut
  max_status_entries: 1000  # Nombre maximal d'entrées dans le cache de statut
```

### Configuration des Filtres

#### Limiteur de Débit

Contrôle le nombre de connexions depuis une source unique :

```yaml
rate_limiter:
  enabled: true
  requests_per_minute: 600  # Requêtes maximales par minute
  burst_size: 10            # Tolérance de rafale
```

#### Listes d'Accès

Disponibles pour les adresses IP, UUIDs et noms de joueurs :

```yaml
ip_filter:  # ou id_filter / name_filter
  enabled: true
  whitelist: ["valeur1", "valeur2"]
  blacklist: ["valeur3"]
```

#### Système de Bannissement

Configurer les bannissements persistants des joueurs :

```yaml
ban:
  enabled: true
  storage_type: "file"  # file, redis, ou database
  file_path: "bans.json"
  enable_audit_log: true
  audit_log_path: "bans_audit.log"
  audit_log_rotation:
    max_size: 10485760  # 10Mo
    max_files: 5
    compress: true
  auto_cleanup_interval: 3600  # 1 heure
  cache_size: 10000
```

### Configuration des Journaux

Affiner la sortie des journaux :

```yaml
logging:
  debug: true
  use_color: true
  use_icons: true
  show_timestamp: true
  time_format: "%Y-%m-%d %H:%M:%S"
  show_target: true
  show_fields: true
  template: "{timestamp} [{level}] {message}"
  regex_filter: "^(pattern)"
  min_level: "info"
  log_types:
    supervisor: "info"
    server_manager: "info"
    packet_processing: "debug"
    ban_system: "info"
    authentication: "info"
    telemetry: "warn"
  exclude_types:
    - "tcp_connection"
    - "cache"
```

Types de journaux disponibles : `tcp_connection`, `supervisor`, `server_manager`, `packet_processing`, `ban_system`, `authentication`, `telemetry`, `config_provider`, `proxy_protocol`, `cache`, `filter`, `proxy_mode`, `motd`

## Fonctionnalités Avancées

### Rechargement à Chaud

Lorsque `file_provider.watch` est activé, les changements de configuration sont automatiquement détectés et appliqués sans redémarrage.

> Actif par défaut

### Intégration Docker

Lorsque `docker_provider.watch` est activé, les changements de conteneurs sont automatiquement détectés et les proxies sont mis à jour en conséquence.

### Système de Bannissement

Le système de bannissement fournit des bannissements persistants avec des options de stockage flexibles et une journalisation d'audit.

## Besoin d'Aide ?

- Signalez les problèmes sur [GitHub](https://github.com/shadowner/infrarust/issues)
- Rejoignez notre [Discord](https://discord.gg/sqbJhZVSgG)
- Consultez la [documentation](https://infrarust.dev)

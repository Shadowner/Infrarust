
# Configuration d'Infrarust

Ce document détaille toutes les options de configuration disponibles dans Infrarust.

## Structure des Fichiers de Configuration

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
keepalive_timeout: 30s          # Délai d'expiration de la connexion
domains: ["example.com"]        # Domaines par défaut (optionnel)
addresses: ["localhost:25566"]  # Adresses cibles par défaut (optionnel)

# Configuration du Fournisseur de Fichiers
file_provider:
  proxies_path: ["./proxies"]   # Chemin vers les configurations de proxy
  file_type: "yaml"            # Type de fichier (seul yaml est supporté actuellement)
  watch: true                  # Activer le rechargement à chaud des configurations

# Configuration du Fournisseur Docker
docker_provider:
  docker_host: "unix:///var/run/docker.sock"  # Socket du démon Docker
  label_prefix: "infrarust"                   # Préfixe des étiquettes pour les conteneurs
  polling_interval: 10                        # Intervalle de sondage en secondes
  watch: true                                 # Surveiller les changements de conteneurs
  default_domains: []                         # Domaines par défaut pour les conteneurs

# Configuration du Cache
cache:
  status_ttl_seconds: 30        # Durée de vie des entrées du cache de statut
  max_status_entries: 1000      # Nombre maximal d'entrées dans le cache de statut

# Configuration de la Télémétrie
telemetry:
  enabled: false               # Activer la collecte de télémétrie
  export_interval_seconds: 30  # Intervalle d'exportation
  export_url: "http://..."    # Destination d'exportation (optionnel)
  enable_metrics: false       # Activer la collecte de métriques
  enable_tracing: false      # Activer le traçage distribué

# Configuration des Journaux
logging:
  use_color: true              # Utiliser des couleurs dans la sortie console
  use_icons: true              # Utiliser des icônes dans la sortie console
  show_timestamp: true         # Afficher l'horodatage dans les journaux
  time_format: "%Y-%m-%d %H:%M:%S%.3f"  # Format d'horodatage
  show_target: false           # Afficher la cible du journal
  show_fields: false           # Afficher les champs du journal
  template: "{timestamp} {level}: {message}"  # Modèle de journal
  field_prefixes: {}           # Mappages des préfixes de champs

# Configuration MOTD par défaut
motds:
  unknown:                    # MOTD pour les serveurs inconnus
    version: "1.20.1"        # Version Minecraft à afficher
    max_players: 100         # Nombre maximal de joueurs à afficher
    online_players: 0        # Nombre de joueurs en ligne à afficher
    description: "Unknown server" # Description du serveur
    favicon: "data:image/png;base64,..." # Icône du serveur (optionnel)
  unreachable:              # MOTD pour les serveurs inaccessibles
    # Mêmes options que 'unknown'
```

## Configuration des Serveurs (proxies/*.yml)

Chaque fichier dans le dossier `proxies/` représente un serveur Minecraft.

```yaml
domains:
  - "play.example.com"      # Noms de domaine pour ce serveur
addresses:
  - "localhost:25566"       # Adresses des serveurs cibles

sendProxyProtocol: false    # Activer le support du protocole PROXY
proxy_protocol_version: 2   # Version du protocole PROXY à utiliser (1 ou 2)

proxyMode: "passthrough"    # Mode proxy (passthrough/client_only/offline/server_only)

# Configuration MOTD (remplace le MOTD par défaut / du serveur)
motd:
  version: "1.20.1"       # Peut être défini comme n'importe quel texte
  max_players: 100
  online_players: 0
  description: "Bienvenue sur mon serveur !"
  favicon: "data:image/png;base64,..."

### FONCTIONNALITÉS CI-DESSOUS IMPLÉMENTÉES MAIS PAS ENCORE SUPPORTÉES ###

# Configuration du Cache
caches:
  status_ttl_seconds: 30    # Durée de vie des entrées du cache de statut
  max_status_entries: 1000  # Nombre maximal d'entrées dans le cache de statut

# Configuration des Filtres
filters:
  rate_limiter:
    requestLimit: 10        # Nombre maximal de requêtes par fenêtre
    windowLength: 1s        # Fenêtre temporelle pour la limitation de débit
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
    whitelist: ["player1"]
    blacklist: []
  ban:
    enabled: true
    storage_type: "file"    # Type de stockage (file/redis/database)
    file_path: "bans.json"  # Chemin vers le fichier de stockage des bannissements
    enable_audit_log: true  # Activer la journalisation des audits de bannissement
    audit_log_path: "bans_audit.log"  # Chemin vers le journal d'audit
    audit_log_rotation:     # Paramètres de rotation des journaux
      max_size: 10485760    # Taille maximale du journal (10Mo)
      max_files: 5          # Nombre maximal de fichiers journaux
      compress: true        # Compresser les journaux pivotés
    auto_cleanup_interval: 3600  # Intervalle de nettoyage automatique en secondes
    cache_size: 10000      # Taille du cache de bannissement
```

## Référence des Fonctionnalités

### Modes de Proxy

| Mode | Description |
|------|-------------|
| `passthrough` | Proxy direct, compatible avec toutes les versions de Minecraft |
| `client_only` | Pour les clients premium se connectant à des serveurs offline |
| `server_only` | Pour les scénarios où l'authentification du serveur nécessite une gestion |
| `offline` | Pour les clients et serveurs offline |

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

Configuration des conteneurs via les étiquettes Docker :
- `infrarust.enable=true` - Activer le proxy pour le conteneur
- `infrarust.domains=mc.example.com,mc2.example.com` - Domaines pour le conteneur
- `infrarust.port=25565` - Port Minecraft à l'intérieur du conteneur
- `infrarust.proxy_mode=passthrough` - Mode proxy
- `infrarust.proxy_protocol=true` - Activer le protocole PROXY

### Télémétrie

La configuration de télémétrie permet la surveillance du proxy :

```yaml
telemetry:
  enabled: false
  export_interval_seconds: 30
  export_url: "http://..."
  enable_metrics: false
  enable_tracing: false
```

### Configuration MOTD

Configure l'affichage de la liste des serveurs :

```yaml
motd:
  version: "1.20.1"        # Version du protocole à afficher
  max_players: 100         # Nombre maximal de joueurs
  online_players: 0        # Nombre actuel de joueurs
  description: "Texte"     # Description du serveur
  favicon: "base64..."     # Icône du serveur (PNG encodé en base64)
```

### Configuration du Cache

Configure le cache de statut :

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
  requestLimit: 10    # Requêtes maximales
  windowLength: 1s    # Fenêtre temporelle
```

#### Listes d'Accès

Disponibles pour les adresses IP, UUIDs, et noms de joueurs :

```yaml
ip_filter:  # ou id_filter / name_filter
  enabled: true
  whitelist: ["valeur1", "valeur2"]
  blacklist: ["valeur3"]
```

#### Système de Bannissement

Configure les bannissements persistants des joueurs :

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

Affine la sortie des journaux :

```yaml
logging:
  use_color: true
  use_icons: true
  show_timestamp: true
  time_format: "%Y-%m-%d %H:%M:%S%.3f"
  show_target: false
  show_fields: false
  template: "{timestamp} {level}: {message}"
  field_prefixes: {}
```

## Fonctionnalités Avancées

### Rechargement à Chaud

Lorsque `file_provider.watch` est activé, les changements de configuration sont automatiquement détectés et appliqués sans redémarrage.

> Actif par défaut

### Intégration Docker

Lorsque `docker_provider.watch` est activé, les changements de conteneurs sont automatiquement détectés et les proxies sont mis à jour en conséquence.

### Système de Bannissement

Le système de bannissement fournit des bannissements persistants avec des options de stockage flexibles et une journalisation d'audit.

## Besoin d'Aide ?

- 🐛 Signalez les problèmes sur [GitHub](https://github.com/shadowner/infrarust/issues)
- 💬 Rejoignez notre [Discord](https://discord.gg/sqbJhZVSgG)
- 📚 Consultez la [documentation](https://infrarust.dev)

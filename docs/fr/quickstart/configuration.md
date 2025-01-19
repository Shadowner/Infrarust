# Configuration d'Infrarust

Ce guide détaille toutes les options de configuration disponibles dans Infrarust.

:::warning
**Note:** Cette documentation peut ne pas refléter toutes les fonctionnalités actuellement implémentées. Les fonctionnalités non disponibles seront clairement indiquées comme telles dans la documentation.
:::

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

## Configuration Globale

Le fichier `config.yaml` contient la configuration principale d'Infrarust.

### Configuration Minimale

```yaml
bind: "0.0.0.0:25565"
domains: ### NOT IMPLEMENTED YET ###
  - "*.minecraft.example.com"
keepalive_timeout: 30s
```

### Configuration Complète

```yaml
# Adresse d'écoute du proxy
bind: "0.0.0.0:25565"

# Liste des domaines acceptés
domains:
  - "*.minecraft.example.com"
  - "play.example.com"

# Paramètres de timeout
keepalive_timeout: 30s
read_timeout: 30s ### NOT IMPLEMENTED YET ###
write_timeout: 30s ### NOT IMPLEMENTED YET ###

# Configuration du cache
status_cache: ### NOT IMPLEMENTED YET ###
  enabled: true
  ttl: 30s
  max_size: 1000

# Sécurité
security: ### NOT IMPLEMENTED YET (only for proxy leve not server level) ###
  # Protection DDoS
  rate_limiter:
    enabled: true
    requests: 10
    window: "1s"
  
  # Filtrage IP
  ip_filter: ### NOT IMPLEMENTED YET ###
    enabled: true
    blacklist:
      - "1.2.3.4"
      - "10.0.0.0/8"
    whitelist:
      - "192.168.1.0/24"

# Configuration des logs
logging: ### NOT IMPLEMENTED YET ###
  level: "info"  # debug, info, warn, error
  file: "logs/infrarust.log"
  format: "json"  # json, text

# Configuration de la performance
performance: ### NOT IMPLEMENTED YET ###
  workers: 4
  connection_pool_size: 100
  buffer_size: 8192
```

## Configuration des Serveurs Proxy

Chaque fichier dans le dossier `proxies/` représente un serveur Minecraft.

### Configuration Simple

```yaml
# proxies/hub.yml
domains:
  - "hub.minecraft.example.com"
addresses:
  - "localhost:25566"
proxy_mode: "passthrough"
```

### Configuration Avancée

:::warning
 **Note:** Toutes les fonctionnalitées ne sont pas encore implémenté mais ce fichier de configuration représente un des objectifs de développement
 [#1 Roadmap](https://github.com/Shadowner/Infrarust/issues/1)
:::

```yaml
# Configuration complète d'un serveur
name: "Hub Principal"

# Domaines acceptés
domains:
  - "hub.minecraft.example.com"
  - "*.hub.minecraft.example.com"

# Adresses des serveurs backend
addresses:
  - "localhost:25566"
  - "localhost:25567"  # Failover 

# Mode de proxy
proxy_mode: "clientOnly"  # passthrough, clientOnly, offline

# Configuration de l'authentification
authentication: ### NOT IMPLEMENTED YET ###
  enabled: true
  timeout: 30s
  cache_time: 300s

# Configuration du protocole proxy
proxy_protocol: ### NOT IMPLEMENTED YET ###
  enabled: true
  version: 2
  trusted_proxies:
    - "10.0.0.0/8"

# Équilibrage de charge
load_balancing: ### NOT IMPLEMENTED YET ###
  method: "round_robin"  # round_robin, least_conn, random
  health_check:
    enabled: true
    interval: 10s
    timeout: 5s
    unhealthy_threshold: 3

# Limites par serveur
limits: ### NOT IMPLEMENTED YET ###
  max_players: 1000
  max_connections_per_ip: 3

# Configuration du motd
motd: ### NOT IMPLEMENTED YET ###
  enabled: true
  custom_text: "§6§lHub Principal §r- §aEn ligne"
  max_players: 1000
  
# Compression
compression: ### NOT IMPLEMENTED YET ###
  threshold: 256
  level: 6
```

## Variables d'Environnement - Non implémenté

Infrarust supporte la configuration via variables d'environnement :

| Variable | Description | Défaut |
|----------|-------------|---------|
| `INFRARUST_CONFIG` | Chemin du fichier config | `config.yaml` |
| `INFRARUST_PROXIES_DIR` | Dossier des proxies | `proxies` |
| `INFRARUST_LOG_LEVEL` | Niveau de log | `info` |
| `INFRARUST_BIND` | Adresse d'écoute | `0.0.0.0:25565` |

## Modes de Proxy

Infrarust prend en charge différents modes de proxy qui peuvent être configurés pour chaque serveur. Voir [Modes de Proxy](/proxy/) pour plus d'informations.

| Mode | Description |
|------|-------------|
| `passthrough` | Transmission directe de la connexion |
| `clientOnly` | Validation côté client uniquement |
| `offline` | Fonctionnement en mode hors ligne |

## Options Avancées

### Configuration du TLS

```yaml
tls:
  enabled: true
  cert_file: "cert.pem"
  key_file: "key.pem"
  min_version: "1.2"
```

### Métriques et Monitoring - Non implémenté

```yaml
metrics:
  enabled: true
  bind: "127.0.0.1:9100"
  path: "/metrics"
  type: prometheus
```

## Validation de la Configuration - Non implémenté

Pour valider votre configuration :

```bash
infrarust validate --config config.yaml
```

::: tip
Les valeurs de timeout acceptent des suffixes : "s" (secondes), "m" (minutes), "h" (heures).

```yaml
keepalive_timeout: 30s
read_timeout: 1m 
write_timeout: 2h
```

:::

::: warning
Les changements de configuration nécessitent un redémarrage du proxy sauf si le rechargement à chaud est activé.
:::

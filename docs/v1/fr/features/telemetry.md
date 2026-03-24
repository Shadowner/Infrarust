# Télémétrie et Surveillance

Infrarust fournit une télémétrie complète via OpenTelemetry, incluant les métriques, les traces et les journaux.

## Configuration

Activez la télémétrie dans votre `config.yaml` :

```yaml
telemetry:
  enabled: true                    # Activer la collecte de télémétrie
  export_interval_seconds: 30      # Intervalle d'exportation
  export_url: "http://127.0.0.1:4317"  # Point de terminaison OTLP
  enable_metrics: true            # Activer la collecte de métriques
  enable_tracing: true           # Activer le traçage distribué
```

## Métriques Disponibles

### Métriques de Connexion

- `connections.active` - Nombre actuel de connexions actives
- `connections.errors` - Nombre d'erreurs de connexion
- `network.bytes` - Octets transférés actuellement
- `network.bytes.total` - Total des octets transférés depuis le démarrage
- `connections.latency` - Latence de connexion
- `requests.rate` - Nombre de requêtes par seconde

### Métriques Backend

- `backends.active` - Nombre de serveurs backend actifs
- `backends.latency` - Temps de réponse du serveur backend
- `backends.errors` - Nombre d'erreurs backend
- `backends.requests` - Total des requêtes backend

### Métriques Système

- `system.cpu` - Pourcentage d'utilisation CPU
- `system.memory` - Utilisation de la mémoire
- `system.open_files` - Nombre de fichiers ouverts
- `system.threads` - Nombre de threads
- `system.internal_errors` - Nombre d'erreurs internes

### Métriques Spécifiques à Minecraft

- `minecraft.protocol_errors` - Nombre d'erreurs de protocole Minecraft
- `minecraft.players` - Nombre de joueurs connectés
- `minecraft.packet_time` - Temps de traitement des paquets

## Stack de Surveillance - Démarrage Rapide

Infrarust inclut une stack de surveillance prête à l'emploi dans le répertoire monitoring.

### Prérequis

- Docker
- Docker Compose

### Démarrer la Stack de Surveillance

```bash
cd docker/monitoring
docker compose up -d
```

Cela démarrera :

- Grafana (Interface : <http://127.0.0.1:3000>)
- Prometheus (Interface : <http://127.0.0.1:9090>)
- Tempo (Traces)
- Collecteur OpenTelemetry

### Fichiers de Configuration

#### Collecteur OpenTelemetry

```yaml
receivers:
  otlp:
    protocols:
      grpc:
        endpoint: "0.0.0.0:4317"
      http:
        endpoint: "0.0.0.0:4318"

processors:
  batch:

exporters:
  prometheus:
    endpoint: "0.0.0.0:8889"
  otlp:
    endpoint: "tempo:4317"

service:
  pipelines:
    metrics:
      receivers: [otlp]
      processors: [batch]
      exporters: [prometheus]
    traces:
      receivers: [otlp]
      processors: [batch]
      exporters: [otlp]
```

### Accès à la Stack de Surveillance

1. **Grafana** : <http://127.0.0.1:3000>
   - Identifiants par défaut : admin/admin
   - Tableaux de bord préconfigurés disponibles

2. **Prometheus** : <http://127.0.0.1:9090>
   - Accès direct aux métriques
   - Interface de requête pour l'exploration des métriques

3. **Tempo** : Accessible via Grafana
   - Visualisation du traçage distribué
   - Recherche et analyse des traces

### Tableaux de Bord Disponibles

La stack de surveillance inclut des tableaux de bord préconfigurés pour :

- Tableau de Bord Global

### Exemples de Traces

Traces courantes disponibles :

- Flux de Connexion TCP
- Traitement des Paquets
- Configuration du Fournisseur
- Mise à jour de Configuration

### Exemples de Métriques

```promql
# Connexions Actives
rate(connections_active_total[5m])

# Latence Backend
histogram_quantile(0.95, sum(rate(backends_latency_bucket[5m])) by (le))

# Erreurs de Protocole
sum(minecraft_protocol_errors_total) by (error_type)
```

## Dépannage

### Problèmes Courants

1. **Aucune métrique n'apparaît**
   - Vérifiez que la configuration de télémétrie est activée
   - Vérifiez l'accessibilité du point de terminaison OTLP
   - Vérifiez que le collecteur est en cours d'exécution

2. **Latence élevée dans la collecte**
   - Ajustez les paramètres de traitement par lots
   - Vérifiez la connectivité réseau
   - Revoyez les paramètres d'intervalle d'exportation

### Mode Debug

Activez la journalisation debug pour plus d'informations détaillées sur la télémétrie :

```yaml
logging:
  level: debug  # Pas encore implémenté
```

## Ressources Additionnelles

- [Documentation OpenTelemetry](https://opentelemetry.io/docs/)
- [Documentation Grafana](https://grafana.com/docs/)
- [Documentation Prometheus](https://prometheus.io/docs/)

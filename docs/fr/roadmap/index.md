# Roadmap

> [!NOTE]  
> Infrarust est en développement actif. Cette roadmap présente les fonctionnalités majeures prévues pour les prochaines versions.

## Live Config Reloading
Mise à jour dynamique des configurations sans redémarrage du proxy.
- Rechargement à chaud des configurations
- Validation automatique des changements
- Rollback en cas d'erreur
[En savoir plus](features/live-config.md)

## Custom Auth System
Système d'authentification Minecraft personnalisable et autonome.
- Gestion des sessions indépendante
- Support des serveurs offline
- Intégration avec des systèmes tiers
[En savoir plus](features/auth-system.md)

## Plugin System
Architecture modulaire pour étendre les fonctionnalités du proxy.
- Interception des paquets en temps réel
- API de modification des données
- Support des événements
[En savoir plus](features/plugins.md)

## Telemetry
Collecte et analyse des métriques d'utilisation.
- Statistiques des joueurs et serveurs
- Monitoring des performances
- Alertes configurables
[En savoir plus](features/telemetry.md)

## REST API
Interface programmatique pour le contrôle du proxy.
- Gestion des joueurs et serveurs
- Contrôle des configurations
- Intégration avec des outils externes
[En savoir plus](features/rest-api.md)

## Web Dashboard
Interface web de gestion du proxy.
### Phase 1 - Lecture seule
- Visualisation des métriques
- État des serveurs
- Logs en temps réel

### Phase 2 - Administration
- Gestion des configurations
- Actions sur les joueurs
- Contrôle du proxy
[En savoir plus](features/dashboard.md)
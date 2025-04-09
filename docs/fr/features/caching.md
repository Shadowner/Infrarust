
# Cache

Le système de cache d'Infrarust optimise les performances en stockant temporairement les réponses des serveurs.

## Configuration

Le système de cache peut être configuré globalement dans `config.yaml` et personnalisé par serveur dans les configurations de proxy :

```yaml
# Configuration globale du cache dans config.yaml
cache:
  status_ttl_seconds: 30       # Durée de vie des entrées du cache de statut (en secondes)
  max_status_entries: 1000     # Nombre maximum d'entrées dans le cache de statut

# Configuration spécifique au serveur dans proxies/*.yml
caches:
  status_ttl_seconds: 15       # Remplace la durée de vie pour ce serveur
  max_status_entries: 500      # Remplace le nombre maximum d'entrées pour ce serveur
```

### Valeurs par défaut

Si non spécifiées, ces valeurs par défaut sont utilisées :
- `status_ttl_seconds` : 30 secondes
- `max_status_entries` : 1000 entrées

## Types de cache

### Cache de statut

Le cache de statut stocke les réponses ping/statut des serveurs :

- Réduit la charge sur les serveurs minecraft en mettant en cache les réponses de statut
- Offre des temps de réponse plus rapides pour les requêtes de liste de serveurs des clients
- Invalide automatiquement les entrées après expiration de la durée de vie
- Utilise le domaine et la version du protocole comme clés de cache

### Avantages du cache de statut

- **Protection** : Protège les serveurs minecraft contre les afflux de requêtes de statut
- **Performance** : Réduit le temps de réponse pour l'écran de liste des serveurs
- **Cohérence** : Fournit des réponses stables même lorsque les serveurs minecraft sont occupés

## Comportement du cache

Lorsqu'un client demande le statut d'un serveur :

1. Infrarust vérifie si un statut en cache valide existe
2. Si trouvé et non expiré, renvoie le statut en cache
3. Si non trouvé ou expiré, transmet la requête au serveur minecraft
4. Met en cache la nouvelle réponse pour les futures requêtes

## Fonctionnalités avancées

### Invalidation du cache

Les entrées du cache de statut sont invalidées :
- Automatiquement après expiration de la durée de vie
- Lorsque la configuration du serveur associé change
- Lorsque le serveur devient inaccessible

### Gestion de la mémoire

Infrarust limite l'utilisation de la mémoire avec le paramètre `max_status_entries`, qui plafonne le nombre de réponses de statut mises en cache. Lorsque cette limite est atteinte, les entrées les plus anciennes sont évincées en premier.

## Améliorations futures

Les fonctionnalités suivantes sont prévues mais pas encore implémentées :

- **Limites de mémoire** : Contrôle précis de l'utilisation de la mémoire
```yaml
cache:
  memory_limit_mb: 512        # Utilisation maximale de la mémoire
  cleanup_interval: 60        # Intervalle de nettoyage en secondes
```

- **Compression du cache** : Réduction de l'empreinte mémoire par compression
- **Éviction intelligente** : Stratégies d'éviction du cache plus sophistiquées
- **Persistance du cache** : Persistance optionnelle des données du cache sur disque
- **Métriques de cache** : Statistiques détaillées sur les performances du cache

## Métriques du cache (Planifiées)

Dans les futures versions, le cache exposera des métriques détaillées :

- Ratio hits/misses
- Utilisation de la mémoire
- Temps de réponse moyen
- Nombre d'entrées actives
- Taux d'éviction

Ces métriques seront disponibles via le système de télémétrie d'Infrarust.

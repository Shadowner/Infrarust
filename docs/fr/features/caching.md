# Cache

Le système de cache d'Infrarust optimise les performances en stockant temporairement les réponses des serveurs.

## Configuration - Non implémenté

```yaml
cache:
  # Durée de vie des entrées du cache (en secondes)
  ttl: 30
  
  # Taille maximale du cache en mémoire (en MB)
  maxSize: 100
  
  # Activer le cache des réponses de statut
  statusCache: true
```

## Types de Cache

### Cache de Statut

- Stocke les réponses de ping/statut des serveurs
- Format de clé : `domain:version`
- Réduit la charge sur les serveurs backend
- Mise à jour automatique à expiration

## Optimisations

### Gestion de la Mémoire  - Non implémenté

```yaml
cache:
  memoryLimit: 512 # MB
  cleanupInterval: 60 # secondes
```

### Performance

- Utilisation de hashmaps pour accès O(1)
- Nettoyage asynchrone
- Compression des données en mémoire
- Éviction intelligente (LRU)

## Métriques - Non implémenté

Le cache expose des statistiques :

- Taux de hit/miss
- Utilisation mémoire
- Temps de réponse moyen
- Entrées actives

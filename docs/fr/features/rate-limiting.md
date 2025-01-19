# Rate Limiting

Le rate limiting est un mécanisme de sécurité essentiel qui permet de contrôler le nombre de connexions par adresse IP dans un intervalle de temps donné.

## Configuration

Pour activer le rate limiting, ajoutez la configuration suivante dans votre fichier `config.yaml` :

```yaml
security:
  rateLimiter:
    # Nombre maximal de requêtes autorisées par fenêtre de temps
    requestLimit: 10
    
    # Durée de la fenêtre de temps (en secondes)
    windowDuration: 1
    
    # Temps de blocage après dépassement (en secondes)
    blockDuration: 300
```

## Paramètres

| Paramètre | Description | Valeur par défaut |
|-----------|-------------|-------------------|
| `requestLimit` | Nombre maximum de requêtes | 10 |
| `windowDuration` | Durée de la fenêtre (secondes) | 1 |
| `blockDuration` | Durée du blocage (secondes) | 300 |

## Fonctionnement

1. Une fenêtre glissante est maintenue pour chaque adresse IP
2. Chaque nouvelle connexion incrémente un compteur
3. Si le compteur dépasse `requestLimit` dans la fenêtre :
   - L'IP est bloquée pendant `blockDuration`
   - Les nouvelles connexions sont rejetées
   - Un message d'erreur est envoyé au client

## Protection DDoS

Le rate limiting fait partie de la stratégie de protection DDoS avec :

- Filtrage par IP
- Limitations par sous-réseau
- Seuils adaptatifs
- Liste noire temporaire

## Exemples de Configuration

### Configuration Basique

```yaml
security:
  rateLimiter:
    requestLimit: 10
    windowDuration: 1
```

### Configuration Stricte

```yaml
security:
  rateLimiter:
    requestLimit: 5
    windowDuration: 1
    blockDuration: 600
```

## Monitoring - Non implémenté

Le rate limiter exposera dans le futur des métriques :

- Nombre de connexions bloquées
- IPs actuellement bloquées
- Taux de blocage
- Pics de connexions

Consultez [Monitoring](../quickstart/deployment) pour plus de détails.
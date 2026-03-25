
# Commandes de diagnostic

Cette page documente les commandes utilisées pour le diagnostic et le dépannage dans Infrarust.

## debug

Affiche des informations de débogage détaillées sur les acteurs et les tâches actifs.

**Utilisation:**
```
> debug
```

**Exemple de sortie:**
```
=== Informations de débogage des acteurs et des tâches ===

Configuration hub.minecraft.example.com - 2 acteurs
  1. Steve - Session: 7f8d3a2e-9c5b-4b1d-8a7c-3d2f6e9a1b5c - Âge: 2h 15m 30s - ACTIF
  2. Alex - Session: 6e7c2b1d-8a5e-3b9f-7c8d-2e1a4b3c5d6e - Âge: 45m 12s - ACTIF

Configuration survival.minecraft.example.com - 2 acteurs
  1. Notch - Session: 5d4c3b2a-1e9f-8d7c-6b5e-4a3c2d1e9f8a - Âge: 1h 5m 42s - ACTIF
  2. <status> - Session: 4c3b2a1d-0f9e-8d7c-6b5e-4a3c2d1e9f8a - Âge: 3h 20m 15s - ACTIF #Ne devrait jamais être aussi âgé

Utilisation mémoire du processus actuel: 42.75 MB

=== Total des acteurs: 4 ===
```

**Remarques:**
- Cette commande est principalement destinée à des fins de diagnostic
- Les informations d'utilisation de la mémoire ne sont disponibles que sur certaines plateformes
- Les acteurs `<status>` sont utilisés pour vérifier l'état du serveur

## tasks

Affiche des informations détaillées sur les tâches d'arrière-plan et leur état.

**Utilisation:**
```
> tasks
```

**Exemple de sortie:**
```
=== Moniteur de tâches ===

Résumé: 8 au total, 5 en cours, 3 terminées, 0 orphelines

Configuration hub.minecraft.example.com - 4 tâches, 2 acteurs - En bonne santé
  Tâches:
    3 en cours, 1 terminée

Configuration survival.minecraft.example.com - 4 tâches, 2 acteurs - En bonne santé
  Tâches:
    2 en cours, 2 terminées
```

**Remarques:**
- Cette commande est utile pour surveiller l'état des tâches d'arrière-plan
- Si des tâches orphelines existent, un avertissement sera affiché
- Pour les configurations présentant des problèmes potentiels, des listes détaillées de tâches seront affichées

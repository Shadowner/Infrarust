
# Commandes du Système de Bannissement

Cette page documente les commandes liées au bannissement des joueurs et à la gestion de la sécurité dans Infrarust.

## ban

Bannit un joueur par adresse IP, nom d'utilisateur ou UUID.

**Utilisation:**
```
> ban [--ip/-ip <adresse> | --username/-u <nom d'utilisateur> | --uuid/-id <uuid>] [--reason <raison>] [--duration <durée>]
```

**Paramètres:**
- `--ip` ou `-ip`: L'adresse IP à bannir
- `--username` ou `-u`: Le nom d'utilisateur à bannir
- `--uuid` ou `-id`: L'UUID à bannir
- `--reason`: La raison du bannissement (optionnel, par défaut "Banni par l'administrateur")
- `--duration`: La durée du bannissement (optionnel, par défaut permanent)

**Format de durée:**
- `Xs`: X secondes
- `Xm`: X minutes
- `Xh`: X heures
- `Xd`: X jours
- `Xw`: X semaines
- `Xmo`: X mois
- `Xy`: X années

**Exemples:**
```
> ban --ip 192.168.1.10 --reason "Spam" --duration 2d
Bannissement appliqué avec succès:
  IP: 192.168.1.10
  Raison: Spam
  Durée: 2 jours

> ban --username Steve --reason "Griefing"
Bannissement appliqué avec succès:
  Nom d'utilisateur: Steve
  Raison: Griefing
  Durée: Permanent

> ban --uuid 7f8d3a2e-9c5b-4b1d-8a7c-3d2f6e9a1b5c --duration 1w
Bannissement appliqué avec succès:
  UUID: 7f8d3a2e-9c5b-4b1d-8a7c-3d2f6e9a1b5c
  Raison: Banni par l'administrateur
  Durée: 1 semaine
```

**Remarques:**
- Au moins un identifiant (IP, nom d'utilisateur ou UUID) est requis
- Le filtre de bannissement doit être activé dans votre configuration pour que cette commande fonctionne

## unban

Supprime un bannissement par adresse IP, nom d'utilisateur ou UUID.

**Utilisation:**
```
> unban [--ip/-ip <adresse> | --username/-u <nom d'utilisateur> | --uuid/-id <uuid>]
```

**Paramètres:**
- `--ip` ou `-ip`: L'adresse IP à débannir
- `--username` ou `-u`: Le nom d'utilisateur à débannir
- `--uuid` ou `-id`: L'UUID à débannir

**Exemples:**
```
> unban --ip 192.168.1.10
Bannissement supprimé avec succès pour l'IP: 192.168.1.10

> unban --username Steve
Bannissement supprimé avec succès pour le nom d'utilisateur: Steve

> unban --uuid 7f8d3a2e-9c5b-4b1d-8a7c-3d2f6e9a1b5c
Bannissement supprimé avec succès pour l'UUID: 7f8d3a2e-9c5b-4b1d-8a7c-3d2f6e9a1b5c
```

**Remarques:**
- Vous devez spécifier exactement un identifiant (IP, nom d'utilisateur ou UUID)
- Si aucun bannissement n'existe pour l'identifiant spécifié, vous recevrez un avertissement
- Le filtre de bannissement doit être activé dans votre configuration pour que cette commande fonctionne

## bans

Liste tous les bannissements actifs.

**Utilisation:**
```
> bans
```

**Exemple de sortie:**
```
=== Bannissements Actifs (2) ===

1. IP: 192.168.1.10
   Raison: Spam
   Banni par: console
   Créé: il y a 2 heures
   Expire: Dans 1 jour et 22 heures (dans 1 jour)

2. Nom d'utilisateur: Griefer123
   Raison: Griefing
   Banni par: console
   Créé: il y a 3 jours
   Expire: Jamais (bannissement permanent)
```

**Remarques:**
- Si aucun bannissement n'est actif, vous verrez un message l'indiquant
- Le filtre de bannissement doit être activé dans votre configuration pour que cette commande fonctionne

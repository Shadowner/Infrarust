# Commandes Courantes

Cette page documente les commandes essentielles pour la gestion quotidienne de votre serveur Infrarust.

## list

Affiche tous les joueurs connectés sur tous les serveurs.

**Utilisation:**
```
> list
```

**Exemple de sortie:**
```
=== Joueurs connectés ===

Serveur hub.minecraft.example.com (2 joueurs)
  1. Steve - 192.168.1.10 (session: 7f8d3a2e-9c5b-4b1d-8a7c-3d2f6e9a1b5c)
  2. Alex - 192.168.1.11 (session: 6e7c2b1d-8a5e-3b9f-7c8d-2e1a4b3c5d6e)

Serveur survival.minecraft.example.com (1 joueur)
  1. Notch - 192.168.1.12 (session: 5d4c3b2a-1e9f-8d7c-6b5e-4a3c2d1e9f8a)

=== Total des joueurs (3) ===
```

## kick

Expulse un joueur du serveur.

**Utilisation:**
```
> kick <nom_utilisateur> [identifiant_serveur]
```

**Paramètres:**
- `nom_utilisateur`: Le nom d'utilisateur du joueur à expulser
- `identifiant_serveur` (facultatif): L'identifiant de configuration du serveur spécifique si plusieurs serveurs ont des joueurs avec le même nom d'utilisateur

**Exemples:**
```
> kick Steve
Joueur 'Steve' expulsé du serveur 'hub.minecraft.example.com'.

> kick Notch survival.minecraft.example.com
Joueur 'Notch' expulsé du serveur 'survival.minecraft.example.com'.
```

**Remarques:**
- Si plusieurs joueurs avec le même nom d'utilisateur existent sur différents serveurs, vous serez invité à spécifier un identifiant de serveur.

## configs

Liste toutes les configurations de serveur actuellement chargées.

**Utilisation:**
```
> configs
```

**Exemple de sortie:**
```
=== Configurations de serveur (2 au total) ===

hub.minecraft.example.com
  Domaines: hub.minecraft.example.com
  Adresses: localhost:25566
  Mode proxy: Passthrough
  Protocole proxy: Désactivé

survival.minecraft.example.com
  Domaines: survival.minecraft.example.com
  Adresses: localhost:25567
  Mode proxy: Offline
  Protocole proxy: Désactivé
```

## help

Affiche le message d'aide avec toutes les commandes disponibles.

**Utilisation:**
```
> help
```

**Exemple de sortie:**
```
=== Commandes disponibles ===

  list - Liste tous les joueurs connectés par serveur
  kick - Expulse un joueur du serveur. Utilisation: kick <nom_utilisateur> [identifiant_serveur]
  configs - Liste toutes les configurations de serveur
  ban - Bannit un joueur par IP, nom d'utilisateur ou UUID. Utilisez les options --ip/-ip, --username/-u ou --uuid/-id.
  unban - Supprime un bannissement par adresse IP, nom d'utilisateur ou UUID. Utilisez les options --ip, --username ou --uuid.
  bans - Liste tous les bannissements actifs
  debug - Affiche des informations de débogage détaillées sur les acteurs et tâches actifs
  tasks - Affiche des informations détaillées sur les tâches d'arrière-plan et leur statut
  help - Affiche ce message d'aide
  exit/quit - Quitte le programme
```

## exit/quit

Quitte le serveur Infrarust.

**Utilisation:**
```
> exit
```
ou
```
> quit
```

**Remarques:**
- Ces commandes initient un arrêt gracieux du serveur
- Tous les joueurs connectés seront déconnectés
- Les modifications de configuration seront enregistrées

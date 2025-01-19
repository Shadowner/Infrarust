# Vue d'Ensemble du Proxy

Infrarust est un proxy inverse Minecraft haute performance écrit en Rust. Cette page explique son fonctionnement et son architecture.

## Architecture Générale

```mermaid
graph TD
    A[Client Minecraft] --> B[Infrarust Proxy]
    B --> C[Server 1]
    B --> D[Server 2]
    B --> E[Server 3]
    
    subgraph "Proxy Layer"
    B
    end
    
    subgraph "Backend Servers"
    C
    D
    E
    end
```

## Composants Principaux

### 1. Gestion des Connexions

Le proxy gère trois types de connexions :

- **Entrantes** : Connexions des clients Minecraft
- **Sortantes** : Connexions vers les serveurs backend
- **Pool de connexions** : Gestion optimisée des ressources

### 2. Routage

Le routage se fait principalement sur deux critères :

- **Domaine** : Correspondance avec les patterns configurés
- **Adresse IP** : Connexion directe si configurée

```yaml
# Exemple de configuration de routage
domains:
  - "hub.minecraft.example.com" -> Server 1
  - "survival.minecraft.example.com" -> Server 2
  - "*.creative.minecraft.example.com" -> Server 3
```

### 3. Pipeline de Traitement

```mermaid
sequenceDiagram
    participant C as Client
    participant P as Proxy
    participant S as Server

    C->>P: Handshake
    P->>P: Vérification du domaine
    P->>P: Application des filtres
    P->>S: Établissement connexion
    P->>P: Configuration du mode proxy
    C->>P: Login/Status
    P->>S: Transmission
```

## Modes de Fonctionnement

### 1. Mode Passthrough

- Transmission directe des paquets
- Performances maximales
- Pas de modification des données

### 2. Mode ClientOnly

- Authentification côté client
- Vérification des sessions
- Cache des authentifications

### 3. Mode Offline

- Sans authentification
- Idéal pour les serveurs crackés
- Configuration simplifiée

## Optimisations de Performance

### 1. Cache de Status

```mermaid
graph LR
    A[Requête Status] --> B{Cache?}
    B -->|Oui| C[Retour Cache]
    B -->|Non| D[Requête Server]
    D --> E[Mise en Cache]
    E --> C
```

### 2. Connection Pooling

- Réutilisation des connexions
- Réduction de la latence
- Économie des ressources

### 3. Buffer Management

- Gestion optimisée de la mémoire
- Zero-copy quand possible
- Buffers pré-alloués

## Sécurité

### 1. Protection DDoS

Le proxy intègre plusieurs mécanismes de protection :

- Rate limiting par IP
- Filtrage des paquets
- Protection contre les flood

### 2. Filtrage IP

```yaml
security: ### NOT IMPLEMENTED  YET ###
  ip_filter:
    blacklist:
      - "1.2.3.4"
      - "10.0.0.0/8"
    whitelist:
      - "192.168.1.0/24"
```

### 3. Limitation de Taux

- Par IP
- Par connexion
- Par requête


## Flux de Données

```mermaid
graph TD
    A[Client] --> B[Handshake Handler]
    B --> C{Mode Sélection}
    C -->|Status| D[Status Handler]
    C -->|Login| E[Login Handler]
    D --> F[Cache Layer]
    E --> G[Auth Layer]
    F --> H[Backend Server]
    G --> H
```


::: tip
Consultez les [bonnes pratiques](./best-practices.md) pour une configuration optimale de votre proxy.
:::

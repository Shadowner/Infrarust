# Mode Full (Non Fonctionnel)

⚠️ **Ce mode est actuellement non fonctionnel et ne peut pas être utilisé.**

## Objectif Initial

Ce mode visait à combiner :

- Support des plugins
- Authentification complète
- Serveurs en `online_mode=true`

## Limitation Technique

```mermaid
sequenceDiagram
    participant C as Client
    participant P as Proxy
    participant S as Server
    
    C->>P: Handshake (0x00)
    C->>P: Login Start (0x00)
    P->>S: Forward des paquets

    S->>P: Encryption Request (0x01)
    P->>C: Foward Packet (0x01)
    Note over C,S: ❌ Impossible de déchiffrer <br/>le shared secret du client

    C->>P: Encryption Response (0x01)
    P->>P: Déchiffrer le shared secret
    P->>S: Foward le Encryption Response (0x01)
```

## Raison de l'Échec

Le mode Full ne peut pas fonctionner car :

1. Le serveur et le client utilisent une API externe
2. Le processus dépend d'un secret partagé chiffré
3. Le proxy ne peut pas déchiffrer et retransmettre ce secret

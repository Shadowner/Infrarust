
# Intégration Docker

Infrarust inclut une fonctionnalité d'intégration Docker robuste qui détecte et fait automatiquement le proxy des serveurs Minecraft fonctionnant dans des conteneurs Docker. Cela élimine le besoin de configuration manuelle du proxy lors de l'utilisation de serveurs Minecraft conteneurisés.

## Aperçu

Le fournisseur Docker dans Infrarust :
- Surveille les conteneurs Docker pour détecter les changements en temps réel (démarrage, arrêt, etc.)
- Crée automatiquement des configurations de proxy pour les conteneurs Minecraft
- Prend en charge les domaines personnalisés, les mappages de ports et les modes de proxy
- Fonctionne avec les réseaux bridge et les liaisons de ports

## Configuration

Pour activer l'intégration Docker, ajoutez ce qui suit à votre `config.yaml` :

```yaml
docker_provider:
  docker_host: "unix:///var/run/docker.sock"  # Socket du démon Docker
  label_prefix: "infrarust"                   # Préfixe d'étiquette pour la configuration du conteneur
  polling_interval: 10                        # Intervalle de sondage de secours (secondes)
  watch: true                                 # Activer la surveillance des conteneurs en temps réel
  default_domains: ["docker.local"]           # Suffixe de domaine par défaut pour les conteneurs
```

### Options de configuration

| Option | Description | Valeur par défaut |
|--------|-------------|---------|
| `docker_host` | Socket/URL du démon Docker | `unix:///var/run/docker.sock` |
| `label_prefix` | Préfixe pour les étiquettes Docker | `infrarust` |
| `polling_interval` | Intervalle de sondage | `10` |
| `watch` | Surveiller les changements de conteneurs | `true` |
| `default_domains` | Suffixes de domaine par défaut | `[]` |

### Types de connexion

- **Socket Unix** : `unix:///var/run/docker.sock` (par défaut sous Linux)
- **TCP** : `tcp://localhost:2375` (démon Docker distant)

## Configuration des conteneurs

Infrarust utilise des étiquettes Docker pour configurer le proxy pour les conteneurs. Appliquez ces étiquettes à vos conteneurs Minecraft :

### Étiquettes de base

```yaml
labels:
  # Activer le proxy Infrarust (requis)
  infrarust.enable: true

  # Noms de domaine (séparés par des virgules)
  infrarust.domains: mc.example.com,mc-alt.example.com

  # Port du serveur Minecraft à l'intérieur du conteneur
  infrarust.port: 25565
```

### Étiquettes avancées

```yaml
labels:
  # Mode proxy (passthrough, offline, client_only, server_only)
  infrarust.proxy_mode: passthrough

  # Activer le protocole PROXY
  infrarust.proxy_protocol: true

  # Adresse cible personnalisée (remplace la détection automatique)
  infrarust.address: custom-host:25565
```

## Découverte des conteneurs

Infrarust détermine automatiquement la meilleure adresse à utiliser pour se connecter aux conteneurs :

1. Essaie d'abord les adresses IP des conteneurs à partir des réseaux Docker
2. Se replie sur les liaisons de port si aucune IP réseau utilisable n'est trouvée
3. Utilise enfin le nom du conteneur comme nom d'hôte si rien d'autre ne fonctionne

## Exemple Docker Compose

Voici un exemple de fichier Docker Compose avec configuration du proxy Infrarust :

```yaml
version: '3'
services:
  minecraft:
    image: itzg/minecraft-server
    ports:
      - "25565:25565"
    environment:
      EULA: "TRUE"
      MEMORY: "2G"
      TYPE: "PAPER"
      VERSION: "1.19.2"
    volumes:
      - minecraft_data:/data
    labels:
      infrarust.enable: "true"
      infrarust.domains: "mc.example.com,survival.example.com"
      infrarust.port: "25565"
      infrarust.proxy_mode: "passthrough"

  infrarust:
    image: shadowner/infrarust
    ports:
      - "25565:25565"
    volumes:
      - ./config:/config
      - /var/run/docker.sock:/var/run/docker.sock:ro
    depends_on:
      - minecraft

volumes:
  minecraft_data:
```

## Utilisation des noms de réseau

Avec un réseau Docker personnalisé, vos conteneurs peuvent se référencer les uns aux autres par leur nom :

```yaml
version: '3'
services:
  minecraft:
    # ... autre configuration ...
    networks:
      - minecraft_network
    labels:
      infrarust.enable: "true"
      infrarust.domains: "mc.example.com"

  infrarust:
    # ... autre configuration ...
    networks:
      - minecraft_network
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock:ro

networks:
  minecraft_network:
    driver: bridge
```

## Configuration multi-serveurs

Vous pouvez exécuter plusieurs serveurs Minecraft avec différents domaines :

```yaml
version: '3'
services:
  survival:
    image: itzg/minecraft-server
    # ... autre configuration ...
    labels:
      infrarust.enable: "true"
      infrarust.domains: "survival.mc.example.com"

  creative:
    image: itzg/minecraft-server
    # ... autre configuration ...
    labels:
      infrarust.enable: "true"
      infrarust.domains: "creative.mc.example.com"

  infrarust:
    # ... configuration standard d'infrarust ...
```

## Génération automatique de domaines

Si aucun domaine n'est spécifié via l'étiquette `infrarust.domains`, Infrarust génère automatiquement des noms de domaine :

1. Si `default_domains` est vide : Utilise `nomduconteneur.docker.local`
2. Si `default_domains` est défini : Utilise `nomduconteneur.votredomaine.com` pour chaque domaine dans la liste

## Réseaux de conteneurs

Si Infrarust et les conteneurs Minecraft sont sur différents réseaux :

1. Ajoutez les deux à un réseau partagé ou
2. Exposez les ports Minecraft et utilisez des liaisons de port

## Considérations de sécurité

Lorsque vous montez le socket Docker, vous donnez à Infrarust un accès à votre démon Docker. Considérez :

1. Utiliser un accès en lecture seule : `/var/run/docker.sock:/var/run/docker.sock:ro`
2. Exécuter Infrarust avec des permissions minimales
3. Dans les environnements de production, envisagez d'utiliser l'API Docker avec authentification TLS

## Optimisation des performances

Pour les grandes installations avec de nombreux conteneurs :

```yaml
docker_provider:
  polling_interval: 30  # Augmenter l'intervalle de sondage
```

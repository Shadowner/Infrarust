# Guide d'Installation

Ce guide détaille toutes les méthodes d'installation d'Infrarust pour différents systèmes d'exploitation et environnements.

## Table des Matières

[[toc]]

## Prérequis Système

### Matériel Minimum

- CPU : 1 cœur
- RAM : 256 Mo
- Stockage : 100 Mo

### Matériel Recommandé

- CPU : 2 cœurs ou plus
- RAM : 1 Go ou plus
- Stockage : 250 Mo

### Logiciels Requis

- Rust 1.80+
- Git (pour l'installation depuis les sources)
- Un système d'exploitation compatible :
  - Linux (kernel 3.17+)
  - Windows 10/11
  - macOS 10.15+

## Installation via Cargo

La méthode la plus simple pour installer Infrarust est d'utiliser Cargo, le gestionnaire de paquets de Rust.

### 1. Installation de Rust et Cargo

```bash
# Sur Linux et macOS
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Sur Windows
# Téléchargez et exécutez rustup-init.exe depuis https://rustup.rs/
```

### 2. Installation d'Infrarust

```bash
cargo install infrarust
```

## Installation depuis les Sources

Cette méthode permet d'avoir la version la plus récente et de personnaliser la compilation.

### 1. Cloner le Dépôt

```bash
git clone https://github.com/shadowner/infrarust
cd infrarust
```

### 2. Compilation

```bash
# Compilation en mode release
cargo build --release

# L'exécutable se trouve dans
# target/release/infrarust
```

## Installation via Binaires Précompilés

### Linux

```bash
# Téléchargement
curl -LO https://github.com/shadowner/infrarust/releases/latest/download/infrarust-linux-x86_64.tar.gz

# Extraction
tar xzf infrarust-linux-x86_64.tar.gz

# Déplacement dans le PATH
sudo mv infrarust /usr/local/bin/
```

### Windows

1. Téléchargez le fichier ZIP depuis la [page des releases](https://github.com/shadowner/infrarust/releases)
2. Extrayez le contenu
3. Ajoutez le dossier au PATH système ou utilisez le chemin complet

### macOS

```bash
# Téléchargement
curl -LO https://github.com/shadowner/infrarust/releases/latest/download/infrarust-macos-x86_64.tar.gz

# Extraction
tar xzf infrarust-macos-x86_64.tar.gz

# Déplacement dans le PATH
sudo mv infrarust /usr/local/bin/
```

## Installation via Docker

### Utilisation de l'Image Officielle

```bash
docker pull shadowner/infrarust:latest
```

### Docker Compose

```yaml
version: "3.8"

services:
  infrarust:
    image: shadowner/infrarust:latest
    container_name: infrarust
    restart: always
    ports:
      - "25565:25565"
    volumes:
      - ./config.yaml:/etc/infrarust/config.yaml
      - ./proxies:/etc/infrarust/proxies
```

## Installation pour le Développement

Si vous souhaitez contribuer au développement :

```bash
# Cloner avec les sous-modules
git clone --recursive https://github.com/shadowner/infrarust
cd infrarust

# Compilation en mode développement
cargo build

# Lancer les tests
cargo test
```

## Configurations Post-Installation

### Linux : Service Systemd

Créez un fichier service :

```ini
# /etc/systemd/system/infrarust.service
[Unit]
Description=Infrarust Minecraft Proxy
After=network.target

[Service]
Type=simple
User=minecraft
ExecStart=/usr/local/bin/infrarust
WorkingDirectory=/opt/infrarust
Restart=always

[Install]
WantedBy=multi-user.target
```

Activation du service :

```bash
sudo systemctl enable infrarust
sudo systemctl start infrarust
```

### Windows : Service Windows

Utilisez NSSM pour créer un service Windows :

```powershell
nssm install Infrarust "C:\Path\To\infrarust.exe"
nssm set Infrarust AppDirectory "C:\Path\To\WorkingDirectory"
nssm start Infrarust
```

## Résolution des Problèmes

### Erreurs Communes

1. **Erreur de Compilation**

   ```
   Solution : Mettez à jour Rust avec 'rustup update'
   ```

2. **Port déjà utilisé**

   ```
   Solution : Changez le port dans config.yaml ou libérez le port 25565
   ```

3. **Permissions insuffisantes**

   ```
   Solution : Exécutez avec sudo ou en tant qu'administrateur
   ```

## Mise à Jour

### Via Cargo

```bash
cargo install infrarust --force
```

### Depuis les Sources

```bash
git pull
cargo build --release
```

### Via Docker

```bash
docker pull shadowner/infrarust:latest
```

::: tip
Pour les environnements de production, il est recommandé d'utiliser une version spécifique plutôt que latest.
:::

## Support

Si vous rencontrez des problèmes lors de l'installation :

1. Consultez les [problèmes connus](https://github.com/shadowner/infrarust/issues)
2. Rejoignez notre [Discord](https://discord.gg/sqbJhZVSgG)
3. Ouvrez un ticket sur GitHub

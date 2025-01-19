# Contribuer

N'hésitez pas à ajouter ou modifier le code source. Sur GitHub, la meilleure façon de le faire est de forker ce dépôt, puis de cloner votre fork avec Git sur votre système local. Après avoir ajouté ou modifié le code source, poussez-le vers votre fork et ouvrez une pull request dans ce dépôt.

## Outils Requis

- Développement
  - [Rust](https://www.rust-lang.org/tools/install) (dernière version stable)
  - [Cargo](https://doc.rust-lang.org/cargo/) (fourni avec Rust)
  - [rustfmt](https://github.com/rust-lang/rustfmt) (pour le formatage du code)
  - [clippy](https://github.com/rust-lang/rust-clippy) (pour l'analyse statique)
- Outils Optionnels
  - [Docker](https://www.docker.com/get-started/) (pour la conteneurisation)
  - [rust-analyzer](https://rust-analyzer.github.io/) (plugin IDE recommandé)

## Structure du Projet

```C#
rust/
├── src/
│   ├── bin/           # Exécutables binaires
│   ├── core/          # Fonctionnalités principales
│   ├── network/       # Code réseau
│   ├── protocol/      # Implémentation du protocole Minecraft
│   └── proxy_modes/   # Implémentations des différents modes de proxy
├── tests/             # Tests d'intégration // TODO
├── Cargo.toml         # Dépendances et métadonnées du projet
└── Cargo.lock         # Verrouillage des dépendances
```

> Plus de détails seront ajoutés dans la documentation du site web

## Style de Code

- Suivez le [Guide de Style Rust](https://rust-lang.github.io/api-guidelines/) officiel
- Utilisez `cargo fmt` avant de commiter pour assurer un formatage cohérent
- Exécutez `cargo clippy` pour détecter les erreurs courantes et améliorer la qualité du code

## Messages de Commit

Lorsque vous contribuez à ce projet, veuillez suivre la spécification [Conventional Commits](https://www.conventionalcommits.org/fr/v1.0.0/).

Exemples en franças :

- `feat: ajout du support pour la version de protocole 1.19.4`
- `fix: gestion correcte du seuil de compression`
- `docs: mise à jour du README avec les nouvelles options de configuration`
- `test: ajout de tests unitaires pour la gestion des paquets`

> Plus d'exemples ici <https://www.conventionalcommits.org/fr/v1.0.0/#exemples>

:::warning Attention
Bien que le projet soit un projet français, pour permettre a plus de personne de participer merci de rédiger vos messages de commits en **Anglais**
:::

## Compilation et Tests

```bash
# Compiler le projet
cargo build

# Exécuter les tests
cargo test

# Exécuter avec des fonctionnalités spécifiques
cargo run --bin infrarust -- --config-path custom_config.yaml --proxies-path proxies_path_foler
```

## Versionnement

Nous suivons le [Versionnement Sémantique](https://semver.org/lang/fr/) :

- Version MAJEURE (MAJOR) pour des changements d'API incompatibles
- Version MINEURE (MINOR) pour l'ajout de fonctionnalités compatibles
- Version CORRECTIVE (PATCH) pour les corrections de bugs compatibles

# Feuille de Route Infrarust

<style>
.feature-list {
  padding-left: 1.5rem;
  margin-bottom: 2rem;
}

.phase-badge {
  display: inline-block;
  padding: 0.25rem 0.75rem;
  border-radius: 1rem;
  font-size: 0.875rem;
  font-weight: 500;
  margin-right: 0.5rem;
}

.phase-badge.current {
  background: var(--vp-c-brand-1);
  color: var(--vp-c-soft-3);
}

.phase-badge.next {
  background: var(--vp-c-default-1);
}

.completed {
  color: var(--vp-c-green);
}
</style>

::: info Statut Actuel
Infrarust est en développement actif. Cette feuille de route présente nos fonctionnalités et améliorations planifiées.
:::

# Phases de Développement

## <span class="phase-badge current">Actuelle</span> Phase 0 : Fondation

<div class="feature-list completed">

- ✅ Proxy inverse basique avec routage par nom d'hôte
- ✅ Intégration OpenTelemetry
- ✅ Modèles de tableau de bord Grafana
- ✅ Mode passthrough indépendant du protocole
- ✅ Configuration rechargeable à chaud
- ✅ Fournisseur de configuration modulaire

</div>

## <span class="phase-badge next">Suivante</span> Phase 1 : Refactorisation de l'Architecture

<div class="feature-list">

### Intégration Valence

- 🔄 Adaptation de la gestion des protocoles
- 🔄 Système de gestion des paquets
- 🔄 Machine à états des connexions

### Architecture Multi-Crates

- 📦 Modularisation des fonctionnalités
- 📦 Optimisations spécifiques aux plateformes
- 📦 Modèles d'architecture propre

</div>

## Phase 2 : Données & Configuration

<div class="feature-list">

### Couche de Stockage

- 💾 SQLx Asynchrone (PostgreSQL/SQLite)
- 💾 Versionnement des schémas (Refinery)
- 💾 Intégration du cache Redis

### Configuration Améliorée

- 🔐 Gestion des secrets
- 🔐 Identifiants chiffrés
- 🔐 Stockage sécurisé des clés

</div>

## Phase 3 : Framework de Commandes

<div class="feature-list">

### Contrôle d'Accès

- 👥 Implémentation RBAC
- 👥 Hiérarchie des permissions
- 👥 Configuration style Minecraft

### Fonctionnalités de Gestion

- 🎮 Interface REPL
- 🎮 Gestion des états
- 🎮 Système de webhooks
- 🎮 Journalisation d'audit

</div>

## Phase 4 : Équilibrage de Charge Avancé

<div class="feature-list">

### Distribution de Charge

- ⚖️ Round-robin pondéré
- ⚖️ Équilibrage basé sur les connexions
- ⚖️ Routage sensible à la latence
- ⚖️ Persistance des sessions

### Haute Disponibilité

- 🔄 Regroupement de serveurs
- 🔄 Basculement automatique
- 🔄 Arrêt progressif
- 🔄 Clustering multi-proxy

</div>

## Phase 5 : Interface d'Administration

<div class="feature-list">

### API Backend

- 🔌 Points de terminaison RESTful
- 🔌 Mises à jour en temps réel
- 🔌 Agrégation des métriques

### Suite de Surveillance

- 📊 Visualisation du trafic
- 📊 Configuration des alertes
- 📊 Analyses de performance

### Panneau de Contrôle

- 🎛️ Authentification JWT
- 🎛️ Interface de configuration
- 🎛️ Surveillance des connexions

</div>

## Phase 6 : Architecture des Plugins

<div class="feature-list">

### Système Central

- 🧩 Runtime WASM
- 🧩 Macros derive pour plugins
- 🧩 Chargement dynamique

### Intégration

- 🔌 Interception des paquets
- 🔌 Compatibilité des plateformes
- 🔌 Système d'événements

</div>

## Phase 7 : Optimisations Réseau

<div class="feature-list">

### Support des Protocoles

- 🌐 Traduction des versions (1.8→1.20+)
- 🌐 Implémentation QUIC
- 🌐 Optimisation zero-copy

### Fonctionnalités de Sécurité

- 🛡️ Intégration BungeeGuard
- 🛡️ Limitation de débit avancée
- 🛡️ Protection DDoS

</div>

::: warning Considérations Futures
Les fonctionnalités suivantes sont en cours d'évaluation mais ne sont pas actuellement dans la feuille de route :

## Système d'Authentification

- 🔒 Gestion personnalisée des sessions
- 🔒 Support du mode hors ligne
- 🔒 Intégration tierce
:::

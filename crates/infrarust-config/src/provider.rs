//! Trait abstraction for configuration sources.

use tokio::sync::mpsc;

use crate::error::ConfigError;
use crate::server::ServerConfig;

/// Source de configuration des serveurs.
///
/// Implémenté par `FileProvider`, `DockerProvider`, etc.
/// Les implémentations concrètes vivent hors de ce crate
/// (elles tirent des dépendances lourdes comme `notify` ou `bollard`).
pub trait ConfigProvider: Send + Sync {
    /// Charge toutes les configurations serveurs.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if the configuration source cannot be read
    /// or the configuration data is invalid.
    fn load_configs(&self) -> Result<Vec<ServerConfig>, ConfigError>;

    /// S'abonne aux changements de configuration.
    ///
    /// Retourne `Some(receiver)` si le provider supporte le hot-reload,
    /// `None` sinon. Le receiver émet des `ConfigChange` au fil du temps.
    fn watch(&self) -> Option<mpsc::Receiver<ConfigChange>>;
}

/// Un changement de configuration détecté par un provider.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ConfigChange {
    /// Un nouveau serveur a été ajouté.
    Added(ServerConfig),
    /// Un serveur existant a été modifié.
    Updated { id: String, config: ServerConfig },
    /// Un serveur a été supprimé.
    Removed { id: String },
    /// Rechargement complet (tous les serveurs).
    /// Utilisé quand le provider ne peut pas calculer un diff.
    FullReload(Vec<ServerConfig>),
}

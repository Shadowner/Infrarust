//! Backend server configuration (one per `.toml` file in `servers_dir`).

use serde::Deserialize;

use crate::types::{
    DomainRewrite, IpFilterConfig, MotdConfig, ProxyMode, ServerAddress, ServerManagerConfig,
    TimeoutConfig,
};

/// Configuration d'un serveur backend Minecraft.
/// Chaque fichier dans `servers_dir/` désérialise vers ce type.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    /// Identifiant unique. Dérivé du nom de fichier si absent.
    #[serde(default)]
    pub id: Option<String>,

    /// Domaines qui routent vers ce serveur.
    /// Supporte les wildcards : "*.mc.example.com"
    pub domains: Vec<String>,

    /// Adresses du backend (host:port). Plusieurs = load balancing futur.
    pub addresses: Vec<ServerAddress>,

    /// Mode de proxy pour ce serveur
    #[serde(default)]
    pub proxy_mode: ProxyMode,

    /// Envoyer le proxy protocol au backend
    #[serde(default)]
    pub send_proxy_protocol: bool,

    /// Réécriture du domaine dans le handshake
    #[serde(default)]
    pub domain_rewrite: DomainRewrite,

    /// MOTD par état du serveur
    #[serde(default)]
    pub motd: MotdConfig,

    /// Gestion automatique du serveur (start/stop)
    #[serde(default)]
    pub server_manager: Option<ServerManagerConfig>,

    /// Timeouts spécifiques (override global)
    #[serde(default)]
    pub timeouts: Option<TimeoutConfig>,

    /// Nombre max de joueurs (0 = illimité)
    #[serde(default)]
    pub max_players: u32,

    /// Filtres IP spécifiques
    #[serde(default)]
    pub ip_filter: Option<IpFilterConfig>,

    /// Message de déconnexion envoyé au joueur quand le backend est injoignable.
    #[serde(default)]
    pub disconnect_message: Option<String>,
}

impl ServerConfig {
    /// Returns the effective identifier for this config.
    ///
    /// If `id` is `None`, returns `"unknown"`. In practice the `FileProvider`
    /// sets `id` from the filename (without extension) before handing the
    /// config to the rest of the system.
    pub fn effective_id(&self) -> String {
        self.id.clone().unwrap_or_else(|| "unknown".to_string())
    }

    /// Returns the disconnect message for when the backend is unreachable.
    pub fn effective_disconnect_message(&self) -> &str {
        self.disconnect_message
            .as_deref()
            .unwrap_or("Server is currently unreachable. Please try again later.")
    }
}

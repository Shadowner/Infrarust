//! Fundamental types: enums, value objects, and shared configuration structs.

use std::fmt;
use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;

use ipnet::IpNet;
use serde::Deserialize;

use crate::defaults;
use crate::error::ConfigError;

/// Port Minecraft par défaut.
pub const DEFAULT_MC_PORT: u16 = 25565;

// ─────────────────────────── Proxy Mode ───────────────────────────

/// Les modes de proxy supportés.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProxyMode {
    /// Forward brut via `tokio::io::copy_bidirectional`.
    #[default]
    Passthrough,
    /// Forward brut via `splice(2)` sur Linux.
    ZeroCopy,
    /// Auth Mojang côté proxy, backend en `online_mode=false`.
    ClientOnly,
    /// Pas d'auth, relais transparent.
    Offline,
    /// Auth gérée par le backend.
    ServerOnly,
    /// Chiffrement des deux côtés (nouveau V2).
    Full,
}

impl ProxyMode {
    /// `true` si le proxy parse les paquets au-delà du handshake.
    pub const fn is_intercepted(&self) -> bool {
        matches!(self, Self::ClientOnly | Self::Offline | Self::Full)
    }

    /// `true` si le proxy fait du forward brut après le handshake.
    pub const fn is_forwarding(&self) -> bool {
        matches!(self, Self::Passthrough | Self::ZeroCopy | Self::ServerOnly)
    }
}

// ─────────────────────────── Server Address ───────────────────────

/// Adresse d'un serveur backend.
///
/// Se désérialise depuis une string `"host:port"` ou `"host"` (port par défaut = 25565).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServerAddress {
    pub host: String,
    pub port: u16,
}

impl FromStr for ServerAddress {
    type Err = ConfigError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Tente de parser comme SocketAddr d'abord (IP:port)
        if let Ok(sock) = s.parse::<SocketAddr>() {
            return Ok(Self {
                host: sock.ip().to_string(),
                port: sock.port(),
            });
        }

        // Sinon, split sur le dernier ':'
        if let Some((host, port_str)) = s.rsplit_once(':')
            && let Ok(port) = port_str.parse::<u16>()
        {
            return Ok(Self {
                host: host.to_string(),
                port,
            });
        }

        // Pas de port → défaut 25565
        if s.is_empty() {
            return Err(ConfigError::InvalidAddress(s.to_string()));
        }

        Ok(Self {
            host: s.to_string(),
            port: DEFAULT_MC_PORT,
        })
    }
}

impl fmt::Display for ServerAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

/// Désérialisation serde depuis un string.
impl<'de> Deserialize<'de> for ServerAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

// ─────────────────────────── Domain Rewrite ───────────────────────

/// Comment réécrire le domaine dans le handshake Minecraft
/// avant de le transmettre au backend.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DomainRewrite {
    /// Pas de réécriture — le domaine original est transmis tel quel.
    #[default]
    None,
    /// Réécrire avec un domaine explicite.
    Explicit(String),
    /// Extraire le domaine depuis l'adresse du premier backend.
    FromBackend,
}

// ─────────────────────────── Rate Limit ───────────────────────────

/// Configuration du rate limiting.
#[derive(Debug, Clone, Deserialize)]
pub struct RateLimitConfig {
    /// Connexions login max par IP par fenêtre.
    #[serde(default = "defaults::rate_limit_max")]
    pub max_connections: u32,

    /// Durée de la fenêtre pour les logins.
    #[serde(default = "defaults::rate_limit_window")]
    #[serde(with = "humantime_serde")]
    pub window: Duration,

    /// Limite séparée pour les status pings (plus permissif).
    #[serde(default = "defaults::rate_limit_status_max")]
    pub status_max: u32,

    /// Durée de la fenêtre pour les status pings.
    #[serde(default = "defaults::rate_limit_status_window")]
    #[serde(with = "humantime_serde")]
    pub status_window: Duration,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_connections: defaults::rate_limit_max(),
            window: defaults::rate_limit_window(),
            status_max: defaults::rate_limit_status_max(),
            status_window: defaults::rate_limit_status_window(),
        }
    }
}

// ─────────────────────────── Status Cache ─────────────────────────

/// Configuration du cache de status ping.
#[derive(Debug, Clone, Deserialize)]
pub struct StatusCacheConfig {
    /// Durée de vie d'une entrée en cache.
    #[serde(default = "defaults::status_cache_ttl")]
    #[serde(with = "humantime_serde")]
    pub ttl: Duration,

    /// Nombre max d'entrées.
    #[serde(default = "defaults::status_cache_max_entries")]
    pub max_entries: usize,
}

impl Default for StatusCacheConfig {
    fn default() -> Self {
        Self {
            ttl: defaults::status_cache_ttl(),
            max_entries: defaults::status_cache_max_entries(),
        }
    }
}

// ─────────────────────────── MOTD ─────────────────────────────────

/// MOTD par état du serveur.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MotdConfig {
    pub online: Option<MotdEntry>,
    pub offline: Option<MotdEntry>,
    pub sleeping: Option<MotdEntry>,
    pub starting: Option<MotdEntry>,
    pub crashed: Option<MotdEntry>,
    pub unreachable: Option<MotdEntry>,
}

/// Une entrée MOTD.
#[derive(Debug, Clone, Deserialize)]
pub struct MotdEntry {
    /// Texte du MOTD (supporte les codes Minecraft §).
    pub text: String,
    /// Chemin vers le favicon (PNG), base64, ou URL.
    #[serde(default)]
    pub favicon: Option<String>,
    /// Version affichée dans le client.
    #[serde(default)]
    pub version_name: Option<String>,
    /// Nombre de joueurs max affiché.
    #[serde(default)]
    pub max_players: Option<u32>,
}

// ─────────────────────────── Timeouts ─────────────────────────────

/// Timeouts spécifiques à un serveur (override global).
#[derive(Debug, Clone, Deserialize)]
pub struct TimeoutConfig {
    #[serde(default = "defaults::connect_timeout")]
    #[serde(with = "humantime_serde")]
    pub connect: Duration,

    #[serde(default = "defaults::read_timeout")]
    #[serde(with = "humantime_serde")]
    pub read: Duration,

    #[serde(default = "defaults::write_timeout")]
    #[serde(with = "humantime_serde")]
    pub write: Duration,
}

// ─────────────────────────── Keepalive ────────────────────────────

/// Configuration TCP keepalive.
///
/// Contrôle les sondes keepalive envoyées sur les connexions TCP
/// pour détecter les connexions mortes.
#[derive(Debug, Clone, Deserialize)]
pub struct KeepaliveConfig {
    /// Durée d'inactivité avant la première sonde.
    #[serde(default = "defaults::keepalive_time")]
    #[serde(with = "humantime_serde")]
    pub time: Duration,

    /// Intervalle entre les sondes.
    #[serde(default = "defaults::keepalive_interval")]
    #[serde(with = "humantime_serde")]
    pub interval: Duration,

    /// Nombre de sondes échouées avant fermeture.
    #[serde(default = "defaults::keepalive_retries")]
    pub retries: u32,
}

impl Default for KeepaliveConfig {
    fn default() -> Self {
        Self {
            time: defaults::keepalive_time(),
            interval: defaults::keepalive_interval(),
            retries: defaults::keepalive_retries(),
        }
    }
}

// ─────────────────────────── IP Filter ────────────────────────────

/// Filtrage IP par CIDR.
///
/// Si `whitelist` est non-vide, seules les IPs dans la whitelist sont autorisées.
/// Si `blacklist` est non-vide, les IPs dans la blacklist sont refusées.
/// La whitelist est évaluée en premier.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct IpFilterConfig {
    #[serde(default)]
    pub whitelist: Vec<IpNet>,
    #[serde(default)]
    pub blacklist: Vec<IpNet>,
}

impl IpFilterConfig {
    /// Vérifie si une IP est autorisée par ce filtre.
    pub fn is_allowed(&self, ip: &std::net::IpAddr) -> bool {
        if !self.whitelist.is_empty() {
            return self.whitelist.iter().any(|net| net.contains(ip));
        }
        if !self.blacklist.is_empty() {
            return !self.blacklist.iter().any(|net| net.contains(ip));
        }
        true
    }
}

// ─────────────────────────── Server Manager ───────────────────────

/// Configuration du server manager (auto start/stop).
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerManagerConfig {
    Local(LocalManagerConfig),
    Pterodactyl(PterodactylManagerConfig),
    Crafty(CraftyManagerConfig),
}

/// Provider Local : lance un processus Java local.
#[derive(Debug, Clone, Deserialize)]
pub struct LocalManagerConfig {
    /// Commande à exécuter (ex: "java")
    pub command: String,
    /// Répertoire de travail
    pub working_dir: std::path::PathBuf,
    /// Arguments (ex: `["-Xmx4G", "-jar", "server.jar", "nogui"]`)
    #[serde(default)]
    pub args: Vec<String>,
    /// Pattern dans stdout qui indique que le serveur est prêt
    #[serde(default = "defaults::ready_pattern")]
    pub ready_pattern: String,
    /// Timeout pour le shutdown graceful
    #[serde(default = "defaults::shutdown_timeout")]
    #[serde(with = "humantime_serde")]
    pub shutdown_timeout: Duration,
    /// Durée d'inactivité avant shutdown auto
    #[serde(with = "humantime_serde")]
    pub shutdown_after: Duration,
}

/// Provider Pterodactyl : API REST.
#[derive(Debug, Clone, Deserialize)]
pub struct PterodactylManagerConfig {
    pub api_url: String,
    pub api_key: String,
    pub server_id: String,
    #[serde(with = "humantime_serde")]
    pub shutdown_after: Duration,
}

/// Provider Crafty Controller : API REST.
#[derive(Debug, Clone, Deserialize)]
pub struct CraftyManagerConfig {
    pub api_url: String,
    pub api_key: String,
    pub server_id: String,
    #[serde(with = "humantime_serde")]
    pub shutdown_after: Duration,
}

// ─────────────────────────── Telemetry ────────────────────────────

/// Configuration de la télémétrie OpenTelemetry.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TelemetryConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "defaults::otlp_endpoint")]
    pub otlp_endpoint: String,
    #[serde(default = "defaults::service_name")]
    pub service_name: String,
}

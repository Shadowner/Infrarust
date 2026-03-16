//! Global proxy configuration (`infrarust.toml`).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;

use crate::defaults;
use crate::types::{
    BanConfig, DockerProviderConfig, KeepaliveConfig, MotdConfig, RateLimitConfig,
    StatusCacheConfig, TelemetryConfig,
};

/// Configuration racine du proxy.
/// Correspond au fichier `infrarust.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyConfig {
    /// Adresse d'écoute, ex: "0.0.0.0:25565"
    #[serde(default = "defaults::bind")]
    pub bind: SocketAddr,

    /// Nombre max de connexions simultanées (0 = illimité)
    #[serde(default)]
    pub max_connections: u32,

    /// Timeout de connexion au backend
    #[serde(default = "defaults::connect_timeout")]
    #[serde(with = "humantime_serde")]
    pub connect_timeout: Duration,

    /// Active la réception du proxy protocol (`HAProxy` v1/v2)
    #[serde(default)]
    pub receive_proxy_protocol: bool,

    /// Chemin vers le dossier de configs serveurs
    #[serde(default = "defaults::servers_dir")]
    pub servers_dir: PathBuf,

    /// Nombre de worker threads tokio (0 = auto)
    #[serde(default)]
    pub worker_threads: usize,

    /// Config du rate limiting global
    #[serde(default)]
    pub rate_limit: RateLimitConfig,

    /// Config du cache de status ping
    #[serde(default)]
    pub status_cache: StatusCacheConfig,

    /// MOTD par défaut quand aucun serveur ne matche
    #[serde(default)]
    pub default_motd: Option<MotdConfig>,

    /// Config de la télémétrie (absent = désactivé)
    #[serde(default)]
    pub telemetry: Option<TelemetryConfig>,

    /// Config TCP keepalive
    #[serde(default)]
    pub keepalive: KeepaliveConfig,

    /// Active `SO_REUSEPORT` (Linux uniquement)
    #[serde(default)]
    pub so_reuseport: bool,

    /// Configuration du système de ban
    #[serde(default)]
    pub ban: BanConfig,

    /// Configuration du provider Docker (optionnel).
    /// Présent dans le TOML même sans la feature `docker` compilée.
    #[serde(default)]
    pub docker: Option<DockerProviderConfig>,
}

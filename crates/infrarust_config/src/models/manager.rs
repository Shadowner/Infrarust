use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct PterodactylManagerConfig {
    pub enabled: bool,
    pub base_url: String,
    pub api_key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CraftyControllerManagerConfig {
    pub enabled: bool,
    pub base_url: String,
    pub api_key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ManagerConfig {
    pub pterodactyl: Option<PterodactylManagerConfig>,
    pub crafty: Option<CraftyControllerManagerConfig>,
}

use std::io::{self, Read};
use std::time::Duration;
use std::{fs, path::Path};

use log::debug;
use serde::{de::DeserializeOwned, Deserialize};
use walkdir::WalkDir;

use crate::network::proxy_protocol::errors::ProxyProtocolError;
use crate::network::proxy_protocol::ProtocolResult;
use crate::proxy_modes::ProxyModeEnum;

#[derive(Debug, Clone, Copy)]
pub enum FileType {
    Yaml,
}

pub struct FileProvider {
    pub config_path: String,
    pub proxies_path: String,
    pub file_type: FileType,
}

impl FileProvider {
    pub fn new(config_path: String, proxies_path: String, file_type: FileType) -> Self {
        Self {
            config_path,
            proxies_path,
            file_type,
        }
    }

    pub fn load_config(&self) -> ProtocolResult<InfrarustConfig> {
        let decoder = match self.file_type {
            FileType::Yaml => yaml_decoder,
        };

        let config_path = fs::canonicalize(&self.config_path)?;
        let mut file = fs::File::open(&config_path)?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;

        let mut config: InfrarustConfig = decoder(&contents)?;
        debug!("Loaded config: {:?}", config);

        config.server_configs = self.load_server_configs()?;
        Ok(config)
    }

    fn load_server_configs(&self) -> ProtocolResult<Vec<ServerConfig>> {
        let proxies_path = fs::canonicalize(&self.proxies_path)?;
        let mut configs = Vec::new();

        for entry in WalkDir::new(&proxies_path)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path();
            let config = self.load_server_config(path)?;
            debug!("Loaded server config: {:?}", config);
            if !config.is_empty() {
                configs.push(config);
            }
        }

        Ok(configs)
    }

    fn load_server_config<P: AsRef<Path>>(&self, path: P) -> ProtocolResult<ServerConfig> {
        let decoder = match self.file_type {
            FileType::Yaml => yaml_decoder,
        };

        let mut file = fs::File::open(path)?;
        let mut contents = String::new();

        file.read_to_string(&mut contents)?;

        decoder(&contents)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub domains: Vec<String>,
    pub addresses: Vec<String>,
    #[serde(rename = "sendProxyProtocol")]
    pub send_proxy_protocol: Option<bool>,
    #[serde(rename = "proxyMode")]
    pub proxy_mode: Option<ProxyModeEnum>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            domains: Vec::new(),
            addresses: Vec::new(),
            send_proxy_protocol: Some(false),
            proxy_mode: Some(ProxyModeEnum::default()),
        }
    }
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct InfrarustConfig {
    pub bind: Option<String>,
    pub domains: Option<Vec<String>>,
    pub addresses: Option<Vec<String>>,
    pub keepalive_timeout: Option<Duration>,

    #[serde(skip)]
    pub server_configs: Vec<ServerConfig>,
}

impl ServerConfig {
    pub fn is_empty(&self) -> bool {
        self.domains.is_empty() && self.addresses.is_empty()
    }
}

impl InfrarustConfig {
    pub fn is_empty(&self) -> bool {
        self.bind.is_none() && self.domains.is_none() && self.addresses.is_none()
    }
}

fn yaml_decoder<T: DeserializeOwned>(content: &str) -> ProtocolResult<T> {
    debug!("Decoding YAML content: {}", content);
    serde_yaml::from_str(content)
        .map_err(|e| ProxyProtocolError::Io(io::Error::new(io::ErrorKind::InvalidData, e)))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_file_provider() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yml");
        let proxies_path = temp_dir.path().join("proxies");

        fs::create_dir(&proxies_path).unwrap();

        fs::write(&config_path, "bind: ':25565'\n").unwrap();
        fs::write(
            proxies_path.join("server1.yml"),
            "domains: ['example.com']\naddresses: ['127.0.0.1:25566']\n",
        )
        .unwrap();

        let provider = FileProvider::new(
            config_path.to_str().unwrap().to_string(),
            proxies_path.to_str().unwrap().to_string(),
            FileType::Yaml,
        );

        let config = provider.load_config().unwrap();
        assert!(!config.server_configs.is_empty());
    }
}

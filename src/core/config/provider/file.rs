use std::{
    collections::HashMap,
    fs,
    io::{self, Read},
    path::Path,
};

use log::debug;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::de::DeserializeOwned;
use tokio::sync::mpsc::{self, Sender};
use walkdir::WalkDir;

use crate::{
    core::{
        config::ServerConfig,
        event::{GatewayMessage, ProviderMessage},
    },
    network::proxy_protocol::{errors::ProxyProtocolError, ProtocolResult},
    InfrarustConfig,
};

use super::Provider;

#[derive(Debug, Clone, Copy)]
pub enum FileType {
    Yaml,
}

pub struct FileProvider {
    proxies_path: Vec<String>,
    file_type: FileType,

    provider_sender: mpsc::Sender<ProviderMessage>,

    watch: bool,
}

impl FileProvider {
    pub fn new(
        proxies_path: Vec<String>,
        file_type: FileType,
        watch: bool,
        provider_sender: mpsc::Sender<ProviderMessage>,
    ) -> Self {
        Self {
            proxies_path,
            file_type,
            provider_sender,
            watch,
        }
    }

    fn load_server_configs(&self, path: String) -> ProtocolResult<HashMap<String, ServerConfig>> {
        let proxies_path = fs::canonicalize(path)?;
        let mut configs = HashMap::new();

        for entry in WalkDir::new(&proxies_path)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path();
            let config = Self::load_server_config(path, self.file_type, self.get_name())?;

            // debug!("Loaded server config: {:?}", config);
            if !config.is_empty() {
                configs.insert(config.config_id.clone(), config);
            }
        }

        Ok(configs)
    }

    fn load_server_config<P: AsRef<Path>>(
        path: P,
        file_type: FileType,
        provider_name: String,
    ) -> ProtocolResult<ServerConfig> {
        let decoder = match file_type {
            FileType::Yaml => yaml_decoder::<ServerConfig>,
        };

        let file_name = path
            .as_ref()
            .file_name()
            .unwrap_or(path.as_ref().as_os_str())
            .to_string_lossy()
            .to_string();

        let config_id = file_name + "@" + provider_name.as_str();

        let mut file = fs::File::open(path)?;
        let mut contents = String::new();

        file.read_to_string(&mut contents)?;

        let mut config = match decoder(&contents) {
            Ok(config) => config,
            Err(e) => {
                debug!("Failed to decode server config: {:?}", e);
                ServerConfig::default()
            }
        };

        config.config_id = config_id;

        Ok(config)
    }
    
    fn setup_watcher(&self, sender: Sender<ProviderMessage>) -> ProtocolResult<RecommendedWatcher> {
        debug!("Setting up file watcher");
        // Utiliser une durée de debounce pour éviter les événements multiples
        let mut last_event = std::time::Instant::now();
        let debounce_duration = std::time::Duration::from_millis(100);

        let file_type = self.file_type.clone();
        let provider_name = self.get_name();

        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| match res {
                Ok(event) => {
                    let now = std::time::Instant::now();
                    if now.duration_since(last_event) >= debounce_duration {
                        debug!("File change detected: {:?}", event);

                        for path in event.paths.iter() {
                            debug!("Path changed: {:?}", path);
                            if let Ok(config) =
                                Self::load_server_config(path, file_type, provider_name.clone())
                            {
                                let _ = sender.send(ProviderMessage::Update {
                                    key: config.config_id.clone(),
                                    configuration: config,
                                });
                            }

                            last_event = now;
                        }
                    }
                }
                Err(e) => log::error!("Watch error: {:?}", e),
            })
            .map_err(|e| ProxyProtocolError::Io(io::Error::new(io::ErrorKind::Other, e)))?;
        for path in &self.proxies_path {
            debug!("Watching path: {:?} for changes", path);
            let proxy_path = fs::canonicalize(path)?;

            watcher
                .watch(proxy_path.as_path(), RecursiveMode::Recursive)
                .map_err(|e| ProxyProtocolError::Io(io::Error::new(io::ErrorKind::Other, e)))?;
        }

        Ok(watcher)
    }
}

#[async_trait::async_trait]
impl Provider for FileProvider {
    async fn run(&mut self) {
        let (tx,mut rx) = mpsc::channel::<ProviderMessage>(32);
        let _watcher = if self.watch {
            Some(self.setup_watcher(tx).unwrap())
        } else {
            None
        };

        for path in &self.proxies_path {
            let configs = match self.load_server_configs(path.clone()) {
                Ok(configs) => configs,
                Err(e) => {
                    debug!("Failed to load server configs: {:?}", e);
                    HashMap::new()
                }
            };

            debug!("Loaded server configs: {:?}", configs);
            self
                .provider_sender
                .send(ProviderMessage::FirstInit(configs)).await.unwrap();
        }

        loop {
            if let Some(event) = rx.recv().await {
                debug!("Received event: {:?}", event);
                match event {
                    ProviderMessage::Update { key, configuration } => {
                        debug!("Configuration update received for key: {}", key);
                        let _ = self
                            .provider_sender
                            .send(ProviderMessage::Update { key, configuration });
                    }
                    ProviderMessage::Shutdown => break,
                    _ => {}
                }
            }
            // tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }
    fn get_name(&self) -> String {
        "FileProvider".to_string()
    }

    fn new(sender: mpsc::Sender<ProviderMessage>) -> Self {
        Self {
            proxies_path: Vec::new(),
            file_type: FileType::Yaml,
            provider_sender: sender,
            watch: false,
        }
    }
}

fn yaml_decoder<T: DeserializeOwned>(content: &str) -> ProtocolResult<T> {
    serde_yaml::from_str(content)
        .map_err(|e| ProxyProtocolError::Io(io::Error::new(io::ErrorKind::InvalidData, e)))
}

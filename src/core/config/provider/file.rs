use std::{
    collections::HashMap,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    thread::sleep,
    time::{Duration, Instant},
};

use log::{debug, error, warn};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::de::DeserializeOwned;
use tokio::sync::mpsc::{self, channel, Sender};
use walkdir::WalkDir;

use crate::{
    core::{config::ServerConfig, event::ProviderMessage},
    network::proxy_protocol::errors::ProxyProtocolError,
};

use super::Provider;

#[derive(Debug, Clone, Copy)]
pub enum FileType {
    Yaml,
}

// Configuration structure pour FileProvider
#[derive(Debug)]
pub struct FileProviderConfig {
    proxies_path: Vec<String>,
    file_type: FileType,
    watch: bool,
}

pub struct FileProvider {
    config: FileProviderConfig,
    provider_sender: mpsc::Sender<ProviderMessage>,
}

#[derive(Debug)]
struct FileEvent {
    path: PathBuf,
    kind: notify::EventKind,
}

impl FileProvider {
    pub fn new(
        proxies_path: Vec<String>,
        file_type: FileType,
        watch: bool,
        provider_sender: mpsc::Sender<ProviderMessage>,
    ) -> Self {
        Self {
            config: FileProviderConfig {
                proxies_path,
                file_type,
                watch,
            },
            provider_sender,
        }
    }

    fn generate_config_id(path: &Path, provider_name: &str) -> String {
        format!(
            "{}@{}",
            path.file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy(),
            provider_name
        )
    }

    async fn handle_config_update(
        path: &Path,
        file_type: FileType,
        provider_name: &str,
        sender: &Sender<ProviderMessage>,
    ) -> io::Result<()> {
        if let Ok(config) = Self::load_server_config(path, file_type, provider_name.to_string()) {
            let message = ProviderMessage::Update {
                key: config.config_id.clone(),
                configuration: if config.is_empty() {
                    None
                } else {
                    Some(config)
                },
            };

            if let Err(e) = sender.send(message).await {
                error!("Failed to send update message: {}", e);
            }
        }
        Ok(())
    }

    fn load_configs(&self, path: &str) -> io::Result<HashMap<String, ServerConfig>> {
        let proxies_path = fs::canonicalize(path)?;
        let mut configs = HashMap::new();

        for entry in WalkDir::new(&proxies_path)
            .follow_links(true)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
        {
            if let Ok(config) =
                Self::load_server_config(entry.path(), self.config.file_type, self.get_name())
            {
                if !config.is_empty() {
                    configs.insert(config.config_id.clone(), config);
                }
            }
        }

        Ok(configs)
    }

    fn load_server_config<P: AsRef<Path>>(
        path: P,
        file_type: FileType,
        provider_name: String,
    ) -> io::Result<ServerConfig> {
        let path = path.as_ref();
        let metadata = fs::metadata(path)?;

        if metadata.len() == 0 {
            debug!("File is empty: {:?}", path);
            return Err(io::Error::new(io::ErrorKind::InvalidData, "File is empty"));
        }

        let mut content = String::new();
        fs::File::open(path)?.read_to_string(&mut content)?;

        let mut config: ServerConfig = match file_type {
            FileType::Yaml => {
                yaml_decoder(&content).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
            }
        };

        config.config_id = Self::generate_config_id(path, &provider_name);
        Ok(config)
    }

    async fn setup_file_watcher(&self) -> io::Result<RecommendedWatcher> {
        debug!("Setting up file watcher");
        let (file_event_tx, mut file_event_rx) = channel::<FileEvent>(5);
        let debounce_duration = Duration::from_millis(100);

        let file_type = self.config.file_type;
        let provider_name = self.get_name();
        let provider_sender = self.provider_sender.clone();

        tokio::spawn(async move {
            let mut last_paths: HashMap<PathBuf, Instant> = HashMap::new();

            while let Some(event) = file_event_rx.recv().await {
                Self::handle_file_event(
                    event,
                    &mut last_paths,
                    debounce_duration,
                    file_type,
                    &provider_name,
                    &provider_sender,
                )
                .await;
            }
        });

        self.create_watcher(file_event_tx)
    }

    async fn handle_file_event(
        event: FileEvent,
        last_paths: &mut HashMap<PathBuf, Instant>,
        debounce_duration: Duration,
        file_type: FileType,
        provider_name: &str,
        provider_sender: &Sender<ProviderMessage>,
    ) {
        if notify::EventKind::is_remove(&event.kind) {
            let message = ProviderMessage::Update {
                key: Self::generate_config_id(&event.path, provider_name),
                configuration: None,
            };
            if let Err(e) = provider_sender.send(message).await {
                error!("Failed to send update message: {}", e);
            }
            return;
        }

        let now = Instant::now();
        if Self::should_skip_event(&event.path, now, last_paths, debounce_duration) {
            return;
        }

        last_paths.insert(event.path.clone(), now);
        last_paths.retain(|_, time| now.duration_since(*time) < debounce_duration);

        if let Err(e) =
            Self::handle_config_update(&event.path, file_type, provider_name, provider_sender).await
        {
            error!("Failed to handle config update: {}", e);
        }
    }

    fn should_skip_event(
        path: &Path,
        now: Instant,
        last_paths: &HashMap<PathBuf, Instant>,
        debounce_duration: Duration,
    ) -> bool {
        last_paths.get(path).map_or(false, |last_time| {
            now.duration_since(*last_time) < debounce_duration
        })
    }

    fn create_watcher(&self, file_event_tx: Sender<FileEvent>) -> io::Result<RecommendedWatcher> {
        let mut watcher = match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                if notify::EventKind::is_access(&event.kind)
                    || notify::EventKind::is_other(&event.kind)
                {
                    return;
                }

                if notify::EventKind::is_create(&event.kind)
                    || notify::EventKind::is_modify(&event.kind)
                {
                    //HACK: Workaround to let the file be written before reading it
                    // Yes that's an actual bug that I can't find a solution to
                    sleep(Duration::from_millis(100));
                }

                for path in event.paths {
                    if !path.is_dir() {
                        let _ = file_event_tx.blocking_send(FileEvent {
                            path,
                            kind: event.kind,
                        });
                    }
                }
            }
        }) {
            Ok(watcher) => watcher,
            Err(e) => {
                return Err(io::Error::new(io::ErrorKind::Other, e));
            }
        };

        

        for path in &self.config.proxies_path {
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

        //TODO: Implement real communication with the ConfigProvider
        let (_, mut rx) = mpsc::channel::<ProviderMessage>(32);

        let _watcher = if self.config.watch {
            Some(self.setup_file_watcher().await.unwrap())
        } else {
            None
        };

        for path in &self.config.proxies_path {
            match self.load_configs(path) {
                Ok(configs) => {
                    if let Err(e) = self
                        .provider_sender
                        .send(ProviderMessage::FirstInit(configs))
                        .await
                    {
                        warn!("Failed to send FirstInit: {}", e);
                    }
                }
                Err(e) => warn!("Failed to load server configs: {:?}", e),
            }
        }
        
        loop {
            let sender = self.provider_sender.clone();
            tokio::select! {
                () = sender.closed() => {
                    break;
                }
                else => {
                    break;
                }
            }
        }
    }

    fn get_name(&self) -> String {
        "FileProvider".to_string()
    }

    fn new(sender: mpsc::Sender<ProviderMessage>) -> Self {
        Self {
            config: FileProviderConfig {
                proxies_path: Vec::new(),
                file_type: FileType::Yaml,
                watch: false,
            },
            provider_sender: sender,
        }
    }
}

fn yaml_decoder<T: DeserializeOwned>(content: &str) -> Result<T, serde_yaml::Error> {
    serde_yaml::from_str(content)
}

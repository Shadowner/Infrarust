use std::{
    collections::HashMap,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    thread::sleep,
    time::{Duration, Instant},
};

use tracing::{debug, debug_span, error, info, instrument, warn, Instrument};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::{de::DeserializeOwned, Deserialize};
use tokio::sync::mpsc::{self, channel, Sender};
use walkdir::WalkDir;

use crate::{
    core::{config::ServerConfig, event::ProviderMessage},
    network::proxy_protocol::errors::ProxyProtocolError,
    InfrarustConfig,
};

use super::Provider;

#[derive(Debug, Clone, Copy, Deserialize, Default)]
pub enum FileType {
    // serde default value
    #[serde(rename = "yaml")]
    #[default]
    Yaml,
}

// Configuration structure pour FileProvider
#[derive(Debug, Deserialize, Clone)]
pub struct FileProviderConfig {
    #[serde(default)]
    pub proxies_path: Vec<String>,
    #[serde(default)]
    pub file_type: FileType,
    #[serde(default)]
    pub watch: bool,
}

pub struct FileProvider {
    pub paths: Vec<String>,
    pub file_type: FileType,
    pub watch: bool,
    sender: Sender<ProviderMessage>,
}

#[derive(Debug)]
struct FileEvent {
    path: PathBuf,
    kind: notify::EventKind,
}

impl FileProvider {
    pub fn try_load_config(
        path: Option<&str>,
    ) -> Result<InfrarustConfig, Box<dyn std::error::Error>> {
        // try to read the file
        let mut file = fs::File::open(path.unwrap_or("config.yaml"))?;
        let mut content = String::new();

        // read the file
        file.read_to_string(&mut content)?;

        // decode the file
        let mut default_config = InfrarustConfig::default();
        let config: InfrarustConfig = serde_yaml::from_str(&content)?;
        default_config.merge(&config);

        Ok(default_config)
    }

    #[instrument(skip(sender), fields(paths = ?paths, file_type = ?file_type, watch = watch), name = "file_provider: new")]
    pub fn new(
        paths: Vec<String>,
        file_type: FileType,
        watch: bool,
        sender: Sender<ProviderMessage>,
    ) -> Self {
        info!("Creating new file provider");
        Self {
            paths,
            file_type,
            watch,
            sender,
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
        let span = debug_span!("file_provider: handle_config_update", ?path);
        async {
            if let Ok(config) = Self::load_server_config(path, file_type, provider_name.to_string()) {
                let span = debug_span!("file_provider: send_update", key = %config.config_id.clone());
                let message = ProviderMessage::Update {
                    span: span.clone(),
                    key: config.config_id.clone(),
                    configuration: if config.is_empty() {
                        None
                    } else {
                        Some(Box::new(config))
                    },
                };
                if let Err(e) = sender.send(message).instrument(span).await {
                    error!("Failed to send update message: {}", e);
                }
            }
            Ok(())
        }.instrument(span).await
    }

    #[instrument(skip(self), fields(path = %path), name = "file_provider: load_configs")]
    fn load_configs(&self, path: &str) -> io::Result<HashMap<String, ServerConfig>> {
        debug!("Loading configurations from directory");
        let proxies_path = fs::canonicalize(path)?;
        let mut configs = HashMap::new();

        for entry in WalkDir::new(&proxies_path)
            .follow_links(true)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
        {
            if let Ok(config) =
                Self::load_server_config(entry.path(), self.file_type, self.get_name())
            {
                if !config.is_empty() {
                    configs.insert(config.config_id.clone(), config);
                }
            }
        }

        Ok(configs)
    }

    #[instrument(skip(path, file_type), fields(path = %path.as_ref().display()), name = "file_provider: load_server_config")]
    fn load_server_config<P: AsRef<Path>>(
        path: P,
        file_type: FileType,
        provider_name: String,
    ) -> io::Result<ServerConfig> {
        debug!("Loading server configuration from file");
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

    #[instrument(skip(self), name = "file_provider: setup_file_watcher")]
    async fn setup_file_watcher(&self) -> io::Result<RecommendedWatcher> {
        let span = debug_span!("setup_watcher");
        async {
            info!("Setting up file watcher");
            debug!("Setting up file watcher");
            let (file_event_tx, mut file_event_rx) = channel::<FileEvent>(5);
            let debounce_duration = Duration::from_millis(100);

            let file_type = self.file_type;
            let provider_name = self.get_name();
            let provider_sender = self.sender.clone();

            tokio::spawn(
                async move {
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
                }
            );

            self.create_watcher(file_event_tx)
        }.instrument(span).await
    }

    async fn handle_file_event(
        event: FileEvent,
        last_paths: &mut HashMap<PathBuf, Instant>,
        debounce_duration: Duration,
        file_type: FileType,
        provider_name: &str,
        provider_sender: &Sender<ProviderMessage>,
    ) {
        // Créer un nouveau span racine pour chaque événement fichier
        let root_span = debug_span!(
            "file_provider: file_change",
            path = ?event.path,
            event_kind = ?event.kind,
            provider = %provider_name
        );
        
        let span_clone = root_span.clone();
        async {
            if notify::EventKind::is_remove(&event.kind) {
                info!("File removed, sending configuration removal");
                let message = ProviderMessage::Update {
                    span: span_clone,
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
                debug!("Skipping debounced event");
                return;
            }

            last_paths.insert(event.path.clone(), now);
            last_paths.retain(|_, time| now.duration_since(*time) < debounce_duration);

            if let Err(e) =
                Self::handle_config_update(&event.path, file_type, provider_name, provider_sender)
                    .instrument(debug_span!("process_file_change"))
                    .await
            {
                error!("Failed to handle config update: {}", e);
            }
        }
        .instrument(root_span)
        .await;
    }

    fn should_skip_event(
        path: &Path,
        now: Instant,
        last_paths: &HashMap<PathBuf, Instant>,
        debounce_duration: Duration,
    ) -> bool {
        last_paths
            .get(path)
            .is_some_and(|last_time| now.duration_since(*last_time) < debounce_duration)
    }

    #[instrument(skip(self), name = "file_provider: create_watcher")]
    fn create_watcher(&self, file_event_tx: Sender<FileEvent>) -> io::Result<RecommendedWatcher> {
        let mut watcher =
            match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
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

        for path in &self.paths {
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
    #[instrument(skip(self), name = "file_provider: run", fields(paths = ?self.paths))]
    async fn run(&mut self) {
        let span = debug_span!("file_provider_run");
        async {
            info!("Starting file provider");
            //TODO: Implement real communication with the ConfigProvider

            let _watcher = if self.watch {
                Some(
                    self.setup_file_watcher()
                        .instrument(debug_span!("watcher_setup"))
                        .await
                        .expect("Failed to setup file watcher for FileProvider"),
                )
            } else {
                None
            };

            for path in &self.paths {
                match self.load_configs(path)
                {
                    Ok(configs) => {
                        if let Err(e) = self.sender.send(ProviderMessage::FirstInit(configs)).await {
                            warn!("Failed to send FirstInit: {}", e);
                        }
                    }
                    Err(e) => warn!("Failed to load server configs: {:?}", e),
                }
            }

            //HACK: This is a workaround to keep the task running
            // Until the sender is opened
            #[allow(clippy::never_loop)]
            loop {
                let sender = self.sender.clone();
                tokio::select! {
                    () = sender.closed() => {
                        break;
                    }
                    else => {
                        break;
                    }
                }
            }
        }.instrument(span).await
    }

    fn get_name(&self) -> String {
        "FileProvider".to_string()
    }

    fn new(sender: mpsc::Sender<ProviderMessage>) -> Self {
        Self {
            paths: Vec::new(),
            file_type: FileType::Yaml,
            watch: false,
            sender,
        }
    }
}

fn yaml_decoder<T: DeserializeOwned>(content: &str) -> Result<T, serde_yaml::Error> {
    serde_yaml::from_str(content)
}

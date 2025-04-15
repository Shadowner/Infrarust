use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{
        RwLock,
        atomic::{AtomicUsize, Ordering},
    },
    thread::sleep,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, de::DeserializeOwned};
use tokio::sync::mpsc::{self, Sender, channel};
use tracing::{Instrument, debug, debug_span, error, info, instrument, warn};
use walkdir::WalkDir;

use crate::{
    InfrarustConfig,
    models::{infrarust::FileType, server::ServerConfig},
    provider::{Provider, ProviderMessage},
};

use once_cell::sync::{Lazy, OnceCell};

static WATCHED_PATHS: Lazy<RwLock<HashSet<String>>> = Lazy::new(|| RwLock::new(HashSet::new()));
static WATCHER_COUNT: OnceCell<AtomicUsize> = OnceCell::new();

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
    instance_id: usize,
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
        default_config.merge(config);

        Ok(default_config)
    }

    #[instrument(skip(sender), fields(paths = ?paths, file_type = ?file_type, watch = watch), name = "file_provider: new")]
    pub fn new(
        paths: Vec<String>,
        file_type: FileType,
        watch: bool,
        sender: Sender<ProviderMessage>,
    ) -> Self {
        let mut unique_paths = Vec::new();
        for path in paths {
            if !unique_paths.contains(&path) {
                unique_paths.push(path);
            }
        }

        let instance_id = WATCHER_COUNT
            .get_or_init(|| AtomicUsize::new(0))
            .fetch_add(1, Ordering::SeqCst);
        debug!("Initialized file provider instance {}", instance_id);

        Self {
            paths: unique_paths,
            file_type,
            watch,
            sender,
            instance_id,
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
            if let Ok(config) = Self::load_server_config(path, file_type, provider_name.to_string())
            {
                let span =
                    debug_span!("file_provider: send_update", key = %config.config_id.clone());
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
        }
        .instrument(span)
        .await
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
        let span = debug_span!("setup_watcher", instance_id = self.instance_id);
        async {
            debug!("Setting up file watcher for instance {}", self.instance_id);

            let (file_event_tx, mut file_event_rx) = channel::<FileEvent>(100);

            let debounce_duration = Duration::from_millis(1000);

            let file_type = self.file_type;
            let provider_name = self.get_name();
            let provider_sender = self.sender.clone();

            tokio::spawn(async move {
                let mut last_paths: HashMap<PathBuf, Instant> = HashMap::new();
                let mut last_processed: HashMap<String, Instant> = HashMap::new();

                while let Some(event) = file_event_rx.recv().await {
                    Self::handle_file_event(
                        event,
                        &mut last_paths,
                        &mut last_processed,
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
        .instrument(span)
        .await
    }

    async fn handle_file_event(
        event: FileEvent,
        last_paths: &mut HashMap<PathBuf, Instant>,
        last_processed: &mut HashMap<String, Instant>,
        debounce_duration: Duration,
        file_type: FileType,
        provider_name: &str,
        provider_sender: &Sender<ProviderMessage>,
    ) {
        let root_span = debug_span!(
            "file_provider: file_change",
            path = ?event.path,
            event_kind = ?event.kind,
            provider = %provider_name
        );

        let span_clone = root_span.clone();
        async {
            let config_id = Self::generate_config_id(&event.path, provider_name);

            if notify::EventKind::is_remove(&event.kind) {
                debug!("File removed: {}", event.path.display());
                let message = ProviderMessage::Update {
                    span: span_clone,
                    key: config_id,
                    configuration: None,
                };
                if let Err(e) = provider_sender.send(message).await {
                    error!("Failed to send update message: {}", e);
                }
                return;
            }

            let now = Instant::now();

            if Self::should_skip_event(&event.path, now, last_paths, debounce_duration) {
                debug!(
                    "Skipping debounced event for path: {}",
                    event.path.display()
                );
                return;
            }

            if last_processed
                .get(&config_id)
                .is_some_and(|last_time| now.duration_since(*last_time) < debounce_duration)
            {
                debug!("Skipping recently processed config ID: {}", config_id);
                return;
            }

            last_paths.insert(event.path.clone(), now);
            last_processed.insert(config_id.clone(), now);

            last_paths.retain(|_, time| now.duration_since(*time) < debounce_duration * 2);
            last_processed.retain(|_, time| now.duration_since(*time) < debounce_duration * 2);

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
                        sleep(Duration::from_millis(150)); // Increased sleep time
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

        let mut watched_paths = WATCHED_PATHS.write().unwrap();

        for path in &self.paths {
            let canonical_path = match fs::canonicalize(path) {
                Ok(p) => p,
                Err(e) => {
                    warn!("Could not canonicalize path {}: {}", path, e);
                    continue;
                }
            };

            let path_str = canonical_path.to_string_lossy().to_string();

            if watched_paths.contains(&path_str) {
                debug!("Path {} is already being watched, skipping", path_str);
                continue;
            }

            info!(
                "File Provider: Watching path: {} for changes",
                canonical_path.display()
            );

            if let Err(e) = watcher.watch(&canonical_path, RecursiveMode::Recursive) {
                error!("Failed to watch path {}: {}", canonical_path.display(), e);
                continue;
            }

            watched_paths.insert(path_str);
        }

        Ok(watcher)
    }
}

#[async_trait]
impl Provider for FileProvider {
    #[instrument(skip(self), name = "file_provider: run", fields(paths = ?self.paths, instance_id = self.instance_id))]
    async fn run(&mut self) {
        let span = debug_span!("file_provider_run", instance_id = self.instance_id);
        async {
            let mut all_configs = HashMap::new();
            for path in &self.paths {
                match self.load_configs(path) {
                    Ok(configs) => {
                        for (id, config) in configs {
                            all_configs.insert(id, config);
                        }
                    }
                    Err(e) => warn!("Failed to load server configs from {}: {:?}", path, e),
                }
            }

            if !all_configs.is_empty() {
                if let Err(e) = self
                    .sender
                    .send(ProviderMessage::FirstInit(all_configs))
                    .await
                {
                    warn!("Failed to send FirstInit: {}", e);
                }

                tokio::time::sleep(Duration::from_millis(200)).await;
            }

            let _watcher = if self.watch {
                match self
                    .setup_file_watcher()
                    .instrument(debug_span!("watcher_setup", instance_id = self.instance_id))
                    .await
                {
                    Ok(watcher) => Some(watcher),
                    Err(e) => {
                        error!(
                            "Failed to setup file watcher for instance {}: {}",
                            self.instance_id, e
                        );
                        None
                    }
                }
            } else {
                None
            };

            // Block indefinitely to keep the watcher alive
            loop {
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
        }
        .instrument(span)
        .await
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
            instance_id: WATCHER_COUNT
                .get_or_init(|| AtomicUsize::new(0))
                .fetch_add(1, Ordering::SeqCst),
        }
    }
}

fn yaml_decoder<T: DeserializeOwned>(content: &str) -> Result<T, serde_yaml::Error> {
    serde_yaml::from_str(content)
}

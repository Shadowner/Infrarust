//! [`ConfigService`] implementation — access to proxy configuration.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use infrarust_api::services::config_service::{
    ConfigService, ConfigWriteError, ProxyMode, ServerConfig, ServerSource,
};
use infrarust_api::types::{ServerAddress, ServerId};
use infrarust_config::ProxyConfig;
use infrarust_config::secrets::{PROXY_SECRETS, SERVER_SECRETS, redact, reinject, still_redacted};
use toml_edit::DocumentMut;

use crate::routing::DomainRouter;

/// Provider types owned by a plugin are prefixed with `plugin:<id>:`.
const PLUGIN_PROVIDER_PREFIX: &str = "plugin:";

/// Wrapper around the proxy's configuration file and routing tables.
pub struct ConfigServiceImpl {
    router: Arc<DomainRouter>,
    config_path: PathBuf,
    config: Arc<ProxyConfig>,
    /// Serializes the read-modify-write of the configuration file, so a
    /// concurrent writer cannot land between another's read of the stored
    /// secrets and its rename.
    write_lock: std::sync::Mutex<()>,
}

impl ConfigServiceImpl {
    pub fn new(router: Arc<DomainRouter>, config_path: PathBuf, config: Arc<ProxyConfig>) -> Self {
        Self {
            router,
            config_path,
            config,
            write_lock: std::sync::Mutex::new(()),
        }
    }

    /// The configuration file as it currently is on disk.
    fn stored_document(&self) -> Option<DocumentMut> {
        let text = std::fs::read_to_string(&self.config_path).ok()?;
        text.parse::<DocumentMut>()
            .inspect_err(|e| {
                tracing::error!(
                    path = %self.config_path.display(),
                    error = %e,
                    "the proxy configuration file is not valid TOML"
                );
            })
            .ok()
    }

    /// The configuration the proxy is running on: the file it was started
    /// with, CLI overrides applied and defaults filled in.
    fn running_document(&self) -> DocumentMut {
        toml::to_string_pretty(self.config.as_ref())
            .ok()
            .and_then(|text| text.parse::<DocumentMut>().ok())
            .unwrap_or_default()
    }

    /// The configuration as it stands, falling back to what the proxy was
    /// started with when the file is gone or unparsable. Both callers depend
    /// on the fallback still carrying the secrets, so a write cannot drop a
    /// key just because the file went missing.
    fn current_document(&self) -> DocumentMut {
        self.stored_document()
            .unwrap_or_else(|| self.running_document())
    }

    /// Converts an internal [`infrarust_config::ServerConfig`] to an API [`ServerConfig`].
    fn convert_config(id: &str, config: &infrarust_config::ServerConfig) -> ServerConfig {
        ServerConfig::new(
            ServerId::new(id),
            config.network.clone(),
            config
                .addresses
                .iter()
                .map(|a| ServerAddress {
                    host: a.address.host.clone(),
                    port: a.address.port,
                })
                .collect(),
            config.domains.clone(),
            convert_proxy_mode(config.proxy_mode),
            config.limbo_handlers.clone(),
            config.max_players,
            config.disconnect_message.clone(),
            config.send_proxy_protocol,
            config.server_manager.is_some(),
        )
    }
}

impl infrarust_api::services::config_service::private::Sealed for ConfigServiceImpl {}

impl ConfigService for ConfigServiceImpl {
    fn get_server_config(&self, server: &ServerId) -> Option<ServerConfig> {
        let server_id = server.as_str();
        self.router
            .find_by_server_id(server_id)
            .map(|cfg| Self::convert_config(server_id, &cfg))
    }

    fn get_all_server_configs(&self) -> Vec<ServerConfig> {
        self.router
            .list_all()
            .into_iter()
            .map(|(_, cfg)| {
                let id = cfg.effective_id();
                Self::convert_config(&id, &cfg)
            })
            .collect()
    }

    fn get_server_document(&self, server: &ServerId) -> Option<String> {
        let config = self.router.find_by_server_id(server.as_str())?;
        let document = toml::to_string_pretty(config.as_ref())
            .inspect_err(|e| {
                tracing::error!(server = %server.as_str(), error = %e, "failed to serialize server config");
            })
            .ok()?;
        let mut document = document
            .parse::<DocumentMut>()
            .inspect_err(|e| {
                tracing::error!(server = %server.as_str(), error = %e, "serialized server config is not valid TOML");
            })
            .ok()?;
        redact(&mut document, SERVER_SECRETS);
        Some(document.to_string())
    }

    fn list_server_sources(&self) -> Vec<ServerSource> {
        self.router
            .list_all()
            .into_iter()
            .map(|(provider_id, config)| ServerSource {
                id: config.effective_id(),
                provider_id: provider_id.to_string(),
                editable: provider_id
                    .provider_type
                    .starts_with(PLUGIN_PROVIDER_PREFIX),
                provider_type: provider_id.provider_type,
            })
            .collect()
    }

    fn get_proxy_config_document(&self) -> String {
        let mut document = self.current_document();
        redact(&mut document, PROXY_SECRETS);
        document.to_string()
    }

    fn get_effective_proxy_config_document(&self) -> String {
        let mut document = self.running_document();
        redact(&mut document, PROXY_SECRETS);
        document.to_string()
    }

    fn write_proxy_config_document(&self, toml: &str) -> Result<(), ConfigWriteError> {
        let mut document = toml
            .parse::<DocumentMut>()
            .map_err(|e| ConfigWriteError::Parse(e.to_string()))?;

        let _guard = self
            .write_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        reinject(&mut document, &self.current_document(), PROXY_SECRETS);
        let unrestored = still_redacted(&document, PROXY_SECRETS);
        if !unrestored.is_empty() {
            return Err(ConfigWriteError::Validation(format!(
                "nothing to restore the redacted {} from: submit the real value",
                unrestored.join(", ")
            )));
        }

        let text = document.to_string();
        let config: ProxyConfig =
            toml::from_str(&text).map_err(|e| ConfigWriteError::Parse(e.to_string()))?;
        infrarust_config::validate_proxy_document(&config)
            .map_err(|e| ConfigWriteError::Validation(e.to_string()))?;

        write_atomic(&self.config_path, &text).map_err(|e| ConfigWriteError::Io(e.to_string()))
    }

    /// Not implemented: this impl only holds routing tables, and the trait
    /// is part of the frozen plugin surface, so it always returns `None` —
    /// callers cannot distinguish "unimplemented" from "key absent".
    fn get_value(&self, _key: &str) -> Option<String> {
        None
    }
}

/// Writes through a sibling temporary file so a crash mid-write cannot leave
/// a truncated `infrarust.toml` behind.
///
/// The rename replaces the destination's inode, so the scratch file is created
/// private and takes on the destination's permissions before it lands: a
/// config file an operator restricted stays restricted.
fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;

    let mut scratch = ScratchFile::create_beside(path)?;
    scratch.file.write_all(text.as_bytes())?;
    scratch.inherit_permissions(path)?;
    scratch.persist(path)?;

    // Best effort: fsync on a directory handle is not portable, and the
    // rename already landed.
    if let Some(dir) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        let _ = std::fs::File::open(dir).and_then(|d| d.sync_all());
    }
    Ok(())
}

/// How many names a scratch file tries before giving up.
const SCRATCH_ATTEMPTS: u32 = 32;

/// A file that removes itself unless it is [persisted](ScratchFile::persist),
/// under a name no concurrent writer can be using.
struct ScratchFile {
    path: PathBuf,
    file: std::fs::File,
    persisted: bool,
}

impl ScratchFile {
    fn create_beside(destination: &Path) -> std::io::Result<Self> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NONCE: AtomicU64 = AtomicU64::new(0);

        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }

        for _ in 0..SCRATCH_ATTEMPTS {
            let mut name = destination.file_name().unwrap_or_default().to_os_string();
            name.push(format!(
                ".{}.{}.tmp",
                std::process::id(),
                NONCE.fetch_add(1, Ordering::Relaxed)
            ));
            let path = destination.with_file_name(name);

            match options.open(&path) {
                Ok(file) => {
                    return Ok(Self {
                        path,
                        file,
                        persisted: false,
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }

        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!(
                "no free scratch file name next to {}",
                destination.display()
            ),
        ))
    }

    #[cfg(unix)]
    fn inherit_permissions(&self, destination: &Path) -> std::io::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let Ok(metadata) = std::fs::metadata(destination) else {
            return Ok(());
        };
        let mode = metadata.permissions().mode() & 0o7777;
        self.file
            .set_permissions(std::fs::Permissions::from_mode(mode))
    }

    #[cfg(not(unix))]
    fn inherit_permissions(&self, _destination: &Path) -> std::io::Result<()> {
        Ok(())
    }

    fn persist(&mut self, destination: &Path) -> std::io::Result<()> {
        self.file.sync_all()?;
        std::fs::rename(&self.path, destination)?;
        self.persisted = true;
        Ok(())
    }
}

impl Drop for ScratchFile {
    fn drop(&mut self) {
        if !self.persisted {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Refuses writes for plugins without
/// [`Capability::ConfigWrite`](infrarust_api::permissions::Capability::ConfigWrite).
/// Handed out by [`PluginContextImpl`](crate::plugin::context::PluginContextImpl)
/// in place of the real service, so an ungated handle never reaches a plugin.
pub struct ReadOnlyConfigService(Arc<dyn ConfigService>);

impl ReadOnlyConfigService {
    pub fn new(inner: Arc<dyn ConfigService>) -> Self {
        Self(inner)
    }
}

impl infrarust_api::services::config_service::private::Sealed for ReadOnlyConfigService {}

impl ConfigService for ReadOnlyConfigService {
    fn get_server_config(&self, server: &ServerId) -> Option<ServerConfig> {
        self.0.get_server_config(server)
    }

    fn get_all_server_configs(&self) -> Vec<ServerConfig> {
        self.0.get_all_server_configs()
    }

    fn get_server_document(&self, server: &ServerId) -> Option<String> {
        self.0.get_server_document(server)
    }

    fn list_server_sources(&self) -> Vec<ServerSource> {
        self.0.list_server_sources()
    }

    fn get_proxy_config_document(&self) -> String {
        self.0.get_proxy_config_document()
    }

    fn get_effective_proxy_config_document(&self) -> String {
        self.0.get_effective_proxy_config_document()
    }

    fn write_proxy_config_document(&self, _toml: &str) -> Result<(), ConfigWriteError> {
        Err(ConfigWriteError::PermissionDenied)
    }

    fn get_value(&self, key: &str) -> Option<String> {
        self.0.get_value(key)
    }
}

/// Converts internal proxy mode to API proxy mode.
fn convert_proxy_mode(mode: infrarust_config::ProxyMode) -> ProxyMode {
    match mode {
        infrarust_config::ProxyMode::Passthrough => ProxyMode::Passthrough,
        infrarust_config::ProxyMode::ZeroCopy => ProxyMode::ZeroCopy,
        infrarust_config::ProxyMode::ClientOnly => ProxyMode::ClientOnly,
        infrarust_config::ProxyMode::Offline => ProxyMode::Offline,
        infrarust_config::ProxyMode::ServerOnly => ProxyMode::ServerOnly,
        _ => {
            tracing::warn!(
                ?mode,
                "unmapped ProxyMode variant, defaulting to Passthrough"
            );
            ProxyMode::Passthrough
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    const SAMPLE: &str = "\
# the proxy listens here
bind = \"0.0.0.0:25565\"
connect_timeout = \"5s\"

[web]
# the dashboard
bind = \"127.0.0.1:8080\"
api_key = \"super-secret-key-value\"
";

    /// Returns the service plus the temp root it lives in, kept alive by the
    /// caller so the config file outlives the assertions.
    fn service(document: &str) -> (ConfigServiceImpl, tempfile::TempDir) {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("servers")).unwrap();

        let servers_dir = root.path().join("servers");
        let document = format!("servers_dir = {:?}\n{document}", servers_dir.display());
        let path = root.path().join("infrarust.toml");
        std::fs::write(&path, &document).unwrap();

        let config: ProxyConfig = toml::from_str(&document).unwrap();
        let service = ConfigServiceImpl::new(Arc::new(DomainRouter::new()), path, Arc::new(config));
        (service, root)
    }

    fn stored(root: &tempfile::TempDir) -> String {
        std::fs::read_to_string(root.path().join("infrarust.toml")).unwrap()
    }

    #[test]
    fn reading_redacts_the_web_api_key() {
        let (service, _root) = service(SAMPLE);
        let document = service.get_proxy_config_document();

        assert!(!document.contains("super-secret-key-value"));
        assert!(document.contains(infrarust_config::secrets::REDACTED));
        assert!(document.contains("# the dashboard"));
    }

    #[test]
    fn a_read_document_written_back_keeps_the_api_key() {
        let (service, root) = service(SAMPLE);
        let document = service.get_proxy_config_document();

        service.write_proxy_config_document(&document).unwrap();

        let on_disk = stored(&root);
        assert!(on_disk.contains("api_key = \"super-secret-key-value\""));
        let config: ProxyConfig = toml::from_str(&on_disk).unwrap();
        assert_eq!(
            config.web.unwrap().api_key.as_deref(),
            Some("super-secret-key-value")
        );
    }

    #[test]
    fn a_submitted_api_key_replaces_the_stored_one() {
        let (service, root) = service(SAMPLE);
        let document = service
            .get_proxy_config_document()
            .replace(infrarust_config::secrets::REDACTED, "a-brand-new-api-key");

        service.write_proxy_config_document(&document).unwrap();

        let on_disk = stored(&root);
        assert!(on_disk.contains("a-brand-new-api-key"));
        assert!(!on_disk.contains("super-secret-key-value"));
    }

    #[test]
    fn writing_preserves_comments_and_formatting() {
        let (service, root) = service(SAMPLE);
        let document = service
            .get_proxy_config_document()
            .replace("0.0.0.0:25565", "0.0.0.0:25566");

        service.write_proxy_config_document(&document).unwrap();

        let on_disk = stored(&root);
        assert!(on_disk.contains("# the proxy listens here"));
        assert!(on_disk.contains("# the dashboard"));
        assert!(on_disk.contains("0.0.0.0:25566"));
    }

    #[test]
    fn writing_rejects_a_malformed_document() {
        let (service, root) = service(SAMPLE);
        let before = stored(&root);

        let error = service
            .write_proxy_config_document("bind = [unclosed")
            .unwrap_err();

        assert!(matches!(error, ConfigWriteError::Parse(_)));
        assert_eq!(stored(&root), before);
    }

    #[test]
    fn writing_rejects_an_unknown_key() {
        let (service, root) = service(SAMPLE);
        let before = stored(&root);

        let error = service
            .write_proxy_config_document("bnid = \"0.0.0.0:25565\"")
            .unwrap_err();

        assert!(matches!(error, ConfigWriteError::Parse(_)));
        assert_eq!(stored(&root), before);
    }

    #[test]
    fn writing_rejects_a_document_that_fails_validation() {
        let (service, root) = service(SAMPLE);
        let before = stored(&root);
        let document = service
            .get_proxy_config_document()
            .replace("connect_timeout = \"5s\"", "connect_timeout = \"0s\"");

        let error = service.write_proxy_config_document(&document).unwrap_err();

        assert!(matches!(error, ConfigWriteError::Validation(_)));
        assert_eq!(stored(&root), before);
    }

    #[test]
    fn a_deleted_file_falls_back_to_the_running_config() {
        let (service, root) = service(SAMPLE);
        std::fs::remove_file(root.path().join("infrarust.toml")).unwrap();

        let document = service.get_proxy_config_document();
        assert!(!document.contains("super-secret-key-value"));

        service.write_proxy_config_document(&document).unwrap();
        assert!(stored(&root).contains("super-secret-key-value"));
    }

    #[test]
    fn the_read_only_wrapper_denies_writes_but_not_reads() {
        let (service, root) = service(SAMPLE);
        let before = stored(&root);
        let guarded = ReadOnlyConfigService::new(Arc::new(service));

        assert!(
            !guarded
                .get_proxy_config_document()
                .contains("super-secret-key-value")
        );
        assert!(matches!(
            guarded.write_proxy_config_document(SAMPLE).unwrap_err(),
            ConfigWriteError::PermissionDenied
        ));
        assert_eq!(stored(&root), before);
    }

    #[test]
    fn writing_rejects_a_key_the_proxy_could_not_boot_with() {
        let (service, root) = service(SAMPLE);
        let before = stored(&root);
        let document = service
            .get_proxy_config_document()
            .replace(infrarust_config::secrets::REDACTED, "hunter2");

        let error = service.write_proxy_config_document(&document).unwrap_err();

        assert!(matches!(error, ConfigWriteError::Validation(_)));
        assert_eq!(stored(&root), before);
    }

    #[test]
    fn writing_rejects_a_public_bind_with_no_key_to_authenticate_with() {
        let (service, root) = service("bind = \"0.0.0.0:25565\"\n[web]\nbind = \"127.0.0.1:8080\"\n");
        let before = stored(&root);
        let document = service
            .get_proxy_config_document()
            .replace("127.0.0.1:8080", "0.0.0.0:8080");

        let error = service.write_proxy_config_document(&document).unwrap_err();

        assert!(matches!(error, ConfigWriteError::Validation(_)));
        assert_eq!(stored(&root), before);
    }

    #[test]
    fn writing_rejects_a_redaction_nothing_can_restore() {
        let (service, root) = service("bind = \"0.0.0.0:25565\"\n[web]\nbind = \"127.0.0.1:8080\"\n");
        let before = stored(&root);
        let document = format!(
            "{}\n[web]\napi_key = \"{}\"\n",
            "bind = \"0.0.0.0:25565\"",
            infrarust_config::secrets::REDACTED
        );

        let error = service.write_proxy_config_document(&document).unwrap_err();

        assert!(matches!(error, ConfigWriteError::Validation(_)));
        assert_eq!(stored(&root), before);
    }

    #[test]
    fn a_server_document_redacts_the_manager_api_key() {
        let root = tempfile::tempdir().unwrap();
        let router = Arc::new(DomainRouter::new());
        let config: infrarust_config::ServerConfig = toml::from_str(
            "id = \"survival\"\naddresses = [\"10.0.0.1:25565\"]\ndomains = [\"mc.example.com\"]\n\
             [server_manager]\ntype = \"pterodactyl\"\napi_url = \"https://panel\"\n\
             api_key = \"ptlc_live_xxx\"\nserver_id = \"abc\"\n",
        )
        .unwrap();
        router.add(
            crate::provider::ProviderId::new("file", "survival.toml"),
            config,
        );
        let service = ConfigServiceImpl::new(
            router,
            root.path().join("infrarust.toml"),
            Arc::new(toml::from_str::<ProxyConfig>("").unwrap()),
        );

        let document = service
            .get_server_document(&ServerId::new("survival"))
            .expect("the router knows the server");

        assert!(!document.contains("ptlc_live_xxx"));
        assert!(document.contains(infrarust_config::secrets::REDACTED));
        assert!(document.contains("https://panel"));
    }

    #[test]
    fn the_effective_document_reports_the_running_config() {
        let (service, root) = service(SAMPLE);
        let elsewhere = root.path().join("elsewhere");
        let mut running: ProxyConfig = toml::from_str("").unwrap();
        running.servers_dir.clone_from(&elsewhere);
        let service = ConfigServiceImpl::new(
            Arc::new(DomainRouter::new()),
            service.config_path.clone(),
            Arc::new(running),
        );

        let effective: ProxyConfig =
            toml::from_str(&service.get_effective_proxy_config_document()).unwrap();
        let stored: ProxyConfig = toml::from_str(&service.get_proxy_config_document()).unwrap();

        assert_eq!(effective.servers_dir, elsewhere);
        assert_eq!(stored.servers_dir, root.path().join("servers"));
    }

    /// The proxy may have been started with `--servers-dir`, so the directory
    /// the document names is not the one it runs on.
    #[test]
    fn writing_ignores_where_the_documents_directories_point() {
        let (service, root) = service(SAMPLE);
        let document = service
            .get_proxy_config_document()
            .replace(&format!("{:?}", root.path().join("servers").display()), "\"./nowhere\"");
        assert!(document.contains("./nowhere"));

        service.write_proxy_config_document(&document).unwrap();

        assert!(stored(&root).contains("./nowhere"));
    }

    #[cfg(unix)]
    #[test]
    fn writing_keeps_the_files_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let (service, root) = service(SAMPLE);
        let path = root.path().join("infrarust.toml");

        for mode in [0o600, 0o640] {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();

            service
                .write_proxy_config_document(&service.get_proxy_config_document())
                .unwrap();

            let after = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(after, mode, "a config keeps the permissions it was given");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_config_file_created_by_a_write_is_private() {
        use std::os::unix::fs::PermissionsExt;

        let (service, root) = service(SAMPLE);
        let path = root.path().join("infrarust.toml");
        let document = service.get_proxy_config_document();
        std::fs::remove_file(&path).unwrap();

        service.write_proxy_config_document(&document).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn a_failed_write_leaves_no_scratch_file_behind() {
        let root = tempfile::tempdir().unwrap();
        let occupied = root.path().join("infrarust.toml");
        std::fs::create_dir(&occupied).unwrap();

        let error = write_atomic(&occupied, "bind = \"0.0.0.0:25565\"\n").unwrap_err();

        assert!(error.kind() != std::io::ErrorKind::NotFound);
        let leftovers: Vec<_> = std::fs::read_dir(root.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name())
            .filter(|name| name.to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "left behind {leftovers:?}");
    }

    #[test]
    fn two_writers_never_share_a_scratch_file() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("infrarust.toml");

        let first = ScratchFile::create_beside(&destination).unwrap();
        let second = ScratchFile::create_beside(&destination).unwrap();

        assert_ne!(first.path, second.path);
    }

    #[test]
    fn concurrent_writes_leave_a_readable_config() {
        let (service, root) = service(SAMPLE);
        let service = Arc::new(service);
        let document = service.get_proxy_config_document();

        std::thread::scope(|scope| {
            for worker in 0..8 {
                let service = Arc::clone(&service);
                let document = document.replace("0.0.0.0:25565", &format!("0.0.0.0:255{worker:02}"));
                scope.spawn(move || {
                    for _ in 0..20 {
                        service.write_proxy_config_document(&document).unwrap();
                    }
                });
            }
        });

        let on_disk = stored(&root);
        toml::from_str::<ProxyConfig>(&on_disk).expect("the config file is still readable");
        assert!(on_disk.contains("super-secret-key-value"));
    }
}

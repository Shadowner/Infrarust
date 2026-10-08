use std::collections::HashMap;
use std::sync::Mutex;

use crate::services::config_service::{
    ConfigService, ConfigWriteError, ServerConfig, ServerSource,
};
use crate::types::{ServerAddress, ServerId};

use super::lock;

type Transform = Box<dyn Fn(&str) -> String + Send + Sync>;
type Merge = Box<dyn Fn(&str, &str) -> Result<String, ConfigWriteError> + Send + Sync>;

#[derive(Default)]
enum WritePolicy {
    #[default]
    Denied,
    Accept,
    Merge(Merge),
}

#[derive(Default)]
pub struct MockConfigService {
    servers: Mutex<Vec<ServerConfig>>,
    documents: Mutex<HashMap<ServerId, String>>,
    sources: Mutex<Vec<ServerSource>>,
    values: Mutex<HashMap<String, String>>,
    proxy_document: Mutex<String>,
    redact: Option<Transform>,
    effective: Option<Transform>,
    policy: WritePolicy,
    written: Mutex<Vec<String>>,
}

impl std::fmt::Debug for MockConfigService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockConfigService")
            .field("servers", &lock(&self.servers))
            .field("values", &lock(&self.values))
            .field("written", &lock(&self.written))
            .finish_non_exhaustive()
    }
}

impl MockConfigService {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn server(id: impl Into<ServerId>) -> ServerConfig {
        ServerConfig::new(id.into())
    }

    #[must_use]
    pub fn server_at(id: impl Into<ServerId>, address: ServerAddress) -> ServerConfig {
        let mut config = Self::server(id);
        config.addresses = vec![address];
        config
    }

    #[must_use]
    pub fn server_document(id: &ServerId) -> String {
        format!("id = \"{id}\"\naddresses = [\"127.0.0.1:25565\"]\n")
    }

    #[must_use]
    pub fn file_source(id: &ServerId) -> ServerSource {
        ServerSource {
            id: id.to_string(),
            provider_id: "file@server.toml".to_owned(),
            provider_type: "file".to_owned(),
            editable: false,
        }
    }

    #[must_use]
    pub fn with_server(self, config: ServerConfig) -> Self {
        let id = config.id.clone();
        lock(&self.documents).insert(id.clone(), Self::server_document(&id));
        lock(&self.sources).push(Self::file_source(&id));
        let mut servers = lock(&self.servers);
        servers.retain(|s| s.id != id);
        servers.push(config);
        drop(servers);
        self
    }

    #[must_use]
    pub fn with_servers(self, count: usize) -> Self {
        (0..count).fold(self, |this, n| {
            this.with_server(Self::server(format!("server_{n}")))
        })
    }

    #[must_use]
    pub fn with_server_document(self, id: impl Into<ServerId>, document: &str) -> Self {
        lock(&self.documents).insert(id.into(), document.to_owned());
        self
    }

    #[must_use]
    pub fn with_source(self, source: ServerSource) -> Self {
        lock(&self.sources).push(source);
        self
    }

    #[must_use]
    pub fn with_value(self, key: &str, value: &str) -> Self {
        lock(&self.values).insert(key.to_owned(), value.to_owned());
        self
    }

    #[must_use]
    pub fn with_proxy_document(self, document: &str) -> Self {
        *lock(&self.proxy_document) = document.to_owned();
        self
    }

    #[must_use]
    pub fn redacting(mut self, redact: impl Fn(&str) -> String + Send + Sync + 'static) -> Self {
        self.redact = Some(Box::new(redact));
        self
    }

    #[must_use]
    pub fn effective_with(
        mut self,
        effective: impl Fn(&str) -> String + Send + Sync + 'static,
    ) -> Self {
        self.effective = Some(Box::new(effective));
        self
    }

    #[must_use]
    pub fn accepting_writes(mut self) -> Self {
        self.policy = WritePolicy::Accept;
        self
    }

    #[must_use]
    pub fn merging_writes(
        mut self,
        merge: impl Fn(&str, &str) -> Result<String, ConfigWriteError> + Send + Sync + 'static,
    ) -> Self {
        self.policy = WritePolicy::Merge(Box::new(merge));
        self
    }

    #[must_use]
    pub fn written(&self) -> Vec<String> {
        lock(&self.written).clone()
    }

    #[must_use]
    pub fn stored_proxy_document(&self) -> String {
        lock(&self.proxy_document).clone()
    }
}

impl crate::services::config_service::private::Sealed for MockConfigService {}

impl ConfigService for MockConfigService {
    fn get_server_config(&self, server: &ServerId) -> Option<ServerConfig> {
        lock(&self.servers)
            .iter()
            .find(|config| &config.id == server)
            .cloned()
    }

    fn get_server_config_by_domain(&self, domain: &str) -> Option<ServerConfig> {
        let domain = domain.trim_end_matches('.');
        lock(&self.servers)
            .iter()
            .find(|config| {
                config
                    .domains
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(domain))
            })
            .cloned()
    }

    fn get_all_server_configs(&self) -> Vec<ServerConfig> {
        lock(&self.servers).clone()
    }

    fn get_server_document(&self, server: &ServerId) -> Option<String> {
        lock(&self.documents).get(server).cloned()
    }

    fn list_server_sources(&self) -> Vec<ServerSource> {
        lock(&self.sources).clone()
    }

    fn get_proxy_config_document(&self) -> String {
        let stored = lock(&self.proxy_document).clone();
        self.redact.as_ref().map_or(stored.clone(), |f| f(&stored))
    }

    fn get_effective_proxy_config_document(&self) -> String {
        let document = self.get_proxy_config_document();
        self.effective
            .as_ref()
            .map_or(document.clone(), |f| f(&document))
    }

    fn write_proxy_config_document(&self, toml: &str) -> Result<(), ConfigWriteError> {
        lock(&self.written).push(toml.to_owned());
        let mut stored = lock(&self.proxy_document);
        let text = match &self.policy {
            WritePolicy::Denied => return Err(ConfigWriteError::PermissionDenied),
            WritePolicy::Accept => toml.to_owned(),
            WritePolicy::Merge(merge) => merge(toml, &stored)?,
        };
        *stored = text;
        Ok(())
    }

    fn get_value(&self, key: &str) -> Option<String> {
        lock(&self.values).get(key).cloned()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn servers_come_with_a_document_and_a_file_source() {
        let config = MockConfigService::new()
            .with_servers(2)
            .with_value("greeting", "hello");
        let first = ServerId::new("server_0");
        assert_eq!(config.get_all_server_configs().len(), 2);
        assert!(config.get_server_config(&first).is_some());
        assert!(config.get_server_config(&ServerId::new("nope")).is_none());
        assert!(
            config
                .get_server_document(&first)
                .unwrap()
                .contains("server_0")
        );
        assert_eq!(config.list_server_sources().len(), 2);
        assert_eq!(config.get_value("greeting").as_deref(), Some("hello"));
        assert_eq!(config.get_value("other"), None);
    }

    #[test]
    fn a_domain_finds_the_server_that_lists_it() {
        let config = MockConfigService::new().with_server(
            ServerConfig::new(ServerId::new("lobby")).domains(vec!["play.example.com".into()]),
        );
        assert_eq!(
            config
                .get_server_config_by_domain("Play.Example.com.")
                .map(|server| server.id),
            Some(ServerId::new("lobby"))
        );
        assert!(config.get_server_config_by_domain("example.com").is_none());
    }

    #[test]
    fn proxy_documents_are_redacted_on_read_and_merged_on_write() {
        let denied = MockConfigService::new().with_proxy_document("a = 1\n");
        assert!(matches!(
            denied.write_proxy_config_document("a = 2\n"),
            Err(ConfigWriteError::PermissionDenied)
        ));
        assert_eq!(denied.written(), ["a = 2\n"]);
        assert_eq!(denied.get_proxy_config_document(), "a = 1\n");

        let merged = MockConfigService::new()
            .with_proxy_document("key = \"secret\"\n")
            .redacting(|doc| doc.replace("secret", "***"))
            .effective_with(|doc| format!("{doc}effective = true\n"))
            .merging_writes(|submitted, stored| Ok(format!("{stored}{submitted}")));
        assert_eq!(merged.get_proxy_config_document(), "key = \"***\"\n");
        assert!(
            merged
                .get_effective_proxy_config_document()
                .ends_with("effective = true\n")
        );
        merged.write_proxy_config_document("more = 1\n").unwrap();
        assert_eq!(
            merged.stored_proxy_document(),
            "key = \"secret\"\nmore = 1\n"
        );

        let accepting = MockConfigService::new().accepting_writes();
        accepting.write_proxy_config_document("x = 1\n").unwrap();
        assert_eq!(accepting.stored_proxy_document(), "x = 1\n");
    }
}

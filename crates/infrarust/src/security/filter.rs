use std::{
    any::Any,
    collections::HashMap,
    fmt::{Debug, Display},
    io,
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use infrarust_config::LogType;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{net::TcpStream, sync::RwLock};
use tracing::{debug, info};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterType {
    RateLimiter,
    BanFilter,
    IpFilter,
    GeoFilter,
    Custom(u16),
}

impl Display for FilterType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FilterType::RateLimiter => write!(f, "RateLimiter"),
            FilterType::BanFilter => write!(f, "BanFilter"),
            FilterType::IpFilter => write!(f, "IpFilter"),
            FilterType::GeoFilter => write!(f, "GeoFilter"),
            FilterType::Custom(id) => write!(f, "Custom({})", id),
        }
    }
}

#[derive(Debug, Error)]
pub enum FilterError {
    #[error("Filter not found: {0}")]
    NotFound(String),

    #[error("Filter is not configurable")]
    NotConfigurable,

    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("IO error: {0}")]
    IoError(#[from] io::Error),

    #[error("Filter error: {0}")]
    Other(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ConfigValue {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    List(Vec<ConfigValue>),
    Map(HashMap<String, ConfigValue>),
    Duration(u64), // stored as seconds
}

impl ConfigValue {
    pub fn as_string(&self) -> Option<&String> {
        if let ConfigValue::String(s) = self {
            Some(s)
        } else {
            None
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        if let ConfigValue::Integer(i) = self {
            Some(*i)
        } else {
            None
        }
    }

    pub fn as_duration(&self) -> Option<Duration> {
        if let ConfigValue::Duration(secs) = self {
            Some(Duration::from_secs(*secs))
        } else {
            None
        }
    }

    // Add more accessor methods as needed
}

#[async_trait]
pub trait Filter: Send + Sync + Debug {
    async fn filter(&self, stream: &TcpStream) -> io::Result<()>;

    fn name(&self) -> &str;

    fn filter_type(&self) -> FilterType;

    fn is_configurable(&self) -> bool {
        false
    }

    async fn apply_config(&self, _config: ConfigValue) -> Result<(), FilterError> {
        Err(FilterError::NotConfigurable)
    }

    fn is_refreshable(&self) -> bool {
        false
    }

    async fn refresh(&self) -> Result<(), FilterError> {
        Err(FilterError::Other(
            "Filter does not support refresh".to_string(),
        ))
    }

    /// Convert to Any for downcasting
    fn as_any(&self) -> &dyn Any;
}

#[derive(Debug, Clone)]
pub struct FilterRegistryEntry {
    filter: Arc<dyn Filter>,
    enabled: bool,
}

#[derive(Debug, Default)]
pub struct FilterRegistry {
    filters: RwLock<HashMap<String, FilterRegistryEntry>>,
}

impl FilterRegistry {
    pub fn new() -> Self {
        Self {
            filters: RwLock::new(HashMap::new()),
        }
    }

    pub async fn register<F>(&self, filter: F) -> Result<(), FilterError>
    where
        F: Filter + 'static,
    {
        let name = filter.name().to_string();
        let filter_type = filter.filter_type();
        let filter = Arc::new(filter);

        let mut filters = self.filters.write().await;
        if filters.contains_key(&name) {
            return Err(FilterError::Other(format!(
                "Filter '{}' already registered",
                name
            )));
        }

        info!(
            log_type = LogType::Filter.as_str(),
            "Registering filter '{}' of type {}", name, filter_type
        );
        filters.insert(
            name,
            FilterRegistryEntry {
                filter,
                enabled: true,
            },
        );
        Ok(())
    }

    pub async fn unregister(&self, name: &str) -> Result<(), FilterError> {
        let mut filters = self.filters.write().await;
        if filters.remove(name).is_none() {
            return Err(FilterError::NotFound(name.to_string()));
        }
        info!(
            log_type = LogType::Filter.as_str(),
            "Unregistered filter '{}'", name
        );
        Ok(())
    }

    pub async fn enable(&self, name: &str) -> Result<(), FilterError> {
        let mut filters = self.filters.write().await;
        if let Some(entry) = filters.get_mut(name) {
            entry.enabled = true;
            info!(
                log_type = LogType::Filter.as_str(),
                "Enabled filter '{}'", name
            );
            Ok(())
        } else {
            Err(FilterError::NotFound(name.to_string()))
        }
    }

    pub async fn disable(&self, name: &str) -> Result<(), FilterError> {
        let mut filters = self.filters.write().await;
        if let Some(entry) = filters.get_mut(name) {
            entry.enabled = false;
            info!(
                log_type = LogType::Filter.as_str(),
                "Disabled filter '{}'", name
            );
            Ok(())
        } else {
            Err(FilterError::NotFound(name.to_string()))
        }
    }

    pub async fn is_enabled(&self, name: &str) -> Result<bool, FilterError> {
        let filters = self.filters.read().await;
        if let Some(entry) = filters.get(name) {
            Ok(entry.enabled)
        } else {
            Err(FilterError::NotFound(name.to_string()))
        }
    }

    pub async fn get_filter(&self, name: &str) -> Result<Arc<dyn Filter>, FilterError> {
        let filters = self.filters.read().await;
        if let Some(entry) = filters.get(name) {
            Ok(entry.filter.clone())
        } else {
            Err(FilterError::NotFound(name.to_string()))
        }
    }

    pub async fn configure(&self, name: &str, config: ConfigValue) -> Result<(), FilterError> {
        let filters = self.filters.read().await;
        if let Some(entry) = filters.get(name) {
            if !entry.filter.is_configurable() {
                return Err(FilterError::NotConfigurable);
            }
            entry.filter.apply_config(config).await
        } else {
            Err(FilterError::NotFound(name.to_string()))
        }
    }

    pub async fn configure_with<F, T>(
        &self,
        name: &str,
        config_fn: F,
    ) -> Result<Arc<T>, FilterError>
    where
        F: FnOnce(&str, FilterType) -> Option<Arc<T>>,
        T: Filter + 'static,
    {
        let filters = self.filters.read().await;
        if let Some(entry) = filters.get(name) {
            let filter_type = entry.filter.filter_type();
            if let Some(new_filter) = config_fn(name, filter_type) {
                return Ok(new_filter);
            }
        }

        Err(FilterError::NotFound(name.to_string()))
    }

    pub async fn refresh(&self, name: &str) -> Result<(), FilterError> {
        let filters = self.filters.read().await;
        if let Some(entry) = filters.get(name) {
            if !entry.filter.is_refreshable() {
                return Err(FilterError::Other(format!(
                    "Filter '{}' is not refreshable",
                    name
                )));
            }
            entry.filter.refresh().await
        } else {
            Err(FilterError::NotFound(name.to_string()))
        }
    }

    pub async fn refresh_all(&self) -> Vec<(String, Result<(), FilterError>)> {
        let filters = self.filters.read().await;
        let mut results = Vec::new();

        for (name, entry) in filters.iter() {
            if entry.filter.is_refreshable() {
                let result = entry.filter.refresh().await;
                results.push((name.clone(), result));
            }
        }

        results
    }

    pub async fn filter(&self, stream: &TcpStream) -> io::Result<()> {
        let filters = self.filters.read().await;

        for (name, entry) in filters.iter() {
            if !entry.enabled {
                continue;
            }

            match entry.filter.filter(stream).await {
                Ok(_) => debug!(
                    log_type = LogType::Filter.as_str(),
                    "Filter '{}' passed", name
                ),
                Err(e) => {
                    debug!(
                        log_type = LogType::Filter.as_str(),
                        "Filter '{}' rejected connection: {}", name, e
                    );
                    return Err(e);
                }
            }
        }

        Ok(())
    }

    pub async fn list_filters(&self) -> Vec<(String, FilterType, bool)> {
        let filters = self.filters.read().await;
        filters
            .iter()
            .map(|(name, entry)| (name.clone(), entry.filter.filter_type(), entry.enabled))
            .collect()
    }

    pub fn get_rate_limiter_metrics(&self) -> Option<Vec<(String, Option<usize>)>> {
        use crate::security::rate_limiter::RateLimiter;

        let filters = self.filters.try_read().ok()?;
        let mut metrics = Vec::new();

        for (name, entry) in filters.iter() {
            if entry.filter.filter_type() == FilterType::RateLimiter {
                if let Some(rate_limiter) = entry.filter.as_any().downcast_ref::<RateLimiter>() {
                    metrics.push((name.clone(), rate_limiter.counter_size()));
                }
            }
        }

        Some(metrics)
    }

    pub fn filter_count(&self) -> Option<usize> {
        self.filters.try_read().ok().map(|f| f.len())
    }
}

#[derive(Default, Clone)]
pub struct FilterChain {
    filters: Vec<Arc<dyn Filter>>,
}

impl FilterChain {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_filter<F: Filter + 'static>(&mut self, filter: F) {
        self.filters.push(Arc::new(filter));
    }

    pub async fn filter(&self, stream: &TcpStream) -> io::Result<()> {
        for filter in &self.filters {
            filter.filter(stream).await?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct FilterConfig {
    pub rate_limiter: Option<RateLimiterConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RateLimiterConfig {
    pub request_limit: u32,
    pub window_length: Duration,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[derive(Debug)]
    struct TestFilter {
        name: String,
        should_fail: bool,
    }

    impl TestFilter {
        fn new(name: &str, should_fail: bool) -> Self {
            Self {
                name: name.to_string(),
                should_fail,
            }
        }
    }

    #[async_trait]
    impl Filter for TestFilter {
        async fn filter(&self, _: &TcpStream) -> io::Result<()> {
            if self.should_fail {
                Err(io::Error::other("filter failed"))
            } else {
                Ok(())
            }
        }

        fn name(&self) -> &str {
            &self.name
        }

        fn filter_type(&self) -> FilterType {
            FilterType::Custom(0)
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    async fn create_test_connection() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let client_task = tokio::spawn(async move { TcpStream::connect(addr).await.unwrap() });

        let (server_stream, _) = listener.accept().await.unwrap();
        let client_stream = client_task.await.unwrap();

        (client_stream, server_stream)
    }

    #[tokio::test]
    async fn test_filter_chain() {
        let mut chain = FilterChain::new();
        chain.add_filter(TestFilter::new("test1", false));
        chain.add_filter(TestFilter::new("test2", false));

        let (client, _server) = create_test_connection().await;
        assert!(chain.filter(&client).await.is_ok());

        chain.add_filter(TestFilter::new("test3", true));
        assert!(chain.filter(&client).await.is_err());
    }

    #[tokio::test]
    async fn test_empty_registry_allows_all() {
        let registry = FilterRegistry::new();
        let (client, _server) = create_test_connection().await;

        // Empty registry should allow all connections
        let result = registry.filter(&client).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_registry_register_and_unregister() {
        let registry = FilterRegistry::new();

        // Register a filter
        let result = registry
            .register(TestFilter::new("test_filter", false))
            .await;
        assert!(result.is_ok());

        // Verify it's listed
        let filters = registry.list_filters().await;
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].0, "test_filter");

        // Unregister
        let result = registry.unregister("test_filter").await;
        assert!(result.is_ok());

        let filters = registry.list_filters().await;
        assert!(filters.is_empty());
    }

    #[tokio::test]
    async fn test_registry_duplicate_registration_fails() {
        let registry = FilterRegistry::new();

        registry
            .register(TestFilter::new("duplicate", false))
            .await
            .unwrap();

        // Second registration should fail
        let result = registry
            .register(TestFilter::new("duplicate", false))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_enabled_filter_blocks_connection() {
        let registry = FilterRegistry::new();
        let (client, _server) = create_test_connection().await;

        // Register a blocking filter
        registry
            .register(TestFilter::new("blocker", true))
            .await
            .unwrap();

        // Should block
        let result = registry.filter(&client).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_disabled_filter_allows_connection() {
        let registry = FilterRegistry::new();
        let (client, _server) = create_test_connection().await;

        // Register a blocking filter
        registry
            .register(TestFilter::new("blocker", true))
            .await
            .unwrap();

        // Disable it
        registry.disable("blocker").await.unwrap();

        // Should now allow
        let result = registry.filter(&client).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_enable_disable_filter() {
        let registry = FilterRegistry::new();

        registry
            .register(TestFilter::new("toggle_filter", false))
            .await
            .unwrap();

        // Initially enabled
        assert!(registry.is_enabled("toggle_filter").await.unwrap());

        // Disable
        registry.disable("toggle_filter").await.unwrap();
        assert!(!registry.is_enabled("toggle_filter").await.unwrap());

        // Re-enable
        registry.enable("toggle_filter").await.unwrap();
        assert!(registry.is_enabled("toggle_filter").await.unwrap());
    }

    #[tokio::test]
    async fn test_get_filter() {
        let registry = FilterRegistry::new();

        registry
            .register(TestFilter::new("my_filter", false))
            .await
            .unwrap();

        let filter = registry.get_filter("my_filter").await;
        assert!(filter.is_ok());
        assert_eq!(filter.unwrap().name(), "my_filter");
    }

    #[tokio::test]
    async fn test_get_nonexistent_filter() {
        let registry = FilterRegistry::new();

        let result = registry.get_filter("nonexistent").await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), FilterError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_filter_chain_order() {
        let mut chain = FilterChain::new();

        chain.add_filter(TestFilter::new("pass", false));
        chain.add_filter(TestFilter::new("block", true));

        let (client, _server) = create_test_connection().await;

        let result = chain.filter(&client).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_config_value_accessors() {
        let string_val = ConfigValue::String("test".to_string());
        assert_eq!(string_val.as_string(), Some(&"test".to_string()));
        assert_eq!(string_val.as_int(), None);

        let int_val = ConfigValue::Integer(42);
        assert_eq!(int_val.as_int(), Some(42));
        assert_eq!(int_val.as_string(), None);

        let duration_val = ConfigValue::Duration(60);
        assert_eq!(
            duration_val.as_duration(),
            Some(std::time::Duration::from_secs(60))
        );
    }

    #[tokio::test]
    async fn test_filter_type_display() {
        assert_eq!(format!("{}", FilterType::RateLimiter), "RateLimiter");
        assert_eq!(format!("{}", FilterType::BanFilter), "BanFilter");
        assert_eq!(format!("{}", FilterType::IpFilter), "IpFilter");
        assert_eq!(format!("{}", FilterType::GeoFilter), "GeoFilter");
        assert_eq!(format!("{}", FilterType::Custom(42)), "Custom(42)");
    }

    #[tokio::test]
    async fn test_multiple_passing_filters() {
        let registry = FilterRegistry::new();
        let (client, _server) = create_test_connection().await;

        registry
            .register(TestFilter::new("filter1", false))
            .await
            .unwrap();
        registry
            .register(TestFilter::new("filter2", false))
            .await
            .unwrap();
        registry
            .register(TestFilter::new("filter3", false))
            .await
            .unwrap();

        let result = registry.filter(&client).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_unregister_nonexistent_filter() {
        let registry = FilterRegistry::new();

        let result = registry.unregister("nonexistent").await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), FilterError::NotFound(_)));
    }
}

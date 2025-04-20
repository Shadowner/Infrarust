use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct AuditLogRotation {
    pub max_size: usize,  // Maximum file size in bytes before rotation
    pub max_files: usize, // Maximum number of rotated files to keep
    pub compress: bool,   // Whether to compress rotated files
}

impl Default for AuditLogRotation {
    fn default() -> Self {
        Self {
            max_size: 10 * 1024 * 1024, // 10MB
            max_files: 5,
            compress: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct BanConfig {
    pub enabled: bool,
    pub storage_type: String,
    pub file_path: Option<String>,
    pub redis_url: Option<String>,
    pub database_url: Option<String>,
    pub enable_audit_log: bool,
    pub audit_log_path: Option<String>,
    pub audit_log_rotation: Option<AuditLogRotation>,
    pub auto_cleanup_interval: u64,
    pub cache_size: usize,
}

impl Default for BanConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            storage_type: "file".to_string(),
            file_path: Some("bans.json".to_string()),
            redis_url: None,
            database_url: None,
            enable_audit_log: true,
            audit_log_path: Some("bans_audit.log".to_string()),
            audit_log_rotation: Some(AuditLogRotation::default()),
            auto_cleanup_interval: 3600, // 1 hour
            cache_size: 10_000,
        }
    }
}

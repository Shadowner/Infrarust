//! V1 → V2 directory migration.

use std::path::Path;

use super::convert::{convert_v1_proxy_config, convert_v1_to_v2, MigrationSeverity, MigrationWarning};
use super::v1_types::{V1InfrarustConfig, V1ServerConfig};
use crate::error::ConfigError;

pub fn migrate_directory(
    input_dir: &Path,
    output_dir: &Path,
) -> Result<Vec<MigrationWarning>, ConfigError> {
    if !input_dir.is_dir() {
        return Err(ConfigError::Validation(format!(
            "Input directory does not exist: {}",
            input_dir.display()
        )));
    }

    std::fs::create_dir_all(output_dir).map_err(|e| {
        ConfigError::Validation(format!(
            "Cannot create output directory {}: {e}",
            output_dir.display()
        ))
    })?;

    let mut all_warnings = Vec::new();
    let mut converted = 0u32;
    let mut skipped = 0u32;

    let mut entries = Vec::new();
    for result in std::fs::read_dir(input_dir).map_err(|e| {
        ConfigError::Validation(format!(
            "Cannot read input directory {}: {e}",
            input_dir.display()
        ))
    })? {
        match result {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                all_warnings.push(MigrationWarning {
                    severity: MigrationSeverity::Error,
                    file: "unknown".to_string(),
                    message: format!("Cannot read directory entry: {e}"),
                });
            }
        }
    }

    for entry in &entries {
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "yaml" && ext != "yml" {
            continue;
        }

        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                all_warnings.push(MigrationWarning {
                    severity: MigrationSeverity::Error,
                    file: filename.to_string(),
                    message: format!("Cannot read file: {e}"),
                });
                skipped += 1;
                continue;
            }
        };

        let v1: V1ServerConfig = match serde_yml::from_str(&content) {
            Ok(c) => c,
            Err(e) => {
                all_warnings.push(MigrationWarning {
                    severity: MigrationSeverity::Error,
                    file: filename.to_string(),
                    message: format!("YAML parse error: {e}"),
                });
                skipped += 1;
                continue;
            }
        };

        if v1.domains.is_empty() && v1.addresses.is_empty() {
            all_warnings.push(MigrationWarning {
                severity: MigrationSeverity::Warning,
                file: filename.to_string(),
                message: "Skipped: empty config (no domains or addresses)".to_string(),
            });
            skipped += 1;
            continue;
        }

        let result = convert_v1_to_v2(&v1, filename);
        all_warnings.extend(result.warnings);

        let toml_content = match toml::to_string_pretty(&result.config) {
            Ok(t) => t,
            Err(e) => {
                all_warnings.push(MigrationWarning {
                    severity: MigrationSeverity::Error,
                    file: filename.to_string(),
                    message: format!("TOML serialization error: {e}"),
                });
                skipped += 1;
                continue;
            }
        };

        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        let out_path = output_dir.join(format!("{stem}.toml"));

        if let Err(e) = std::fs::write(&out_path, toml_content) {
            all_warnings.push(MigrationWarning {
                severity: MigrationSeverity::Error,
                file: filename.to_string(),
                message: format!("Cannot write output file: {e}"),
            });
            skipped += 1;
            continue;
        }

        converted += 1;
    }

    all_warnings.push(MigrationWarning {
        severity: MigrationSeverity::Info,
        file: "summary".to_string(),
        message: format!("{converted} file(s) converted, {skipped} skipped"),
    });

    Ok(all_warnings)
}

pub fn migrate_proxy_config(
    input_file: &Path,
    output_file: &Path,
) -> Result<Vec<MigrationWarning>, ConfigError> {
    let content = std::fs::read_to_string(input_file).map_err(|e| {
        ConfigError::Validation(format!(
            "Cannot read config file {}: {e}",
            input_file.display()
        ))
    })?;

    let v1: V1InfrarustConfig = serde_yml::from_str(&content).map_err(|e| {
        ConfigError::Validation(format!(
            "YAML parse error in {}: {e}",
            input_file.display()
        ))
    })?;

    let result = convert_v1_proxy_config(&v1);

    let toml_content = toml::to_string_pretty(&result.config).map_err(|e| {
        ConfigError::Validation(format!("TOML serialization error: {e}"))
    })?;

    if let Some(parent) = output_file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            ConfigError::Validation(format!(
                "Cannot create output directory {}: {e}",
                parent.display()
            ))
        })?;
    }

    std::fs::write(output_file, toml_content).map_err(|e| {
        ConfigError::Validation(format!(
            "Cannot write output file {}: {e}",
            output_file.display()
        ))
    })?;

    Ok(result.warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_migrate_directory_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let input = tmp.path().join("v1");
        let output = tmp.path().join("v2");
        fs::create_dir_all(&input).unwrap();

        fs::write(
            input.join("survival.yaml"),
            r#"
domains:
  - "survival.example.com"
addresses:
  - "127.0.0.1:25565"
proxyMode: passthrough
sendProxyProtocol: false
motds:
  online:
    enabled: true
    text: "§aWelcome!"
    version_name: "Paper 1.20"
    max_players: 100
  offline:
    enabled: true
    text: "§eSleeping"
"#,
        )
        .unwrap();

        let warnings = migrate_directory(&input, &output).unwrap();

        let out_path = output.join("survival.toml");
        assert!(out_path.exists());

        let content = fs::read_to_string(&out_path).unwrap();
        let config: crate::server::ServerConfig = toml::from_str(&content).unwrap();
        assert_eq!(config.domains, vec!["survival.example.com"]);
        assert!(config.motd.sleeping.is_some());
        assert!(config.motd.online.is_some());

        let summary = warnings.iter().find(|w| w.file == "summary").unwrap();
        assert!(summary.message.contains("1 file(s) converted"));
    }

    #[test]
    fn test_migrate_invalid_dir() {
        let result = migrate_directory(Path::new("/nonexistent"), Path::new("/tmp/out"));
        assert!(result.is_err());
    }
}

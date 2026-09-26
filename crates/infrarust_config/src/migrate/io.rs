//! V1 → V2 directory migration.

use std::path::Path;

use super::convert::{
    MigrationSeverity, MigrationWarning, convert_v1_proxy_config, convert_v1_to_v2,
};
use super::v1_types::{V1InfrarustConfig, V1ServerConfig};
use crate::error::ConfigError;

#[derive(Debug, Default)]
pub struct MigrationReport {
    pub converted: usize,
    pub skipped: usize,
    pub warnings: Vec<MigrationWarning>,
}

pub fn migrate_directory(
    input_dir: &Path,
    output_dir: &Path,
) -> Result<MigrationReport, ConfigError> {
    if !input_dir.is_dir() {
        return Err(ConfigError::DirectoryNotFound(input_dir.to_path_buf()));
    }

    std::fs::create_dir_all(output_dir).map_err(|source| ConfigError::CreateDir {
        path: output_dir.to_path_buf(),
        source,
    })?;

    let mut all_warnings = Vec::new();
    let mut converted = 0usize;
    let mut skipped = 0usize;

    let mut entries = Vec::new();
    for result in std::fs::read_dir(input_dir).map_err(|source| ConfigError::ReadDir {
        path: input_dir.to_path_buf(),
        source,
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

    if converted == 0 && skipped > 0 {
        return Err(ConfigError::NothingMigrated {
            skipped,
            warnings: all_warnings,
        });
    }

    Ok(MigrationReport {
        converted,
        skipped,
        warnings: all_warnings,
    })
}

pub fn migrate_proxy_config(
    input_file: &Path,
    output_file: &Path,
) -> Result<Vec<MigrationWarning>, ConfigError> {
    let content = std::fs::read_to_string(input_file).map_err(|source| ConfigError::ReadFile {
        path: input_file.to_path_buf(),
        source,
    })?;

    let v1: V1InfrarustConfig =
        serde_yml::from_str(&content).map_err(|source| ConfigError::ParseYaml {
            path: input_file.to_path_buf(),
            source,
        })?;

    let result = convert_v1_proxy_config(&v1);

    let toml_content =
        toml::to_string_pretty(&result.config).map_err(|source| ConfigError::SerializeToml {
            path: output_file.to_path_buf(),
            source,
        })?;

    if let Some(parent) = output_file.parent() {
        std::fs::create_dir_all(parent).map_err(|source| ConfigError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    std::fs::write(output_file, toml_content).map_err(|source| ConfigError::WriteFile {
        path: output_file.to_path_buf(),
        source,
    })?;

    Ok(result.warnings)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
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

        let report = migrate_directory(&input, &output).unwrap();

        let out_path = output.join("survival.toml");
        assert!(out_path.exists());

        let content = fs::read_to_string(&out_path).unwrap();
        let config: crate::server::ServerConfig = toml::from_str(&content).unwrap();
        assert_eq!(config.domains, vec!["survival.example.com"]);
        assert!(config.motd.sleeping.is_some());
        assert!(config.motd.online.is_some());

        assert_eq!(report.converted, 1);
        assert_eq!(report.skipped, 0);
        assert!(report.warnings.iter().all(|w| w.file != "summary"));
    }

    #[test]
    fn test_migrate_counts_skipped_files_next_to_converted_ones() {
        let tmp = tempfile::tempdir().unwrap();
        let input = tmp.path().join("v1");
        let output = tmp.path().join("v2");
        fs::create_dir_all(&input).unwrap();
        fs::write(
            input.join("lobby.yaml"),
            "domains:\n  - lobby.example.com\naddresses:\n  - 127.0.0.1:25566\n",
        )
        .unwrap();
        fs::write(input.join("empty.yaml"), "domains: []\naddresses: []\n").unwrap();

        let report = migrate_directory(&input, &output).unwrap();
        assert_eq!(report.converted, 1);
        assert_eq!(report.skipped, 1);
        assert!(report.warnings.iter().any(|w| w.file == "empty.yaml"));
        assert!(report.warnings.iter().all(|w| w.file != "summary"));
    }

    #[test]
    fn test_migrate_invalid_dir() {
        let result = migrate_directory(Path::new("/nonexistent"), Path::new("/tmp/out"));
        assert!(result.is_err());
    }

    #[test]
    fn test_migrate_total_failure_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let input = tmp.path().join("v1");
        let output = tmp.path().join("v2");
        fs::create_dir_all(&input).unwrap();
        fs::write(input.join("broken.yaml"), ": not [ valid yaml").unwrap();

        let err = migrate_directory(&input, &output).unwrap_err();
        assert!(
            matches!(&err, ConfigError::NothingMigrated { skipped: 1, warnings } if warnings.len() == 1),
            "{err:?}"
        );
        assert!(err.to_string().contains("broken.yaml"));
    }

    #[test]
    fn test_migrate_invalid_dir_is_a_missing_directory() {
        let err = migrate_directory(Path::new("/nonexistent"), Path::new("/tmp/out")).unwrap_err();
        assert!(matches!(err, ConfigError::DirectoryNotFound(_)), "{err:?}");
    }

    #[test]
    fn test_migrate_empty_dir_is_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let input = tmp.path().join("v1");
        let output = tmp.path().join("v2");
        fs::create_dir_all(&input).unwrap();

        let report = migrate_directory(&input, &output).unwrap();
        assert_eq!(report.converted, 0);
        assert_eq!(report.skipped, 0);
        assert!(report.warnings.is_empty());
    }
}

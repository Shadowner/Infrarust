//! V1 → V2 directory migration.

use std::path::Path;

use super::convert::{
    MigrationSeverity, MigrationWarning, Warnings, convert_server, convert_v1_proxy_config,
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

    let mut report = MigrationReport::default();

    let mut paths = Vec::new();
    for result in std::fs::read_dir(input_dir).map_err(|source| ConfigError::ReadDir {
        path: input_dir.to_path_buf(),
        source,
    })? {
        match result {
            Ok(entry) => paths.push(entry.path()),
            Err(e) => report.warnings.push(MigrationWarning {
                severity: MigrationSeverity::Error,
                file: "unknown".to_string(),
                message: format!("Cannot read directory entry: {e}"),
            }),
        }
    }

    for path in &paths {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "yaml" && ext != "yml" {
            continue;
        }

        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        let mut warnings = Warnings::new(filename);
        match migrate_file(path, filename, output_dir, &mut warnings) {
            Ok(()) => report.converted += 1,
            Err(skip) => {
                match skip {
                    Skip::Empty => warnings.warn("Skipped: empty config (no domains or addresses)"),
                    Skip::Failed(message) => warnings.error(message),
                }
                report.skipped += 1;
            }
        }
        report.warnings.extend(warnings.into_vec());
    }

    if report.converted == 0 && report.skipped > 0 {
        return Err(ConfigError::NothingMigrated {
            skipped: report.skipped,
            warnings: report.warnings,
        });
    }

    Ok(report)
}

enum Skip {
    Empty,
    Failed(String),
}

fn migrate_file(
    path: &Path,
    filename: &str,
    output_dir: &Path,
    warnings: &mut Warnings,
) -> Result<(), Skip> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| Skip::Failed(format!("Cannot read file: {e}")))?;
    let v1: V1ServerConfig = serde_norway::from_str(&content)
        .map_err(|e| Skip::Failed(format!("YAML parse error: {e}")))?;

    if v1.domains.is_empty() && v1.addresses.is_empty() {
        return Err(Skip::Empty);
    }

    let config = convert_server(&v1, filename, warnings);
    let toml_content = toml::to_string_pretty(&config)
        .map_err(|e| Skip::Failed(format!("TOML serialization error: {e}")))?;

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    std::fs::write(output_dir.join(format!("{stem}.toml")), toml_content)
        .map_err(|e| Skip::Failed(format!("Cannot write output file: {e}")))
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
        serde_norway::from_str(&content).map_err(|source| ConfigError::ParseYaml {
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

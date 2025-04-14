// Re-export everything from the infrarust_config crate
pub use infrarust_config::{models::*, provider::*, service::*, *};

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    #[test]
    fn test_file_provider() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yml");
        let proxies_path = temp_dir.path().join("proxies");

        fs::create_dir(&proxies_path).unwrap();

        fs::write(&config_path, "bind: ':25565'\n").unwrap();
        fs::write(
            proxies_path.join("server1.yml"),
            "domains: ['example.com']\naddresses: ['127.0.0.1:25566']\n",
        )
        .unwrap();

        // let provider = FileProvider::new(
        //     config_path.to_str().unwrap().to_string(),
        //     proxies_path.to_str().unwrap().to_string(),
        //     FileType::Yaml,
        // );

        // let config = provider.load_config().unwrap();
        // assert!(!config.server_configs.is_empty());
    }
}

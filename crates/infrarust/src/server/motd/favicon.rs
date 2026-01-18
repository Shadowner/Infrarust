use base64::{Engine as _, engine::general_purpose};
use infrarust_config::models::logging::LogType;
use std::{fs, path::Path};
use tracing::debug;

pub const INFRARUST_FAVICON: &str = "data:image/png;base64,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";

pub fn parse_favicon(favicon: &str) -> Option<String> {
    if favicon.is_empty() {
        debug!(
            log_type = LogType::Motd.as_str(),
            "Favicon is empty, returning None"
        );
        return None;
    }

    if favicon.starts_with("data:image/png;base64,") {
        debug!(
            log_type = LogType::Motd.as_str(),
            "Favicon is already a base64 data URL"
        );
        return Some(favicon.to_string());
    }

    let image_data = if Path::new(favicon).is_absolute() {
        debug!(
            log_type = LogType::Motd.as_str(),
            "Trying absolute path: {}", favicon
        );
        fs::read(favicon).ok()
    } else {
        // relative path from current working directory
        debug!(
            log_type = LogType::Motd.as_str(),
            "Trying relative path: {}", favicon
        );
        fs::read(favicon)
            .or_else(|_| {
                // try from executable directory
                if let Ok(exe_path) = std::env::current_exe() {
                    if let Some(exe_dir) = exe_path.parent() {
                        let absolute_path = exe_dir.join(favicon);
                        debug!(
                            log_type = LogType::Motd.as_str(),
                            "Trying path relative to executable: {:?}", absolute_path
                        );
                        fs::read(absolute_path)
                    } else {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            "Cannot determine executable directory",
                        ))
                    }
                } else {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "Cannot determine executable path",
                    ))
                }
            })
            .ok()
    };

    if let Some(data) = image_data {
        let base64_data = general_purpose::STANDARD.encode(&data);
        debug!(
            log_type = LogType::Motd.as_str(),
            "Loaded favicon from path: {}", favicon
        );
        return Some(format!("data:image/png;base64,{}", base64_data));
    }

    debug!(
        log_type = LogType::Motd.as_str(),
        "Could not load favicon from any path: {}", favicon
    );
    None
}

pub fn get_default_favicon(include_default: bool) -> Option<String> {
    if include_default {
        Some(INFRARUST_FAVICON.to_string())
    } else {
        None
    }
}

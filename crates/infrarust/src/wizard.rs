//! Interactive quick-start wizard shown on first run (no `infrarust.toml` found).

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use dialoguer::{Confirm, Input, Select};
use infrarust_config::ProxyConfig;

pub enum WizardOutcome {
    Config(Box<ProxyConfig>),
    ExitClean,
}

struct WizardSettings {
    bind: SocketAddr,
    servers_dir: PathBuf,
    max_connections: u32,
    web_enabled: bool,
    enable_webui: bool,
    web_bind: String,
    web_port: u16,
    api_key: Option<String>,
    sample_server_domain: Option<String>,
    sample_server_address: Option<String>,
}

pub fn run(config_path: &Path) -> anyhow::Result<WizardOutcome> {
    infrarust_core::telemetry::formatter::print_banner();

    println!(
        "  {}\n",
        console::style("No configuration file found. Let's set one up!").bold()
    );

    let mode = Select::new()
        .with_prompt("How would you like to configure Infrarust?")
        .items([
            "Minimal configuration (recommended)",
            "Complete configuration reference (open docs)",
        ])
        .default(0)
        .interact()?;

    if mode == 1 {
        println!(
            "\n  Visit the configuration reference:\n  {}\n",
            console::style("https://infrarust.dev/v2/reference/config-schema")
                .underlined()
                .cyan()
        );
        println!(
            "  Create an {} file and run infrarust again.\n",
            console::style("infrarust.toml").bold()
        );
        return Ok(WizardOutcome::ExitClean);
    }

    println!();

    let bind: SocketAddr = Input::new()
        .with_prompt("Proxy listen address")
        .default("0.0.0.0:25565".parse().expect("valid default address"))
        .interact_text()?;

    let servers_dir: String = Input::new()
        .with_prompt("Server configs directory")
        .default("./servers".to_string())
        .interact_text()?;
    let servers_dir = PathBuf::from(servers_dir);

    let max_connections: u32 = Input::new()
        .with_prompt("Maximum simultaneous connections (0 = unlimited)")
        .default(0u32)
        .interact_text()?;

    let web_enabled = Confirm::new()
        .with_prompt("Enable web admin panel?")
        .default(true)
        .interact()?;

    let mut enable_webui = true;
    let mut web_bind = "127.0.0.1".to_string();
    let mut web_port: u16 = 8080;
    let mut api_key: Option<String> = None;

    if web_enabled {
        let web_mode = Select::new()
            .with_prompt("Web features")
            .items(["API + Web UI (recommended)", "API only"])
            .default(0)
            .interact()?;
        enable_webui = web_mode == 0;

        println!(
            "  {}",
            console::style(
                "127.0.0.1 = local only (secure, use a reverse proxy for external access)"
            )
            .dim()
        );
        println!(
            "  {}",
            console::style("0.0.0.0   = all interfaces (accessible from the network)").dim()
        );
        let web_bind_choice = Select::new()
            .with_prompt("Web bind address")
            .items([
                "127.0.0.1 (local only, recommended)",
                "0.0.0.0 (all interfaces)",
            ])
            .default(0)
            .interact()?;
        web_bind = if web_bind_choice == 0 {
            "127.0.0.1".to_string()
        } else {
            "0.0.0.0".to_string()
        };

        web_port = Input::new()
            .with_prompt("Web listen port")
            .default(8080u16)
            .validate_with(|input: &u16| -> Result<(), String> {
                if *input == 0 {
                    return Err("Port must be between 1 and 65535".to_string());
                }
                if *input == bind.port() {
                    return Err(format!(
                        "Web port conflicts with proxy bind port ({})",
                        bind.port()
                    ));
                }
                Ok(())
            })
            .interact_text()?;

        let key = uuid::Uuid::new_v4().to_string();
        println!();
        println!(
            "  {} {}",
            console::style("Generated API key:").bold(),
            console::style(&key).green().bold()
        );
        println!();
        println!(
            "  {} Save this key! It is required to access the web panel.",
            console::style("WARNING:").yellow().bold()
        );
        println!(
            "  It will be written to the {} section of {}",
            console::style("[web]").dim(),
            console::style("infrarust.toml").dim()
        );
        println!();
        api_key = Some(key);
    }

    let sample_domain: String = Input::new()
        .with_prompt("Domain to route (leave empty to skip)")
        .allow_empty(true)
        .default(String::new())
        .interact_text()?;

    let (sample_server_domain, sample_server_address) = if sample_domain.is_empty() {
        (None, None)
    } else {
        let address: String = Input::new()
            .with_prompt("Backend server address")
            .default("127.0.0.1:25565".to_string())
            .interact_text()?;
        (Some(sample_domain), Some(address))
    };

    let settings = WizardSettings {
        bind,
        servers_dir,
        max_connections,
        web_enabled,
        enable_webui,
        web_bind,
        web_port,
        api_key,
        sample_server_domain,
        sample_server_address,
    };

    println!();
    print_summary(&settings);
    println!();

    let confirmed = Confirm::new()
        .with_prompt("Write configuration?")
        .default(true)
        .interact()?;

    if !confirmed {
        bail!("Configuration cancelled by user");
    }

    write_files(config_path, &settings)?;

    println!(
        "\n  {} Configuration written to {}\n",
        console::style("✓").green().bold(),
        console::style(config_path.display()).bold()
    );

    let content = std::fs::read_to_string(config_path)
        .with_context(|| format!("cannot read generated config: {}", config_path.display()))?;

    let config: ProxyConfig = toml::from_str(&content).with_context(|| {
        format!(
            "generated config is invalid TOML: {}",
            config_path.display()
        )
    })?;

    infrarust_config::validate_proxy_config(&config)
        .context("generated configuration failed validation")?;

    Ok(WizardOutcome::Config(Box::new(config)))
}

fn write_files(config_path: &Path, settings: &WizardSettings) -> anyhow::Result<()> {
    let toml = generate_config_toml(settings);
    std::fs::write(config_path, toml).context("failed to write infrarust.toml")?;

    std::fs::create_dir_all(&settings.servers_dir)
        .with_context(|| format!("failed to create {}", settings.servers_dir.display()))?;

    if let (Some(domain), Some(address)) = (
        &settings.sample_server_domain,
        &settings.sample_server_address,
    ) {
        let filename = sanitize_filename(domain);
        let server_path = settings.servers_dir.join(format!("{filename}.toml"));
        let server_toml = generate_server_toml(domain, address);
        std::fs::write(&server_path, server_toml)
            .with_context(|| format!("failed to write {}", server_path.display()))?;
        println!(
            "  {} Server config written to {}",
            console::style("✓").green().bold(),
            console::style(server_path.display()).bold()
        );
    }

    Ok(())
}

fn generate_config_toml(settings: &WizardSettings) -> String {
    let mut config = format!(
        r#"# Infrarust proxy configuration
# Generated by the quick-start wizard
# Full reference: https://infrarust.dev/v2/reference/config-schema

bind = "{bind}"
servers_dir = "{servers_dir}"
max_connections = {max_connections}
"#,
        bind = settings.bind,
        servers_dir = settings.servers_dir.display(),
        max_connections = settings.max_connections,
    );

    if settings.web_enabled {
        config.push_str(&format!(
            r#"
[web]
enable_api = true
enable_webui = {webui}
bind = "{web_bind}:{port}"
"#,
            webui = settings.enable_webui,
            web_bind = settings.web_bind,
            port = settings.web_port,
        ));

        if let Some(key) = &settings.api_key {
            config.push_str(&format!("api_key = \"{key}\"\n"));
        }
    }

    config
}

fn generate_server_toml(domain: &str, address: &str) -> String {
    format!(
        r#"# Server configuration for {domain}
# Generated by the quick-start wizard
# Full reference: https://infrarust.dev/v2/reference/config-schema

domains = ["{domain}"]
addresses = ["{address}"]
proxy_mode = "passthrough"
"#
    )
}

fn print_summary(settings: &WizardSettings) {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    table.set_header(vec!["Setting", "Value"]);

    table.add_row(vec!["Bind address", &settings.bind.to_string()]);
    table.add_row(vec![
        "Servers directory",
        &settings.servers_dir.display().to_string(),
    ]);
    table.add_row(vec![
        "Max connections",
        &if settings.max_connections == 0 {
            "unlimited".to_string()
        } else {
            settings.max_connections.to_string()
        },
    ]);

    if settings.web_enabled {
        let features = if settings.enable_webui {
            "API + Web UI"
        } else {
            "API only"
        };
        table.add_row(vec!["Web admin", &format!("enabled ({features})")]);
        table.add_row(vec![
            "Web listen",
            &format!("{}:{}", settings.web_bind, settings.web_port),
        ]);
    } else {
        table.add_row(vec!["Web admin", "disabled"]);
    }

    if let Some(domain) = &settings.sample_server_domain {
        let address = settings.sample_server_address.as_deref().unwrap_or("—");
        table.add_row(vec!["Sample server", &format!("{domain} -> {address}")]);
    }

    println!("{table}");
}

fn sanitize_filename(domain: &str) -> String {
    let sanitized: String = domain
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect();
    let sanitized = sanitized.replace("..", "_");
    let sanitized = sanitized.trim_start_matches('.').to_string();
    if sanitized.is_empty() {
        "unnamed".to_string()
    } else {
        sanitized
    }
}

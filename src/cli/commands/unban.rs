use std::net::IpAddr;
use std::str::FromStr;
use std::sync::Arc;

use crate::cli::command::{Command, CommandFuture};
use crate::cli::format as fmt;
use crate::core::shared_component::SharedComponent;
use crate::security::BanSystemAdapter;
use crate::security::filter::FilterError;
use crate::{Filter, with_filter_result, with_filter_void};
use tracing::debug;

pub struct UnbanCommand {
    shared: Arc<SharedComponent>,
}

impl UnbanCommand {
    pub fn new(shared: Arc<SharedComponent>) -> Self {
        Self { shared }
    }

    async fn unban_username(&self, username: &str) -> Result<bool, FilterError> {
        let registry = self.shared.filter_registry();
        let result = match with_filter_result!(
            registry,
            "global_ban_system",
            BanSystemAdapter,
            async |filter: &BanSystemAdapter| {
                debug!("Attempting to remove ban using global_ban_system");
                match filter.remove_ban_by_username(username, "system").await {
                    Ok(removed) => Ok(removed),
                    Err(e) => Err(e),
                }
            },
            false
        ) {
            Ok(result) => Some(result),
            Err(_) => None,
        };

        match result {
            Some(true) => return Ok(true),
            None => {
                return Err(FilterError::NotFound(
                    "Ban filter is not registered".to_string(),
                ));
            }
            _ => {}
        }

        Ok(false)
    }

    async fn unban_player(&self, args: Vec<String>) -> String {
        if args.is_empty() {
            return fmt::error(
                "Usage: unban [--ip/-ip <address> | --username/-u <username> | --uuid/-id <uuid>]",
            )
            .to_string();
        }

        let mut ip = None;
        let mut username = None;
        let mut uuid = None;

        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--ip" | "-ip" => {
                    if i + 1 < args.len() {
                        match IpAddr::from_str(&args[i + 1]) {
                            Ok(addr) => ip = Some(addr),
                            Err(_) => {
                                return fmt::error(&format!("Invalid IP address: {}", args[i + 1]))
                                    .to_string();
                            }
                        }
                        i += 2;
                    } else {
                        return fmt::error("Missing IP address after --ip/-ip").to_string();
                    }
                }
                "--username" | "-u" => {
                    if i + 1 < args.len() {
                        username = Some(args[i + 1].clone());
                        i += 2;
                    } else {
                        return fmt::error("Missing username after --username/-u").to_string();
                    }
                }
                "--uuid" | "-id" => {
                    if i + 1 < args.len() {
                        uuid = Some(args[i + 1].clone());
                        i += 2;
                    } else {
                        return fmt::error("Missing UUID after --uuid/-id").to_string();
                    }
                }
                _ => {
                    return fmt::error(&format!("Unknown option: {}", args[i])).to_string();
                }
            }
        }

        // Make sure only one identifier is specified
        let count = [ip.is_some(), username.is_some(), uuid.is_some()]
            .iter()
            .filter(|&&b| b)
            .count();

        if count == 0 {
            return fmt::error("At least one identifier (IP, username, or UUID) is required")
                .to_string();
        }

        if count > 1 {
            return fmt::error("Only one identifier (IP, username, or UUID) should be specified")
                .to_string();
        }

        let registry = self.shared.filter_registry();

        if let Some(ip) = ip {
            let result = with_filter_result!(
                registry,
                "global_ban_system",
                BanSystemAdapter,
                async |filter: &BanSystemAdapter| {
                    match filter.remove_ban_by_ip(&ip, "system").await {
                        Ok(removed) => Ok(removed),
                        Err(e) => Err(e),
                    }
                },
                false
            );

            match result {
                Ok(removed) => {
                    if removed {
                        fmt::success(&format!(
                            "Successfully removed ban for IP: {}",
                            fmt::entity(&ip.to_string())
                        ))
                        .to_string()
                    } else {
                        fmt::warning(&format!(
                            "No ban found for IP: {}",
                            fmt::entity(&ip.to_string())
                        ))
                        .to_string()
                    }
                }
                Err(e) => match e {
                    FilterError::NotFound(_) => fmt::error(
                        "Ban filter is not registered. Enable ban filter in configuration.",
                    )
                    .to_string(),
                    _ => fmt::error(&format!("Failed to remove ban: {}", e)).to_string(),
                },
            }
        } else if let Some(username) = username {
            match self.unban_username(&username).await {
                Ok(removed) => {
                    if removed {
                        // Refresh ban filter
                        with_filter_void!(
                            registry,
                            "global_ban_system",
                            BanSystemAdapter,
                            async |filter: &BanSystemAdapter| {
                                let _ = filter.refresh().await;
                            }
                        );

                        fmt::success(&format!(
                            "Successfully removed ban for username: {}",
                            fmt::entity(&username)
                        ))
                        .to_string()
                    } else {
                        fmt::warning(&format!(
                            "No ban found for username: {}",
                            fmt::entity(&username)
                        ))
                        .to_string()
                    }
                }
                Err(e) => match e {
                    FilterError::NotFound(_) => fmt::error(
                        "Ban filter is not registered. Enable ban filter in configuration.",
                    )
                    .to_string(),
                    _ => fmt::error(&format!("Failed to remove ban: {}", e)).to_string(),
                },
            }
        } else if let Some(uuid) = uuid {
            let result = with_filter_result!(
                registry,
                "global_ban_system",
                BanSystemAdapter,
                async |filter: &BanSystemAdapter| {
                    match filter.remove_ban_by_uuid(&uuid, "system").await {
                        Ok(removed) => Ok(removed),
                        Err(e) => Err(e),
                    }
                },
                false
            );

            match result {
                Ok(removed) => {
                    if removed {
                        fmt::success(&format!(
                            "Successfully removed ban for UUID: {}",
                            fmt::entity(&uuid)
                        ))
                        .to_string()
                    } else {
                        fmt::warning(&format!("No ban found for UUID: {}", fmt::entity(&uuid)))
                            .to_string()
                    }
                }
                Err(e) => match e {
                    FilterError::NotFound(_) => fmt::error(
                        "Ban filter is not registered. Enable ban filter in configuration.",
                    )
                    .to_string(),
                    _ => fmt::error(&format!("Failed to remove ban: {}", e)).to_string(),
                },
            }
        } else {
            // Should never reach here due to earlier checks
            fmt::error("No identifier provided. Use --ip, --username, or --uuid").to_string()
        }
    }
}

impl Command for UnbanCommand {
    fn name(&self) -> &'static str {
        "unban"
    }

    fn description(&self) -> &'static str {
        "Removes a ban by IP address, username, or UUID. Use --ip, --username, or --uuid flags."
    }

    fn execute(&self, args: Vec<String>) -> CommandFuture {
        debug!("Executing unban command with args: {:?}", args);
        let shared = self.shared.clone();

        Box::pin(async move {
            let unban_cmd = UnbanCommand { shared };
            unban_cmd.unban_player(args).await
        })
    }
}

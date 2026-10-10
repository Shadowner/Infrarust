use infrarust_api::types::ServerId;
use infrarust_config::ServerConfig;
use infrarust_server_manager::{ServerManagerError, ServerState};
use tokio_util::sync::CancellationToken;

use crate::services::ProxyServices;
use crate::session::kick::Kick;

const STOPPING_MESSAGE: &str = "Server is shutting down, please try again later.";
const START_TIMEOUT_MESSAGE: &str = "Server failed to start in time. Please try again.";
const UNAVAILABLE_MESSAGE: &str = "Server is unavailable. Please try again later.";

#[derive(Debug)]
pub(crate) struct Unavailable {
    message: &'static str,
    error: String,
}

impl Unavailable {
    fn new(message: &'static str, error: impl Into<String>) -> Self {
        Self {
            message,
            error: error.into(),
        }
    }

    pub(crate) fn into_kick(self, server: ServerId) -> Kick {
        Kick::unavailable(server, self.message, self.error)
    }
}

pub(crate) async fn wake(
    services: &ProxyServices,
    config: &ServerConfig,
    cancel: &CancellationToken,
) -> Result<bool, Unavailable> {
    let Some(manager) = services.server_manager.as_ref() else {
        return Ok(false);
    };
    if config.server_manager.is_none() {
        return Ok(false);
    }
    let managed_id = config.effective_id();
    let Some(state) = manager.get_state(&managed_id) else {
        return Ok(false);
    };
    match state {
        ServerState::Stopping => Err(Unavailable::new(STOPPING_MESSAGE, "the server is stopping")),
        ServerState::Sleeping | ServerState::Crashed | ServerState::Starting => {
            let started = tokio::select! {
                biased;
                () = cancel.cancelled() => {
                    return Err(Unavailable::new(
                        UNAVAILABLE_MESSAGE,
                        "the session ended while the server was starting",
                    ));
                }
                started = manager.ensure_started(&managed_id) => started,
            };
            match started {
                Ok(()) => {
                    let window = config.balance_config().slow_start.unwrap_or_default();
                    for address in &config.addresses {
                        services
                            .backend_health
                            .mark_warming(&address.address, window);
                    }
                    Ok(true)
                }
                Err(error @ ServerManagerError::StartTimeout { .. }) => {
                    Err(Unavailable::new(START_TIMEOUT_MESSAGE, error.to_string()))
                }
                Err(error) => {
                    tracing::error!(server = %managed_id, "server manager error: {error}");
                    Err(Unavailable::new(UNAVAILABLE_MESSAGE, error.to_string()))
                }
            }
        }
        _ => Ok(false),
    }
}

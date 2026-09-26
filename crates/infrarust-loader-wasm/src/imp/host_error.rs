use infrarust_api::command::CommandError;
use infrarust_api::error::{PlayerError, ServiceError};
use infrarust_api::filter::FilterRegistryError;
use infrarust_api::limbo::LimboHandlerError;
use infrarust_api::messaging::MessagingError;
use infrarust_api::permissions::Capability;
use infrarust_api::services::config_service::ConfigWriteError;
use infrarust_api::services::load_balancer::LbError;
use infrarust_plugin_wit::arena::ArenaError;

use crate::bindings::infrarust::plugin::types::{ErrorKind, HostError};

pub(crate) type HostResult<T> = Result<T, HostError>;

pub(crate) fn host_error(kind: ErrorKind, message: impl Into<String>) -> HostError {
    HostError {
        kind,
        message: message.into(),
    }
}

pub(crate) fn missing_capability(capability: Capability) -> HostError {
    host_error(
        ErrorKind::PermissionDenied,
        format!("missing capability: {}", capability.to_kebab()),
    )
}

pub(crate) fn invalid_component(error: &ArenaError) -> HostError {
    host_error(
        ErrorKind::InvalidArgument,
        format!("invalid text component: {error}"),
    )
}

pub(crate) fn player_gone(id: u64) -> HostError {
    host_error(ErrorKind::PlayerGone, format!("player {id} is not online"))
}

pub(crate) fn no_services() -> HostError {
    host_error(
        ErrorKind::Unavailable,
        "host services are not available while the plugin is inspected",
    )
}

pub(crate) fn timed_out(message: String) -> HostError {
    host_error(ErrorKind::Timeout, message)
}

pub(crate) fn player_error(error: PlayerError) -> HostError {
    let kind = match &error {
        PlayerError::Disconnected => ErrorKind::PlayerGone,
        PlayerError::NotActive | PlayerError::NoBackend | PlayerError::WouldDeadlock => {
            ErrorKind::InvalidState
        }
        PlayerError::ServerNotFound(_) => ErrorKind::NotFound,
        PlayerError::MessageTooLarge { .. } => ErrorKind::InvalidArgument,
        PlayerError::SendFailed(_) | PlayerError::SwitchFailed(_) => ErrorKind::Unavailable,
        PlayerError::Unsupported(_) => ErrorKind::Unsupported,
        PlayerError::InvalidArgument(_) => ErrorKind::InvalidArgument,
        PlayerError::Denied(_) => ErrorKind::PermissionDenied,
        _ => ErrorKind::Internal,
    };
    host_error(kind, error.to_string())
}

pub(crate) fn service_error(error: ServiceError) -> HostError {
    let kind = match &error {
        ServiceError::NotFound(_) => ErrorKind::NotFound,
        ServiceError::Unavailable(_) => ErrorKind::Unavailable,
        ServiceError::AlreadyProvided { .. } => ErrorKind::Conflict,
        ServiceError::OperationFailed(_) => ErrorKind::Internal,
        _ => ErrorKind::Internal,
    };
    let message = match error {
        ServiceError::NotFound(message)
        | ServiceError::Unavailable(message)
        | ServiceError::OperationFailed(message) => message,
        other => other.to_string(),
    };
    host_error(kind, message)
}

pub(crate) fn command_error(error: &CommandError) -> HostError {
    let kind = match error {
        CommandError::Reserved(_) | CommandError::OwnedBy { .. } => ErrorKind::Conflict,
        CommandError::InvalidName(_) => ErrorKind::InvalidArgument,
        CommandError::NotOwned(_) => ErrorKind::NotFound,
        _ => ErrorKind::Internal,
    };
    host_error(kind, error.to_string())
}

pub(crate) fn filter_error(error: &FilterRegistryError) -> HostError {
    let kind = match error {
        FilterRegistryError::OwnedBy { .. } => ErrorKind::Conflict,
        FilterRegistryError::NotFound(_) => ErrorKind::NotFound,
        _ => ErrorKind::Internal,
    };
    host_error(kind, error.to_string())
}

pub(crate) fn messaging_error(error: &MessagingError) -> HostError {
    let kind = match error {
        MessagingError::NoCarrier => ErrorKind::Unavailable,
        MessagingError::TooLarge { .. } => ErrorKind::InvalidArgument,
        _ => ErrorKind::Internal,
    };
    host_error(kind, error.to_string())
}

pub(crate) fn balancer_error(error: &LbError) -> HostError {
    let kind = match error {
        LbError::UnknownServer(_) | LbError::UnknownAddress { .. } => ErrorKind::NotFound,
        _ => ErrorKind::Internal,
    };
    host_error(kind, error.to_string())
}

pub(crate) fn config_write_error(error: &ConfigWriteError) -> HostError {
    let kind = match error {
        ConfigWriteError::PermissionDenied => ErrorKind::PermissionDenied,
        ConfigWriteError::Parse(_) | ConfigWriteError::Validation(_) => ErrorKind::InvalidArgument,
        ConfigWriteError::Io(_) => ErrorKind::Unavailable,
        _ => ErrorKind::Internal,
    };
    host_error(kind, error.to_string())
}

pub(crate) fn limbo_error(error: &LimboHandlerError) -> HostError {
    host_error(ErrorKind::Conflict, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_errors_map_to_the_contract_kinds() {
        assert_eq!(
            player_error(PlayerError::Disconnected).kind,
            ErrorKind::PlayerGone
        );
        assert_eq!(
            player_error(PlayerError::ServerNotFound("x".into())).kind,
            ErrorKind::NotFound
        );
        assert_eq!(
            player_error(PlayerError::WouldDeadlock).kind,
            ErrorKind::InvalidState
        );
        let failed = service_error(ServiceError::OperationFailed("disk full".into()));
        assert_eq!(failed.kind, ErrorKind::Internal);
        assert_eq!(failed.message, "disk full");
        assert_eq!(
            command_error(&CommandError::OwnedBy {
                name: "x".into(),
                plugin: "p".into()
            })
            .kind,
            ErrorKind::Conflict
        );
        assert_eq!(
            filter_error(&FilterRegistryError::OwnedBy {
                id: "x".into(),
                owner: "p".into()
            })
            .kind,
            ErrorKind::Conflict
        );
        assert_eq!(
            filter_error(&FilterRegistryError::NotFound("x".into())).kind,
            ErrorKind::NotFound
        );
        assert_eq!(
            missing_capability(Capability::Ban).message,
            "missing capability: ban"
        );
        assert_eq!(
            player_error(PlayerError::Unsupported("transfers".into())).kind,
            ErrorKind::Unsupported
        );
        assert_eq!(
            player_error(PlayerError::InvalidArgument("key".into())).kind,
            ErrorKind::InvalidArgument
        );
        assert_eq!(
            messaging_error(&MessagingError::NoCarrier).kind,
            ErrorKind::Unavailable
        );
        assert_eq!(
            config_write_error(&ConfigWriteError::Validation("bad".into())).kind,
            ErrorKind::InvalidArgument
        );
    }
}

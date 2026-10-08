use infrarust_api::command::CommandError;
use infrarust_api::error::{PlayerError, ServiceError};
use infrarust_api::filter::FilterRegistryError;
use infrarust_api::limbo::LimboHandlerError;
use infrarust_api::messaging::MessagingError;
use infrarust_api::permissions::{Capability, PermissionNodeError};
use infrarust_api::services::config_service::ConfigWriteError;
use infrarust_api::services::load_balancer::LbError;
use infrarust_plugin_wit::arena::ArenaError;

use infrarust_plugin_common::ErrorKind as Kind;

use crate::bindings::infrarust::plugin::types as wt;
use crate::bindings::infrarust::plugin::types::{ErrorKind, HostError};
use crate::convert::wit_enum_map;
use crate::deadline::HostCallTimeout;

wit_enum_map!(kind_to_wit: Kind => wt::ErrorKind {
    InvalidArgument, NotFound, PermissionDenied, Unavailable, Timeout, PlayerGone, Conflict,
    InvalidState, Unsupported, Internal, LimitExceeded,
});

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

pub(crate) fn limit_exceeded(noun: &str, key: &str, limit: usize) -> HostError {
    host_error(
        ErrorKind::LimitExceeded,
        format!("quota reached: the plugin may hold at most {limit} {noun} (quotas.{key})"),
    )
}

pub(crate) fn timed_out(expired: HostCallTimeout) -> HostError {
    host_error(ErrorKind::Timeout, expired.to_string())
}

pub(crate) fn player_error(error: PlayerError) -> HostError {
    host_error(kind_to_wit(error.kind()), error.to_string())
}

pub(crate) fn service_error(error: ServiceError) -> HostError {
    let kind = kind_to_wit(error.kind());
    let message = match error {
        ServiceError::NotFound(message)
        | ServiceError::Unavailable(message)
        | ServiceError::OperationFailed(message) => message,
        other => other.to_string(),
    };
    host_error(kind, message)
}

pub(crate) fn command_error(error: &CommandError) -> HostError {
    host_error(kind_to_wit(error.kind()), error.to_string())
}

pub(crate) fn permission_node_error(error: &PermissionNodeError) -> HostError {
    host_error(kind_to_wit(error.kind()), error.to_string())
}

pub(crate) fn filter_error(error: &FilterRegistryError) -> HostError {
    host_error(kind_to_wit(error.kind()), error.to_string())
}

pub(crate) fn messaging_error(error: &MessagingError) -> HostError {
    host_error(kind_to_wit(error.kind()), error.to_string())
}

pub(crate) fn balancer_error(error: &LbError) -> HostError {
    host_error(kind_to_wit(error.kind()), error.to_string())
}

pub(crate) fn config_write_error(error: &ConfigWriteError) -> HostError {
    host_error(kind_to_wit(error.kind()), error.to_string())
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
            permission_node_error(&PermissionNodeError::InvalidName("a..b".into())).kind,
            ErrorKind::InvalidArgument
        );
        assert_eq!(
            permission_node_error(&PermissionNodeError::Reserved("infrarust.fly".into())).kind,
            ErrorKind::Conflict
        );
        let owned = permission_node_error(&PermissionNodeError::OwnedBy {
            name: "warps.use".into(),
            plugin: "warps".into(),
        });
        assert_eq!(owned.kind, ErrorKind::Conflict);
        assert_eq!(
            owned.message,
            "'warps.use' is already registered by plugin 'warps'"
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
        let quota = limit_exceeded("commands", "commands", 256);
        assert_eq!(quota.kind, ErrorKind::LimitExceeded);
        assert_eq!(
            quota.message,
            "quota reached: the plugin may hold at most 256 commands (quotas.commands)"
        );
    }
}

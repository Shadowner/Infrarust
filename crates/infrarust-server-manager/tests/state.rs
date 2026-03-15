use infrarust_server_manager::{ProviderStatus, ServerState};

#[test]
fn test_online_is_joinable() {
    assert!(ServerState::Online.is_joinable());
}

#[test]
fn test_sleeping_not_joinable() {
    assert!(!ServerState::Sleeping.is_joinable());
}

#[test]
fn test_sleeping_is_startable() {
    assert!(ServerState::Sleeping.is_startable());
}

#[test]
fn test_crashed_is_startable() {
    assert!(ServerState::Crashed.is_startable());
}

#[test]
fn test_online_not_startable() {
    assert!(!ServerState::Online.is_startable());
}

#[test]
fn test_starting_should_wait() {
    assert!(ServerState::Starting.should_wait());
}

#[test]
fn test_stopping_not_joinable() {
    assert!(!ServerState::Stopping.is_joinable());
}

#[test]
fn test_stopping_not_startable() {
    assert!(!ServerState::Stopping.is_startable());
}

#[test]
fn test_provider_status_running_to_online() {
    assert_eq!(
        ServerState::from(ProviderStatus::Running),
        ServerState::Online
    );
}

#[test]
fn test_provider_status_stopped_to_sleeping() {
    assert_eq!(
        ServerState::from(ProviderStatus::Stopped),
        ServerState::Sleeping
    );
}

#[test]
fn test_provider_status_starting_to_starting() {
    assert_eq!(
        ServerState::from(ProviderStatus::Starting),
        ServerState::Starting
    );
}

#[test]
fn test_provider_status_stopping_to_stopping() {
    assert_eq!(
        ServerState::from(ProviderStatus::Stopping),
        ServerState::Stopping
    );
}

#[test]
fn test_provider_status_unknown_to_unknown() {
    assert_eq!(
        ServerState::from(ProviderStatus::Unknown),
        ServerState::Unknown
    );
}

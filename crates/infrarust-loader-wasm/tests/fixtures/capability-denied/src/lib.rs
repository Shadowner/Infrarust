wit_bindgen::generate!({
    world: "plugin",
    path: "../../../../infrarust-plugin-wit/wit",
    generate_all,
});

use crate::infrarust::plugin::ban_service;
use crate::infrarust::plugin::types::{BanTarget, ServiceError};

struct Component;

fn outcome() -> String {
    match ban_service::is_banned(&BanTarget::Username("nobody".to_string())) {
        Ok(banned) => format!("ok: {banned}"),
        Err(ServiceError::NotFound(message)) => format!("not-found: {message}"),
        Err(ServiceError::OperationFailed(message)) => format!("operation-failed: {message}"),
        Err(ServiceError::Unavailable(message)) => format!("unavailable: {message}"),
    }
}

fixture_common::raw_fixture!(
    Component,
    id: "capability-denied",
    name: "Capability Denied Fixture",
    description: None,
    on_enable: {
        std::fs::write("ban.txt", outcome()).map_err(|e| e.to_string())
    }
);

export!(Component);

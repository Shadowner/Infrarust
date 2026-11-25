use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use infrarust_api::services::{InstallRequest, ProxyUpdates, UpdateRequestError};

use crate::dto::requests::InstallUpdateRequest;
use crate::dto::update::UpdateStatusResponse;
use crate::error::ApiError;
use crate::response::{ApiResponse, MutationResult, accepted, ok};
use crate::state::ApiState;

const CHECK_TIMEOUT: Duration = Duration::from_secs(25);
const MAX_VERSION_LEN: usize = 64;

fn service(state: &ApiState) -> Result<&Arc<dyn ProxyUpdates>, ApiError> {
    state.updates.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable("updates are not available on this proxy".into())
    })
}

pub async fn status(
    State(state): State<Arc<ApiState>>,
) -> Result<Json<ApiResponse<UpdateStatusResponse>>, ApiError> {
    let response = match &state.updates {
        Some(updates) => UpdateStatusResponse::from_status(updates.status()),
        None => UpdateStatusResponse::disabled(state.proxy_version.clone()),
    };
    Ok(ok(response))
}

pub async fn check(
    State(state): State<Arc<ApiState>>,
) -> Result<Json<ApiResponse<UpdateStatusResponse>>, ApiError> {
    let updates = service(&state)?;
    let status = tokio::time::timeout(CHECK_TIMEOUT, updates.check_now())
        .await
        .map_err(|_| {
            ApiError::ServiceUnavailable(format!(
                "the update check did not finish within {}s, the cached status is unchanged",
                CHECK_TIMEOUT.as_secs()
            ))
        })?;
    Ok(ok(UpdateStatusResponse::from_status(status)))
}

pub async fn install(
    State(state): State<Arc<ApiState>>,
    body: Option<Json<InstallUpdateRequest>>,
) -> Result<(StatusCode, Json<ApiResponse<MutationResult>>), ApiError> {
    let updates = service(&state)?;
    let version = requested_version(body.map(|Json(body)| body).unwrap_or_default())?;

    tracing::info!(
        target: "audit",
        action = "proxy_update_install",
        source = "admin_api",
        version = version.as_deref().unwrap_or("latest"),
        "Proxy update installation requested via Admin API"
    );

    updates
        .start_install(InstallRequest {
            version: version.clone(),
            then_restart: false,
        })
        .map_err(install_error)?;

    let target = version.or_else(|| updates.status().available.map(|update| update.version));
    let message = match target {
        Some(version) => format!("Installing Infrarust {version}"),
        None => "Installing the latest update".to_owned(),
    };
    Ok(accepted(MutationResult {
        success: true,
        message,
        details: None,
    }))
}

fn requested_version(body: InstallUpdateRequest) -> Result<Option<String>, ApiError> {
    let Some(version) = body.version else {
        return Ok(None);
    };
    let version = version.trim();
    if version.is_empty() {
        return Err(ApiError::BadRequest("version must not be empty".into()));
    }
    if version.len() > MAX_VERSION_LEN {
        return Err(ApiError::BadRequest(format!(
            "version too long (max {MAX_VERSION_LEN} bytes)"
        )));
    }
    if semver::Version::parse(version.strip_prefix('v').unwrap_or(version)).is_err() {
        return Err(ApiError::BadRequest(format!(
            "version must be a release version such as 2.0.0, got {version:?}"
        )));
    }
    Ok(Some(version.to_owned()))
}

fn install_error(error: UpdateRequestError) -> ApiError {
    match error {
        UpdateRequestError::Busy => ApiError::Conflict(error.to_string()),
        UpdateRequestError::NothingToInstall => ApiError::NotFound(error.to_string()),
        UpdateRequestError::Refused(reason) => ApiError::Conflict(reason),
    }
}

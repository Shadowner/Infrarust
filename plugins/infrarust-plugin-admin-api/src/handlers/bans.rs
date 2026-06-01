use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;

use infrarust_api::services::ban_service::BanTarget;

use crate::dto::ban::{BanCheckResponse, BanResponse};
use crate::dto::requests::{BanTargetRequest, CreateBanRequest};
use crate::error::ApiError;
use crate::response::{
    ApiResponse, MutationResult, PaginatedResponse, PaginationParams, created, default_page,
    default_per_page, mutation_ok, ok,
};
use crate::state::ApiState;
use crate::util::{ban_target_type_str, parse_ban_target};

#[derive(Debug, Deserialize)]
pub struct BanListQuery {
    #[serde(default = "default_page")]
    pub page: usize,
    #[serde(default = "default_per_page")]
    pub per_page: usize,
    pub target_type: Option<String>,
    pub source: Option<String>,
}

pub async fn list(
    State(state): State<Arc<ApiState>>,
    Query(query): Query<BanListQuery>,
) -> Result<Json<PaginatedResponse<BanResponse>>, ApiError> {
    let mut pagination = PaginationParams {
        page: query.page,
        per_page: query.per_page,
    };
    pagination.normalize();

    let mut bans = state
        .ban_service
        .get_all_bans()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to fetch bans: {e}")))?;

    if let Some(ref tt) = query.target_type {
        bans.retain(|b| ban_target_type_str(&b.target) == tt.as_str());
    }

    if let Some(ref src) = query.source {
        bans.retain(|b| b.source == *src);
    }

    bans.sort_by_key(|b| std::cmp::Reverse(b.created_at));

    let responses: Vec<BanResponse> = bans.iter().map(BanResponse::from_entry).collect();

    Ok(Json(pagination.apply(responses)))
}

pub async fn check(
    State(state): State<Arc<ApiState>>,
    Path((target_type, value)): Path<(String, String)>,
) -> Result<Json<ApiResponse<BanCheckResponse>>, ApiError> {
    let target = parse_ban_target(&target_type, &value)?;

    let ban_entry = state
        .ban_service
        .get_ban(&target)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to check ban: {e}")))?;

    let response = match ban_entry {
        Some(entry) if !entry.is_expired() => BanCheckResponse {
            banned: true,
            ban: Some(BanResponse::from_entry(&entry)),
        },
        _ => BanCheckResponse {
            banned: false,
            ban: None,
        },
    };

    Ok(ok(response))
}

pub async fn create(
    State(state): State<Arc<ApiState>>,
    Json(body): Json<CreateBanRequest>,
) -> Result<(axum::http::StatusCode, Json<ApiResponse<MutationResult>>), ApiError> {
    let target = match body.target {
        BanTargetRequest::Ip(ref ip) => ip
            .parse()
            .map(BanTarget::Ip)
            .map_err(|_| ApiError::BadRequest(format!("Invalid IP address: {ip}")))?,
        BanTargetRequest::Username(ref name) => BanTarget::Username(name.clone()),
        BanTargetRequest::Uuid(ref uuid) => uuid
            .parse()
            .map(BanTarget::Uuid)
            .map_err(|_| ApiError::BadRequest(format!("Invalid UUID: {uuid}")))?,
    };

    if let Some(ref reason) = body.reason
        && reason.len() > 256
    {
        return Err(ApiError::BadRequest(
            "ban reason too long (max 256 characters)".into(),
        ));
    }

    let duration = body.duration_seconds.map(Duration::from_secs);

    tracing::info!(
        target: "audit",
        action = "ban",
        ban_target = %target,
        reason = ?body.reason,
        source = "admin_api",
        "Ban created via Admin API"
    );

    state
        .ban_service
        .ban(target, body.reason, duration)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create ban: {e}")))?;

    Ok(created(MutationResult {
        success: true,
        message: "Ban created".into(),
        details: None,
    }))
}

pub async fn delete(
    State(state): State<Arc<ApiState>>,
    Path((target_type, value)): Path<(String, String)>,
) -> Result<Json<ApiResponse<MutationResult>>, ApiError> {
    let target = parse_ban_target(&target_type, &value)?;

    tracing::info!(
        target: "audit",
        action = "unban",
        ban_target = %target,
        source = "admin_api",
        "Ban removed via Admin API"
    );

    let removed = state
        .ban_service
        .unban(&target)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to remove ban: {e}")))?;

    if removed {
        Ok(mutation_ok("Ban removed"))
    } else {
        Err(ApiError::NotFound(format!(
            "No active ban found for {target_type}/{value}"
        )))
    }
}

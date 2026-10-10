use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;

use infrarust_api::services::ban_service::{BanRequest, BanSource, BanTarget, UnbanRequest};

use crate::dto::ban::{BanCheckResponse, BanResponse};
use crate::dto::requests::{BanTargetRequest, CreateBanRequest};
use crate::error::ApiError;
use crate::response::{
    ApiResponse, MutationResult, PaginatedResponse, PaginationParams, created, mutation_ok, ok,
};
use crate::state::{ApiEvent, ApiState};
use crate::util::{BanTargetKind, BanTargetParts, now_iso8601, parse_ban_target};

#[derive(Debug, Deserialize)]
pub struct BanListQuery {
    pub target_type: Option<String>,
    pub source: Option<String>,
}

pub async fn list(
    State(state): State<Arc<ApiState>>,
    Query(mut pagination): Query<PaginationParams>,
    Query(query): Query<BanListQuery>,
) -> Result<Json<PaginatedResponse<BanResponse>>, ApiError> {
    pagination.normalize();

    let target_kind = query
        .target_type
        .as_deref()
        .map(str::parse::<BanTargetKind>)
        .transpose()?
        .map(|kind| kind.to_string());

    let mut bans = state
        .ban_service
        .list_all()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to fetch bans: {e}")))?;

    if let Some(ref src) = query.source {
        bans.retain(|b| b.source.to_string() == *src);
    }

    bans.sort_by_key(|b| std::cmp::Reverse(b.created_at));

    let mut responses = bans
        .iter()
        .map(BanResponse::from_entry)
        .collect::<Result<Vec<_>, _>>()?;

    if let Some(kind) = target_kind {
        responses.retain(|ban| ban.target_type == kind);
    }

    Ok(Json(pagination.apply(responses)))
}

pub async fn check(
    State(state): State<Arc<ApiState>>,
    Path((target_type, value)): Path<(String, String)>,
) -> Result<Json<ApiResponse<BanCheckResponse>>, ApiError> {
    let target = parse_ban_target(&target_type, &value)?;

    let ban_entry = state
        .ban_service
        .get(&target)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to check ban: {e}")))?;

    let response = match ban_entry {
        Some(entry) if !entry.is_expired() => BanCheckResponse {
            banned: true,
            ban: Some(BanResponse::from_entry(&entry)?),
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
            .or_else(|_| ip.parse().map(BanTarget::IpRange))
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

    let parts = BanTargetParts::try_from(&target)?;

    let mut request = BanRequest::new(target).source(web_api());
    request.reason = body.reason.clone();
    request.duration = duration;
    state
        .ban_service
        .ban(request)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create ban: {e}")))?;

    let _ = state.event_tx.send(ApiEvent::BanCreated {
        target_type: parts.kind.to_string(),
        target_value: parts.value,
        reason: body.reason,
        source: "admin_api".to_string(),
        timestamp: now_iso8601(),
    });

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
        .unban(UnbanRequest::new(target.clone()).source(web_api()))
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to remove ban: {e}")))?;

    if removed.is_some() {
        let parts = BanTargetParts::try_from(&target)?;
        let _ = state.event_tx.send(ApiEvent::BanRemoved {
            target_type: parts.kind.to_string(),
            target_value: parts.value,
            timestamp: now_iso8601(),
        });
        Ok(mutation_ok("Ban removed"))
    } else {
        Err(ApiError::NotFound(format!(
            "No active ban found for {target_type}/{value}"
        )))
    }
}

const fn web_api() -> BanSource {
    BanSource::WebApi { actor: None }
}

use std::sync::Arc;

use axum::Json;
use axum::extract::State;

use crate::error::ApiError;
use crate::response::{ApiResponse, ok};
use crate::state::{ApiState, RecentEvent};

pub async fn recent(
    State(state): State<Arc<ApiState>>,
) -> Result<Json<ApiResponse<Vec<RecentEvent>>>, ApiError> {
    let events = state
        .recent_events
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .cloned()
        .collect::<Vec<_>>();

    Ok(ok(events))
}

use crate::error::ApiError;
use crate::state::ApiState;

/// Verifies SSE authentication via query param token.
///
/// `EventSource` cannot send custom headers, so the API key is passed
/// as `?token=<key>` in the URL.
pub fn verify_sse_auth(state: &ApiState, query_token: &Option<String>) -> Result<(), ApiError> {
    match query_token {
        Some(token) => {
            if state.config.verify_api_key(token) {
                Ok(())
            } else {
                Err(ApiError::Unauthorized("Invalid token".into()))
            }
        }
        None => Err(ApiError::Unauthorized(
            "Missing authentication. Use ?token=<api_key> for SSE.".into(),
        )),
    }
}

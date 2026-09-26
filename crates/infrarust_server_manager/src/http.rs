use std::time::Duration;

use crate::error::ServerManagerError;

pub(crate) const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) async fn check_response(
    resp: reqwest::Response,
    context: &str,
) -> Result<reqwest::Response, ServerManagerError> {
    if resp.status().is_success() {
        return Ok(resp);
    }
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    Err(ServerManagerError::ApiResponse(format!(
        "{context} returned {status}: {body}"
    )))
}

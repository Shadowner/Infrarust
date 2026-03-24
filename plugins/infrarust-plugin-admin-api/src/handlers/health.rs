use axum::Json;

pub async fn check() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

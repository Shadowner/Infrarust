use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct KickRequest {
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SendRequest {
    pub server: String,
}

#[derive(Debug, Deserialize)]
pub struct MessageRequest {
    pub text: String,
}

#[derive(Debug, Deserialize)]
pub struct BroadcastRequest {
    pub text: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateBanRequest {
    pub target: BanTargetRequest,
    pub reason: Option<String>,
    pub duration_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum BanTargetRequest {
    Ip(String),
    Username(String),
    Uuid(String),
}

use std::net::IpAddr;
use std::time::Duration;

use infrarust_api::player::Player;
use infrarust_protocol::packets::cookie::{
    CConfigStoreCookie, CLoginCookieRequest, CStoreCookie, SLoginCookieResponse,
};
use infrarust_protocol::packets::play::transfer::{CConfigTransfer, CTransfer};
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};

use crate::error::CoreError;
use crate::hot_restart::{
    RESUME_COOKIE, Redeemed, ResumeHolder, ResumePresenter, ResumeRecord, ResumeToken,
};
use crate::pipeline::types::{ConnectionIntent, HandshakeData, LoginData};
use crate::server::ProxyServices;
use crate::session::client_bridge::ClientBridge;
use crate::session::context::SessionContext;

pub(crate) const RESUME_SINCE: ProtocolVersion = ProtocolVersion::V1_20_5;
const MAX_TRANSFER_HOST: usize = 255;
const MAX_RESUME_RESPONSE_FRAME: usize = 128;
const RESUME_ANSWER: Duration = Duration::from_secs(5);
const DEPARTURE: Duration = Duration::from_secs(2);

pub(crate) enum Claim {
    Absent,
    Presented,
    Resumed(Box<ResumeRecord>),
}

pub(crate) async fn send_back(ctx: &SessionContext<'_>, client: &mut ClientBridge) -> bool {
    let resumes = &ctx.services.resumes;
    let host = ctx.handshake.domain.as_str();
    let sendable = resumes.is_armed()
        && ctx.version().no_less_than(RESUME_SINCE)
        && matches!(
            client.state(),
            ConnectionState::Config | ConnectionState::Play
        )
        && !host.is_empty()
        && host.len() <= MAX_TRANSFER_HOST;
    if !sendable {
        return false;
    }
    let session = &ctx.session;
    let holder = ResumeHolder {
        profile: session.game_profile().clone(),
        online_mode: session.kind().online_mode(),
        client_ip: ctx.real_ip(),
        domain: host.to_string(),
        server: session
            .current_server()
            .map(|server| server.as_str().to_string()),
        protocol_version: ctx.version().0,
    };
    let Some(token) = resumes.issue(holder) else {
        return false;
    };
    let port = ctx.handshake.port;
    match transfer(ctx, client, &token, host, port).await {
        Ok(()) => {
            tracing::debug!(
                username = %ctx.username(),
                host,
                port,
                "player sent back to the proxy for the restart"
            );
            tokio::time::timeout(DEPARTURE, client.closed()).await.ok();
            client.shutdown().await;
            true
        }
        Err(error) => {
            tracing::debug!(username = %ctx.username(), %error, "the transfer could not be written");
            false
        }
    }
}

async fn transfer(
    ctx: &SessionContext<'_>,
    client: &mut ClientBridge,
    token: &ResumeToken,
    host: &str,
    port: u16,
) -> Result<(), CoreError> {
    let registry = ctx.registry();
    let key = RESUME_COOKIE.to_string();
    let payload = token.to_vec();
    let host = host.to_string();
    let port = i32::from(port);
    if client.state() == ConnectionState::Config {
        client
            .send_packet(&CConfigStoreCookie { key, payload }, registry)
            .await?;
        client
            .send_packet(&CConfigTransfer { host, port }, registry)
            .await
    } else {
        client
            .send_packet(&CStoreCookie { key, payload }, registry)
            .await?;
        client
            .send_packet(&CTransfer { host, port }, registry)
            .await
    }
}

pub(crate) async fn claim(
    services: &ProxyServices,
    client: &mut ClientBridge,
    handshake: &HandshakeData,
    login: Option<&LoginData>,
    client_ip: IpAddr,
) -> Result<Claim, CoreError> {
    let Some(login) = login else {
        return Ok(Claim::Absent);
    };
    let version = handshake.protocol_version;
    if handshake.intent != ConnectionIntent::Transfer
        || !version.no_less_than(RESUME_SINCE)
        || !services.resumes.expects_any()
    {
        return Ok(Claim::Absent);
    }
    let registry = &services.packet_registry;
    let request = CLoginCookieRequest {
        key: RESUME_COOKIE.to_string(),
    };
    client.send_packet(&request, registry).await?;
    let frame = tokio::time::timeout(
        RESUME_ANSWER,
        client.read_preauth_frame(MAX_RESUME_RESPONSE_FRAME),
    )
    .await
    .map_err(|_| CoreError::Timeout("the client did not answer the resume request".to_string()))??
    .ok_or(CoreError::ConnectionClosed)?;
    let answer = registry
        .decode_as::<SLoginCookieResponse>(&frame, version)?
        .ok_or_else(|| {
            CoreError::Auth(format!(
                "expected a cookie response, got packet 0x{:02x}",
                frame.id
            ))
        })?;
    if answer.key != RESUME_COOKIE {
        return Ok(Claim::Absent);
    }
    let Some(token) = answer.payload else {
        return Ok(Claim::Absent);
    };
    let presenter = ResumePresenter {
        client_ip,
        domain: &handshake.domain,
        username: &login.username,
        uuid: login.player_uuid,
        protocol_version: version.0,
    };
    match services.resumes.redeem(&token, &presenter) {
        Redeemed::Resumed(record) => Ok(Claim::Resumed(record)),
        Redeemed::Refused(reason) => {
            tracing::info!(
                username = %login.username,
                %reason,
                "a resume ticket was refused, the player logs in normally"
            );
            Ok(Claim::Presented)
        }
        Redeemed::Unknown => {
            tracing::debug!(username = %login.username, "unknown resume ticket");
            Ok(Claim::Presented)
        }
    }
}

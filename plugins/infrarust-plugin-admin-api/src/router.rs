use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::http::{Method, Request, header};
use axum::middleware;
use axum::routing::{delete, get, post};
use tower::ServiceBuilder;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

use crate::auth;
use crate::config::ApiConfig;
use crate::handlers;
use crate::rate_limit;
use crate::sse;
use crate::state::ApiState;

pub fn build_router(state: Arc<ApiState>) -> Router {
    let timeout_layer = TimeoutLayer::with_status_code(
        axum::http::StatusCode::REQUEST_TIMEOUT,
        Duration::from_secs(30),
    );

    let public_routes = Router::new()
        .route("/api/v1/health", get(handlers::health::check))
        .route_layer(timeout_layer);

    let protected_routes = Router::new()
        // ── Read endpoints (GET) ──
        .route("/api/v1/proxy", get(handlers::proxy::status))
        .route("/api/v1/players", get(handlers::players::list))
        .route("/api/v1/players/count", get(handlers::players::count))
        .route(
            "/api/v1/players/{id_or_username}",
            get(handlers::players::get),
        )
        .route(
            "/api/v1/bans",
            get(handlers::bans::list).post(handlers::bans::create),
        )
        .route(
            "/api/v1/bans/check/{target_type}/{value}",
            get(handlers::bans::check),
        )
        .route("/api/v1/servers", get(handlers::servers::list).post(handlers::servers::create))
        .route("/api/v1/servers/{id}", get(handlers::servers::get).put(handlers::servers::update).delete(handlers::servers::delete))
        .route("/api/v1/plugins", get(handlers::plugins::list))
        .route("/api/v1/plugins/{id}", get(handlers::plugins::get))
        .route("/api/v1/stats", get(handlers::stats::overview))
        .route("/api/v1/events/recent", get(handlers::events::recent))
        .route(
            "/api/v1/config/providers",
            get(handlers::config::list_providers),
        )
        // ── Log history (REST, not SSE) ──
        .route(
            "/api/v1/logs/history",
            get(sse::handlers::log_history),
        )
        // ── Mutation endpoints (POST / DELETE) ──
        .route(
            "/api/v1/players/broadcast",
            post(handlers::players::broadcast),
        )
        .route(
            "/api/v1/players/{username}/kick",
            post(handlers::players::kick),
        )
        .route(
            "/api/v1/players/{username}/send",
            post(handlers::players::send),
        )
        .route(
            "/api/v1/players/{username}/message",
            post(handlers::players::message),
        )
        .route(
            "/api/v1/bans/{target_type}/{value}",
            delete(handlers::bans::delete),
        )
        .route(
            "/api/v1/servers/{id}/start",
            post(handlers::servers::start),
        )
        .route(
            "/api/v1/servers/{id}/stop",
            post(handlers::servers::stop),
        )
        .route(
            "/api/v1/servers/{id}/health",
            get(handlers::servers::health_check),
        )
        .route(
            "/api/v1/servers/{id}/health/cached",
            get(handlers::servers::health_cached),
        )
        .route("/api/v1/config/reload", post(handlers::config::reload))
        .route(
            "/api/v1/plugins/{id}/disable",
            post(handlers::plugins::disable),
        )
        .route(
            "/api/v1/plugins/{id}/enable",
            post(handlers::plugins::enable),
        )
        .route(
            "/api/v1/proxy/shutdown",
            post(handlers::proxy::shutdown),
        )
        .route("/api/v1/proxy/gc", post(handlers::proxy::gc))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit::rate_limit_middleware,
        ))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::auth_middleware,
        ))
        .route_layer(timeout_layer);

    // SSE routes: no timeout (streams are infinite), no auth middleware
    // (auth verified via ?token= query param inside each handler)
    let sse_routes = Router::new()
        .route("/api/v1/events", get(sse::handlers::event_stream))
        .route("/api/v1/logs", get(sse::handlers::log_stream));

    Router::new()
        .merge(public_routes)
        .merge(protected_routes)
        .merge(sse_routes)
        .with_state(state.clone())
        .layer(
            ServiceBuilder::new()
                .layer(
                    TraceLayer::new_for_http().make_span_with(|request: &Request<_>| {
                        tracing::info_span!(
                            "http_request",
                            method = %request.method(),
                            uri = %request.uri(),
                        )
                    }),
                )
                .layer(CompressionLayer::new())
                .layer(build_cors_layer(&state.config)),
        )
}

fn build_cors_layer(config: &ApiConfig) -> CorsLayer {
    let cors = CorsLayer::new()
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE, header::ACCEPT]);

    if config.cors_origins.is_empty() {
        cors
    } else {
        let origins: Vec<_> = config
            .cors_origins
            .iter()
            .filter_map(|o| o.parse().ok())
            .collect();
        cors.allow_origin(origins)
    }
}

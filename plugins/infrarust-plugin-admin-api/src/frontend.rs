use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "frontend/.output/public/"]
struct FrontendAssets;

pub async fn spa_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

    if path.starts_with("api/") {
        return (StatusCode::NOT_FOUND, "Not Found").into_response();
    }

    if !path.is_empty()
        && let Some(file) = FrontendAssets::get(path)
    {
        return serve_file(path, &file.data);
    }

    serve_index()
}

fn serve_file(path: &str, data: &[u8]) -> Response {
    let mime = mime_guess::from_path(path)
        .first_or_octet_stream()
        .to_string();

    let cache_control = if path.starts_with("_nuxt/") {
        "public, max-age=31536000, immutable"
    } else if path == "index.html" || path == "200.html" {
        "no-cache"
    } else {
        "public, max-age=3600"
    };

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, cache_control.to_string()),
        ],
        data.to_vec(),
    )
        .into_response()
}

fn serve_index() -> Response {
    match FrontendAssets::get("index.html") {
        Some(file) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8".to_string()),
                (header::CACHE_CONTROL, "no-cache".to_string()),
            ],
            file.data.into_owned(),
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "Frontend not available").into_response(),
    }
}

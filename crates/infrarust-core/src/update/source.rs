use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::header::{ACCEPT, ETAG, HeaderMap, IF_NONE_MATCH};
use reqwest::{Response, StatusCode, Url};
use serde::Deserialize;

use super::error::UpdateError;

pub const RELEASE_LIST_LIMIT: u64 = 2 * 1024 * 1024;
pub const MANIFEST_LIMIT: u64 = 64 * 1024;
pub const SIGNATURE_LIMIT: u64 = 4 * 1024;
pub const MANIFEST_ASSET: &str = "manifest.json";
pub const SIGNATURE_ASSET: &str = "manifest.json.minisig";

const PER_PAGE: &str = "20";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(30);
const API_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const RATE_LIMIT_RESET: &str = "x-ratelimit-reset";
const RATE_LIMIT_REMAINING: &str = "x-ratelimit-remaining";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
}

impl Release {
    pub fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|asset| asset.name == name)
    }
}

#[derive(Debug)]
struct CachedList {
    etag: String,
    releases: Arc<[Release]>,
}

#[derive(Debug)]
pub struct ReleaseSource {
    client: reqwest::Client,
    url: String,
    cache: Mutex<Option<CachedList>>,
}

impl ReleaseSource {
    pub fn new(url: &str, version: &str) -> Result<Self, UpdateError> {
        let invalid = |reason: String| UpdateError::InvalidSource {
            url: url.to_string(),
            reason,
        };
        let parsed = Url::parse(url).map_err(|e| invalid(e.to_string()))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(invalid("only http and https are supported".to_string()));
        }
        let client = reqwest::Client::builder()
            .user_agent(format!("infrarust/{version}"))
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
            .build()
            .map_err(|e| invalid(e.to_string()))?;
        Ok(Self {
            client,
            url: url.to_string(),
            cache: Mutex::new(None),
        })
    }

    pub async fn releases(&self) -> Result<Arc<[Release]>, UpdateError> {
        let etag = self.cached().map(|cached| cached.0);
        let mut request = self
            .client
            .get(&self.url)
            .query(&[("per_page", PER_PAGE)])
            .header(ACCEPT, "application/vnd.github+json")
            .timeout(API_TIMEOUT);
        if let Some(etag) = &etag {
            request = request.header(IF_NONE_MATCH, etag);
        }
        let response = request.send().await.map_err(|e| network(&self.url, e))?;
        if response.status() == StatusCode::NOT_MODIFIED {
            return match self.cached() {
                Some((cached_etag, releases)) if Some(&cached_etag) == etag.as_ref() => {
                    Ok(releases)
                }
                _ => Err(UpdateError::Status {
                    url: self.url.clone(),
                    status: StatusCode::NOT_MODIFIED.as_u16(),
                }),
            };
        }
        let response = checked(&self.url, response)?;
        let fresh_etag = response
            .headers()
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = read_capped(&self.url, response, "the release list", RELEASE_LIST_LIMIT).await?;
        let releases: Vec<Release> =
            serde_json::from_slice(&body).map_err(|e| UpdateError::Malformed {
                what: "the release list".to_string(),
                reason: e.to_string(),
            })?;
        let releases: Arc<[Release]> = releases.into();
        *self.cache.lock().unwrap_or_else(PoisonError::into_inner) =
            fresh_etag.map(|etag| CachedList {
                etag,
                releases: Arc::clone(&releases),
            });
        Ok(releases)
    }

    pub async fn fetch(&self, url: &str, what: &str, limit: u64) -> Result<Vec<u8>, UpdateError> {
        let response = self
            .client
            .get(url)
            .timeout(API_TIMEOUT)
            .send()
            .await
            .map_err(|e| network(url, e))?;
        let response = checked(url, response)?;
        read_capped(url, response, what, limit).await
    }

    pub async fn download(&self, url: &str) -> Result<Response, UpdateError> {
        let response = self
            .client
            .get(url)
            .timeout(DOWNLOAD_TIMEOUT)
            .send()
            .await
            .map_err(|e| network(url, e))?;
        checked(url, response)
    }

    fn cached(&self) -> Option<(String, Arc<[Release]>)> {
        self.cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|cached| (cached.etag.clone(), Arc::clone(&cached.releases)))
    }
}

pub(super) fn network(url: &str, source: reqwest::Error) -> UpdateError {
    UpdateError::Network {
        url: url.to_string(),
        source,
    }
}

fn checked(url: &str, response: Response) -> Result<Response, UpdateError> {
    let status = response.status();
    if status == StatusCode::TOO_MANY_REQUESTS
        || (status == StatusCode::FORBIDDEN && rate_limit_exhausted(response.headers()))
    {
        return Err(UpdateError::RateLimited {
            reset: rate_limit_reset(response.headers()),
        });
    }
    if !status.is_success() {
        return Err(UpdateError::Status {
            url: url.to_string(),
            status: status.as_u16(),
        });
    }
    Ok(response)
}

fn rate_limit_exhausted(headers: &HeaderMap) -> bool {
    headers.contains_key(reqwest::header::RETRY_AFTER)
        || headers
            .get(RATE_LIMIT_REMAINING)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.trim() == "0")
}

fn rate_limit_reset(headers: &HeaderMap) -> Option<SystemTime> {
    let seconds: u64 = headers
        .get(RATE_LIMIT_RESET)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    UNIX_EPOCH.checked_add(Duration::from_secs(seconds))
}

async fn read_capped(
    url: &str,
    mut response: Response,
    what: &str,
    limit: u64,
) -> Result<Vec<u8>, UpdateError> {
    let too_large = || UpdateError::TooLarge {
        what: what.to_string(),
        limit,
    };
    if response.content_length().is_some_and(|len| len > limit) {
        return Err(too_large());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| network(url, e))? {
        if body.len() as u64 + chunk.len() as u64 > limit {
            return Err(too_large());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::super::testing::{HttpServer, Reply};
    use super::*;

    fn list(tags: &[&str]) -> Vec<u8> {
        let releases: Vec<serde_json::Value> = tags
            .iter()
            .map(|tag| serde_json::json!({"tag_name": tag, "assets": []}))
            .collect();
        serde_json::to_vec(&releases).unwrap()
    }

    #[tokio::test]
    async fn the_list_is_requested_with_the_version_user_agent_and_20_per_page() {
        let server = HttpServer::start(|_| Reply::ok(list(&["v2.0.0"]))).await;
        let source = ReleaseSource::new(&server.url("/releases"), "2.0.0-beta.5").unwrap();
        let releases = source.releases().await.unwrap();
        assert_eq!(releases[0].tag_name, "v2.0.0");

        let requests = server.requests();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.path, "/releases");
        assert_eq!(request.query.as_deref(), Some("per_page=20"));
        assert_eq!(request.header("user-agent"), Some("infrarust/2.0.0-beta.5"));
        assert!(request.header("cookie").is_none());
        assert!(request.header("authorization").is_none());
    }

    #[tokio::test]
    async fn a_304_reuses_the_cached_list() {
        let server = HttpServer::start(|request| {
            if request.header("if-none-match") == Some("\"abc\"") {
                Reply::status(304)
            } else {
                Reply::ok(list(&["v2.0.0", "v1.0.0"])).with_header("ETag", "\"abc\"")
            }
        })
        .await;
        let source = ReleaseSource::new(&server.url("/releases"), "2.0.0").unwrap();
        let first = source.releases().await.unwrap();
        let second = source.releases().await.unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(second.len(), 2);

        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].header("if-none-match").is_none());
        assert_eq!(requests[1].header("if-none-match"), Some("\"abc\""));
    }

    #[tokio::test]
    async fn a_dropped_request_leaves_the_etag_cache_intact() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let server =
            HttpServer::start(
                move |request| match counter.fetch_add(1, Ordering::SeqCst) {
                    0 => Reply::ok(list(&["v2.0.0"])).with_header("ETag", "\"one\""),
                    1 => Reply::stall(),
                    _ if request.header("if-none-match") == Some("\"one\"") => Reply::status(304),
                    _ => Reply::ok(list(&["v9.0.0"])),
                },
            )
            .await;
        let source = ReleaseSource::new(&server.url("/releases"), "2.0.0").unwrap();
        let first = source.releases().await.unwrap();
        let dropped = tokio::time::timeout(Duration::from_millis(200), source.releases()).await;
        assert!(dropped.is_err());
        let third = source.releases().await.unwrap();
        assert!(Arc::ptr_eq(&first, &third));
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn a_list_without_an_etag_is_not_cached() {
        let server = HttpServer::start(|_| Reply::ok(list(&["v2.0.0"]))).await;
        let source = ReleaseSource::new(&server.url("/releases"), "2.0.0").unwrap();
        source.releases().await.unwrap();
        source.releases().await.unwrap();
        assert!(
            server
                .requests()
                .iter()
                .all(|r| r.header("if-none-match").is_none())
        );
    }

    #[tokio::test]
    async fn a_403_reports_the_rate_limit_reset() {
        let server = HttpServer::start(|_| {
            Reply::status(403)
                .with_header("x-ratelimit-remaining", "0")
                .with_header("x-ratelimit-reset", "1700000000")
        })
        .await;
        let source = ReleaseSource::new(&server.url("/releases"), "2.0.0").unwrap();
        match source.releases().await {
            Err(UpdateError::RateLimited { reset }) => {
                assert_eq!(reset, Some(UNIX_EPOCH + Duration::from_secs(1_700_000_000)));
            }
            other => panic!("expected a rate limit, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_403_with_quota_left_is_a_plain_status() {
        let server = HttpServer::start(|_| Reply::status(403)).await;
        let source = ReleaseSource::new(&server.url("/releases"), "2.0.0").unwrap();
        assert!(matches!(
            source.releases().await,
            Err(UpdateError::Status { status: 403, .. })
        ));
    }

    #[tokio::test]
    async fn a_429_without_reset_is_still_a_rate_limit() {
        let server = HttpServer::start(|_| Reply::status(429)).await;
        let source = ReleaseSource::new(&server.url("/releases"), "2.0.0").unwrap();
        assert!(matches!(
            source.releases().await,
            Err(UpdateError::RateLimited { reset: None })
        ));
    }

    #[tokio::test]
    async fn other_statuses_are_reported() {
        let server = HttpServer::start(|_| Reply::status(500)).await;
        let source = ReleaseSource::new(&server.url("/releases"), "2.0.0").unwrap();
        assert!(matches!(
            source.releases().await,
            Err(UpdateError::Status { status: 500, .. })
        ));
    }

    #[tokio::test]
    async fn oversized_bodies_are_cut() {
        let server = HttpServer::start(|request| match request.path.as_str() {
            "/releases" => Reply::ok(vec![b' '; RELEASE_LIST_LIMIT as usize + 1]),
            _ => Reply::ok(vec![b'x'; SIGNATURE_LIMIT as usize + 1]),
        })
        .await;
        let source = ReleaseSource::new(&server.url("/releases"), "2.0.0").unwrap();
        assert!(matches!(
            source.releases().await,
            Err(UpdateError::TooLarge { .. })
        ));
        assert!(matches!(
            source
                .fetch(&server.url("/sig"), "the signature", SIGNATURE_LIMIT)
                .await,
            Err(UpdateError::TooLarge { .. })
        ));
        let fits = source
            .fetch(&server.url("/sig"), "the signature", SIGNATURE_LIMIT + 1)
            .await
            .unwrap();
        assert_eq!(fits.len() as u64, SIGNATURE_LIMIT + 1);
    }

    #[tokio::test]
    async fn malformed_lists_are_reported() {
        let server = HttpServer::start(|_| Reply::ok(b"{\"message\":\"nope\"}".to_vec())).await;
        let source = ReleaseSource::new(&server.url("/releases"), "2.0.0").unwrap();
        assert!(matches!(
            source.releases().await,
            Err(UpdateError::Malformed { .. })
        ));
    }

    #[test]
    fn only_http_urls_are_accepted() {
        assert!(ReleaseSource::new("ftp://example.invalid/releases", "2.0.0").is_err());
        assert!(ReleaseSource::new("not a url", "2.0.0").is_err());
        assert!(ReleaseSource::new("https://example.invalid/releases", "2.0.0").is_ok());
    }
}

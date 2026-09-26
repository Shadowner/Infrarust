use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use http_body_util::BodyExt;
use hyper::client::conn::http1::SendRequest;
use rustls::ClientConfig;
use rustls::pki_types::ServerName;
use rustls_platform_verifier::BuilderVerifierExt;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tracing::instrument::WithSubscriber;
use wasmtime_wasi::runtime::AbortOnDropJoinHandle;
use wasmtime_wasi_http::io::TokioIo;
use wasmtime_wasi_http::p2::bindings::http::types::{DnsErrorPayload, ErrorCode};
use wasmtime_wasi_http::p2::body::HyperOutgoingBody;
use wasmtime_wasi_http::p2::types::{
    HostFutureIncomingResponse, IncomingResponse, OutgoingRequestConfig,
};
use wasmtime_wasi_http::p2::{HttpResult, WasiHttpHooks, hyper_request_error};

use super::policy::{HttpResolveError, HttpRoute, NetworkPolicy, Refusal};

pub(crate) struct HttpHooks {
    policy: Arc<NetworkPolicy>,
    limit: Duration,
}

impl HttpHooks {
    pub(crate) fn new(policy: Arc<NetworkPolicy>, limit: Duration) -> Self {
        Self { policy, limit }
    }
}

impl WasiHttpHooks for HttpHooks {
    fn send_request(
        &mut self,
        request: hyper::Request<HyperOutgoingBody>,
        config: OutgoingRequestConfig,
    ) -> HttpResult<HostFutureIncomingResponse> {
        let Some(authority) = request.uri().authority().cloned() else {
            return Err(ErrorCode::HttpRequestUriInvalid.into());
        };
        let port = authority
            .port_u16()
            .unwrap_or(if config.use_tls { 443 } else { 80 });
        let host = authority.host().to_owned();
        let destination = format!("{host}:{port}");
        let route = match self.policy.route_http(&host, port) {
            Ok(route) => route,
            Err(refusal) => {
                self.policy.report_denied("http", &destination, refusal);
                return Err(ErrorCode::HttpRequestDenied.into());
            }
        };
        let config = OutgoingRequestConfig {
            use_tls: config.use_tls,
            connect_timeout: config.connect_timeout.min(self.limit),
            first_byte_timeout: config.first_byte_timeout.min(self.limit),
            between_bytes_timeout: config.between_bytes_timeout.min(self.limit),
        };
        let exchange = Exchange {
            policy: Arc::clone(&self.policy),
            host,
            destination,
            route,
            config,
        };
        Ok(HostFutureIncomingResponse::pending(
            wasmtime_wasi::runtime::spawn(
                async move { Ok(exchange.run(request).await) }.with_current_subscriber(),
            ),
        ))
    }
}

struct Exchange {
    policy: Arc<NetworkPolicy>,
    host: String,
    destination: String,
    route: HttpRoute,
    config: OutgoingRequestConfig,
}

impl Exchange {
    async fn run(
        self,
        mut request: hyper::Request<HyperOutgoingBody>,
    ) -> Result<IncomingResponse, ErrorCode> {
        let targets = match self.policy.resolve_http(self.route.clone()).await {
            Ok(targets) => targets,
            Err(HttpResolveError::Denied) => {
                self.policy
                    .report_denied("http", &self.destination, Refusal::AllowList);
                return Err(ErrorCode::HttpRequestDenied);
            }
            Err(HttpResolveError::Lookup) => {
                return Err(ErrorCode::DnsError(DnsErrorPayload {
                    rcode: Some("address not available".to_owned()),
                    info_code: Some(0),
                }));
            }
        };
        let stream = connect(&targets, self.config.connect_timeout).await?;

        if !request.headers().contains_key(hyper::header::HOST)
            && let Some(authority) = request.uri().authority()
            && let Ok(value) = hyper::header::HeaderValue::from_str(authority.as_str())
        {
            request.headers_mut().insert(hyper::header::HOST, value);
        }
        let path = request
            .uri()
            .path_and_query()
            .map_or("/", |path| path.as_str())
            .to_owned();
        *request.uri_mut() = http::Uri::builder()
            .path_and_query(path)
            .build()
            .map_err(|_| ErrorCode::HttpRequestUriInvalid)?;

        let (mut sender, worker) = if self.config.use_tls {
            let tls = self.tls_config()?;
            let name = self
                .host
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
                .unwrap_or(&self.host);
            let server_name =
                ServerName::try_from(name.to_owned()).map_err(|_| ErrorCode::TlsProtocolError)?;
            let stream = timeout(
                self.config.connect_timeout,
                tokio_rustls::TlsConnector::from(tls).connect(server_name, stream),
            )
            .await
            .map_err(|_| ErrorCode::ConnectionTimeout)?
            .map_err(tls_error)?;
            handshake(stream, self.config.connect_timeout).await?
        } else {
            handshake(stream, self.config.connect_timeout).await?
        };

        let resp = timeout(self.config.first_byte_timeout, sender.send_request(request))
            .await
            .map_err(|_| ErrorCode::ConnectionReadTimeout)?
            .map_err(hyper_request_error)?
            .map(|body| body.map_err(hyper_request_error).boxed_unsync());
        Ok(IncomingResponse {
            resp,
            worker: Some(worker),
            between_bytes_timeout: self.config.between_bytes_timeout,
        })
    }

    fn tls_config(&self) -> Result<Arc<ClientConfig>, ErrorCode> {
        let built = self.policy.tls.get_or_init(|| {
            let built = build_tls_config();
            if let Err(error) = &built {
                tracing::warn!(
                    plugin = %self.policy.plugin_id(),
                    %error,
                    "wasm plugin HTTPS is unavailable: the TLS client could not be built"
                );
            }
            built
        });
        built.clone().map_err(|_| ErrorCode::TlsProtocolError)
    }
}

fn build_tls_config() -> Result<Arc<ClientConfig>, String> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_platform_verifier()
        .map_err(|e| e.to_string())?
        .with_no_client_auth();
    Ok(Arc::new(config))
}

async fn connect(targets: &[SocketAddr], limit: Duration) -> Result<TcpStream, ErrorCode> {
    let attempt = async {
        let mut failure = ErrorCode::ConnectionRefused;
        for target in targets {
            match TcpStream::connect(target).await {
                Ok(stream) => return Ok(stream),
                Err(error) => failure = connect_error(&error),
            }
        }
        Err(failure)
    };
    timeout(limit, attempt)
        .await
        .map_err(|_| ErrorCode::ConnectionTimeout)?
}

fn connect_error(error: &std::io::Error) -> ErrorCode {
    match error.kind() {
        std::io::ErrorKind::TimedOut => ErrorCode::ConnectionTimeout,
        std::io::ErrorKind::HostUnreachable | std::io::ErrorKind::NetworkUnreachable => {
            ErrorCode::DestinationUnavailable
        }
        _ => ErrorCode::ConnectionRefused,
    }
}

fn tls_error(error: std::io::Error) -> ErrorCode {
    match error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
    {
        Some(rustls::Error::InvalidCertificate(_)) => ErrorCode::TlsCertificateError,
        _ => ErrorCode::TlsProtocolError,
    }
}

async fn handshake<S>(
    stream: S,
    limit: Duration,
) -> Result<(SendRequest<HyperOutgoingBody>, AbortOnDropJoinHandle<()>), ErrorCode>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (sender, connection) = timeout(
        limit,
        hyper::client::conn::http1::handshake(TokioIo::new(stream)),
    )
    .await
    .map_err(|_| ErrorCode::ConnectionTimeout)?
    .map_err(hyper_request_error)?;
    let worker = wasmtime_wasi::runtime::spawn(
        async move {
            if let Err(error) = connection.await {
                tracing::debug!(%error, "wasm plugin HTTP connection ended with an error");
            }
        }
        .with_current_subscriber(),
    );
    Ok((sender, worker))
}

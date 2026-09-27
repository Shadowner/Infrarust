use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use http::uri::Scheme;
use http_body_util::BodyExt;
use hyper::body::{Body, Bytes, Frame, Incoming, SizeHint};
use hyper::client::conn::http1::SendRequest;
use rustls::ClientConfig;
use rustls::pki_types::ServerName;
use rustls_platform_verifier::BuilderVerifierExt;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::time::{Instant, Sleep, sleep, timeout};
use tracing::instrument::WithSubscriber;
use wasmtime_wasi::runtime::AbortOnDropJoinHandle;
use wasmtime_wasi_http::io::TokioIo;
use wasmtime_wasi_http::{Error, RequestOptions, WasiBody, WasiHttpHooks};

use super::policy::{HttpResolveError, HttpRoute, NetworkPolicy, Refusal};

type IoFuture = Box<dyn Future<Output = Result<(), Error>> + Send>;
type SendFuture =
    Box<dyn Future<Output = Result<(http::Response<WasiBody>, IoFuture), Error>> + Send>;

pub(crate) struct HttpHooks {
    policy: Arc<NetworkPolicy>,
    limit: Duration,
}

impl HttpHooks {
    pub(crate) fn new(policy: Arc<NetworkPolicy>, limit: Duration) -> Self {
        Self { policy, limit }
    }

    fn prepare(
        &self,
        request: &http::Request<WasiBody>,
        options: Option<RequestOptions>,
    ) -> Result<Exchange, Error> {
        let Some(authority) = request.uri().authority().cloned() else {
            return Err(Error::HttpRequestUriInvalid);
        };
        let use_tls = request.uri().scheme() == Some(&Scheme::HTTPS);
        let port = authority
            .port_u16()
            .unwrap_or(if use_tls { 443 } else { 80 });
        let host = authority.host().to_owned();
        let destination = format!("{host}:{port}");
        let route = match self.policy.route_http(&host, port) {
            Ok(route) => route,
            Err(refusal) => {
                self.policy.report_denied("http", &destination, refusal);
                return Err(Error::HttpRequestDenied);
            }
        };
        let options = options.unwrap_or_default();
        let bound =
            |value: Option<Duration>| value.map_or(self.limit, |value| value.min(self.limit));
        Ok(Exchange {
            policy: Arc::clone(&self.policy),
            host,
            destination,
            route,
            use_tls,
            connect_timeout: bound(options.connect_timeout),
            first_byte_timeout: bound(options.first_byte_timeout),
            between_bytes_timeout: bound(options.between_bytes_timeout),
        })
    }
}

impl WasiHttpHooks for HttpHooks {
    fn send_request(
        &mut self,
        request: http::Request<WasiBody>,
        options: Option<RequestOptions>,
        _fut: IoFuture,
    ) -> SendFuture {
        let exchange = self.prepare(&request, options);
        Box::new(async move { exchange?.run(request).await }.with_current_subscriber())
    }
}

struct Exchange {
    policy: Arc<NetworkPolicy>,
    host: String,
    destination: String,
    route: HttpRoute,
    use_tls: bool,
    connect_timeout: Duration,
    first_byte_timeout: Duration,
    between_bytes_timeout: Duration,
}

impl Exchange {
    async fn run(
        self,
        mut request: http::Request<WasiBody>,
    ) -> Result<(http::Response<WasiBody>, IoFuture), Error> {
        let targets = match self.policy.resolve_http(self.route.clone()).await {
            Ok(targets) => targets,
            Err(HttpResolveError::Denied) => {
                self.policy
                    .report_denied("http", &self.destination, Refusal::AllowList);
                return Err(Error::HttpRequestDenied);
            }
            Err(HttpResolveError::Lookup) => {
                return Err(Error::DnsError {
                    rcode: Some("address not available".to_owned()),
                    info_code: Some(0),
                });
            }
        };
        let stream = connect(&targets, self.connect_timeout).await?;

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
            .map_err(|_| Error::HttpRequestUriInvalid)?;

        let (mut sender, worker) = if self.use_tls {
            let tls = self.tls_config()?;
            let name = self
                .host
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
                .unwrap_or(&self.host);
            let server_name =
                ServerName::try_from(name.to_owned()).map_err(|_| Error::TlsProtocolError)?;
            let stream = timeout(
                self.connect_timeout,
                tokio_rustls::TlsConnector::from(tls).connect(server_name, stream),
            )
            .await
            .map_err(|_| Error::ConnectionTimeout)?
            .map_err(tls_error)?;
            handshake(stream, self.connect_timeout).await?
        } else {
            handshake(stream, self.connect_timeout).await?
        };

        let between_bytes = self.between_bytes_timeout;
        let resp = timeout(self.first_byte_timeout, sender.send_request(request))
            .await
            .map_err(|_| Error::ConnectionReadTimeout)?
            .map_err(Error::from)?
            .map(|incoming| TimedBody::new(incoming, between_bytes).boxed_unsync());
        let io: IoFuture = Box::new(async move {
            worker.await;
            Ok(())
        });
        Ok((resp, io))
    }

    fn tls_config(&self) -> Result<Arc<ClientConfig>, Error> {
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
        built.clone().map_err(|_| Error::TlsProtocolError)
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

async fn connect(targets: &[SocketAddr], limit: Duration) -> Result<TcpStream, Error> {
    let attempt = async {
        let mut failure = Error::ConnectionRefused;
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
        .map_err(|_| Error::ConnectionTimeout)?
}

fn connect_error(error: &std::io::Error) -> Error {
    match error.kind() {
        std::io::ErrorKind::TimedOut => Error::ConnectionTimeout,
        std::io::ErrorKind::HostUnreachable | std::io::ErrorKind::NetworkUnreachable => {
            Error::DestinationUnavailable
        }
        _ => Error::ConnectionRefused,
    }
}

fn tls_error(error: std::io::Error) -> Error {
    match error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
    {
        Some(rustls::Error::InvalidCertificate(_)) => Error::TlsCertificateError,
        _ => Error::TlsProtocolError,
    }
}

async fn handshake<S>(
    stream: S,
    limit: Duration,
) -> Result<(SendRequest<WasiBody>, AbortOnDropJoinHandle<()>), Error>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (sender, connection) = timeout(
        limit,
        hyper::client::conn::http1::handshake(TokioIo::new(stream)),
    )
    .await
    .map_err(|_| Error::ConnectionTimeout)?
    .map_err(Error::from)?;
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

struct TimedBody {
    incoming: Incoming,
    limit: Duration,
    deadline: Pin<Box<Sleep>>,
}

impl TimedBody {
    fn new(incoming: Incoming, limit: Duration) -> Self {
        Self {
            incoming,
            limit,
            deadline: Box::pin(sleep(limit)),
        }
    }
}

impl Body for TimedBody {
    type Data = Bytes;
    type Error = Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Error>>> {
        let this = &mut *self;
        match Pin::new(&mut this.incoming).poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                let next = Instant::now() + this.limit;
                this.deadline.as_mut().reset(next);
                Poll::Ready(Some(Ok(frame)))
            }
            Poll::Ready(Some(Err(error))) => Poll::Ready(Some(Err(Error::from(error)))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => match this.deadline.as_mut().poll(cx) {
                Poll::Ready(()) => Poll::Ready(Some(Err(Error::ConnectionReadTimeout))),
                Poll::Pending => Poll::Pending,
            },
        }
    }

    fn is_end_stream(&self) -> bool {
        self.incoming.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.incoming.size_hint()
    }
}

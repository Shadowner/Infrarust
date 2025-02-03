use opentelemetry::global;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    trace::{RandomIdGenerator, Sampler, Tracer, TracerProvider},
    Resource,
};
use tracing::Level;
use tracing_subscriber::fmt::format::FmtSpan;

pub struct TracerProviderGuard(pub TracerProvider);

impl Drop for TracerProviderGuard {
    fn drop(&mut self) {
        global::shutdown_tracer_provider();
    }
}

pub fn init_tracer_provider(resource: Resource) -> TracerProviderGuard {
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint("http://localhost:4317") 
        .build()
        .unwrap();

    let provider = TracerProvider::builder()
        .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
        .with_resource(resource)
        .build();

    global::set_tracer_provider(provider.clone());

    TracerProviderGuard(provider)
}

pub fn init_subscriber(provider: &TracerProvider) {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

    let tracer = provider.tracer("proxy");
    let layer = tracing_opentelemetry::layer().with_tracer(tracer);

    tracing_subscriber::registry()
        .with(tracing_subscriber::filter::LevelFilter::from_level(
            Level::DEBUG,
        ))
        .with(layer)
        .with(tracing_subscriber::fmt::Layer::default().with_span_events(FmtSpan::NONE))
        .init();
}

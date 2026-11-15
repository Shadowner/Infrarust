use infrarust_api::permissions::CapabilitySet;

use crate::bindings::infrarust::plugin::log;

use super::state_with;

struct DebugSink;

impl tracing::Subscriber for DebugSink {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        *metadata.level() <= tracing::Level::DEBUG
    }
    fn max_level_hint(&self) -> Option<tracing::level_filters::LevelFilter> {
        Some(tracing::level_filters::LevelFilter::DEBUG)
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, _: &tracing::Event<'_>) {}
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

#[tokio::test]
async fn the_guest_is_told_the_most_verbose_level_the_proxy_logs() {
    let mut state = state_with(CapabilitySet::baseline(), vec![]);
    let sink = tracing::Dispatch::new(DebugSink);
    let told = log::Host::max_level(&mut state).await.unwrap();
    let expected = crate::hosts::enabled_level().map(|level| match level {
        tracing::Level::ERROR => log::Level::Error,
        tracing::Level::WARN => log::Level::Warn,
        tracing::Level::INFO => log::Level::Info,
        tracing::Level::DEBUG => log::Level::Debug,
        _ => log::Level::Trace,
    });
    assert_eq!(told, expected);
    assert!(
        matches!(told, Some(log::Level::Debug | log::Level::Trace)),
        "a live subscriber at debug makes debug lines worth sending, got {told:?}"
    );
    drop(sink);
}

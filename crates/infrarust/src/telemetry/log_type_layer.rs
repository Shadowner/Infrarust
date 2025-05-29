use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use tracing::{
    field::{Field, Visit},
    Event, Id, Metadata, Subscriber,
};
use tracing_subscriber::{
    layer::{Context, Layer},
    registry::LookupSpan,
};

#[derive(Debug, Clone)]
pub struct LogTypeStorage {
    span_log_types: Arc<RwLock<HashMap<Id, String>>>,
    current_event_log_type: Arc<RwLock<Option<String>>>,
}

impl LogTypeStorage {
    pub fn new() -> Self {
        Self {
            span_log_types: Arc::new(RwLock::new(HashMap::new())),
            current_event_log_type: Arc::new(RwLock::new(None)),
        }
    }

    pub fn get_current_log_type(&self) -> Option<String> {
        if let Ok(guard) = self.current_event_log_type.read() {
            if let Some(ref log_type) = *guard {
                return Some(log_type.clone());
            }
        }
        None
    }

    fn set_current_event_log_type(&self, log_type: Option<String>) {
        if let Ok(mut guard) = self.current_event_log_type.write() {
            *guard = log_type;
        }
    }
}

#[derive(Debug)]
pub struct LogTypeLayer {
    storage: LogTypeStorage,
}

impl Clone for LogTypeLayer {
    fn clone(&self) -> Self {
        Self {
            storage: self.storage.clone(),
        }
    }
}

impl LogTypeLayer {
    pub fn new() -> Self {
        Self {
            storage: LogTypeStorage::new(),
        }
    }

    pub fn storage(&self) -> &LogTypeStorage {
        &self.storage
    }
}

impl Default for LogTypeLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> Layer<S> for LogTypeLayer
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_new_span(&self, attrs: &tracing::span::Attributes<'_>, id: &Id, _ctx: Context<'_, S>) {
        let mut visitor = LogTypeVisitor::new();
        attrs.record(&mut visitor);

        if let Some(log_type) = visitor.log_type {
            if let Ok(mut guard) = self.storage.span_log_types.write() {
                guard.insert(id.clone(), log_type);
            }
        }
    }

    fn on_close(&self, id: Id, _ctx: Context<'_, S>) {
        if let Ok(mut guard) = self.storage.span_log_types.write() {
            guard.remove(&id);
        }
    }

    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = LogTypeVisitor::new();
        event.record(&mut visitor);
        self.storage.set_current_event_log_type(visitor.log_type.clone());
    }

    fn event_enabled(&self, event: &Event<'_>, _ctx: Context<'_, S>) -> bool {
        let mut visitor = LogTypeVisitor::new();
        event.record(&mut visitor);
        self.storage.set_current_event_log_type(visitor.log_type);
        true
    }

    fn enabled(&self, _metadata: &Metadata<'_>, _ctx: Context<'_, S>) -> bool {
        true
    }
}

#[derive(Debug)]
struct LogTypeVisitor {
    log_type: Option<String>,
}

impl LogTypeVisitor {
    fn new() -> Self {
        Self { log_type: None }
    }
}

impl Visit for LogTypeVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "log_type" {
            let value_str = format!("{:?}", value);
            self.log_type = Some(value_str.trim_matches('"').to_string());
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "log_type" {
            self.log_type = Some(value.to_string());
        }
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        if field.name() == "log_type" {
            self.log_type = Some(value.to_string());
        }
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        if field.name() == "log_type" {
            self.log_type = Some(value.to_string());
        }
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        if field.name() == "log_type" {
            self.log_type = Some(value.to_string());
        }
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        if field.name() == "log_type" {
            self.log_type = Some(value.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::{debug, info_span};
    use tracing_subscriber::{layer::SubscriberExt};

    #[test]
    fn test_log_type_layer_span() {
        let layer = LogTypeLayer::new();
        let layer_clone = layer.clone();

        let subscriber = tracing_subscriber::registry().with(layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        let span = info_span!("test_span", log_type = "test_log_type");
        let span_id = span.id().expect("Span should have an ID");

        let _enter = span.enter();

        // Check that the log_type was captured
        assert_eq!(
            layer_clone.storage().span_log_types.read().unwrap().get(&span_id),
            Some(&"test_log_type".to_string())
        );
    }

    #[test]
    fn test_log_type_layer_event() {
        let layer = LogTypeLayer::new();
        let layer_clone = layer.clone();

        let subscriber = tracing_subscriber::registry().with(layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        // Create an event with log_type
        debug!(log_type = "debug_event", "This is a test event");

        // The event log_type should be captured temporarily
        let current_log_type = layer_clone.storage().get_current_log_type();
        assert_eq!(current_log_type, Some("debug_event".to_string()));
    }
}

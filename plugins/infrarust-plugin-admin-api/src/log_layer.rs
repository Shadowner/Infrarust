use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tokio::sync::broadcast;
use tracing::field::{Field, Visit};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

use crate::util::now_iso8601;

#[derive(Clone)]
pub struct LogBroadcast {
    pub tx: broadcast::Sender<LogEntry>,
    pub history: Arc<Mutex<VecDeque<LogEntry>>>,
    max_history: usize,
}

impl LogBroadcast {
    pub fn new(buffer_size: usize, max_history: usize) -> Self {
        let (tx, _) = broadcast::channel(buffer_size);
        Self {
            tx,
            history: Arc::new(Mutex::new(VecDeque::with_capacity(max_history))),
            max_history,
        }
    }

    pub fn layer(&self) -> BroadcastLogLayer {
        BroadcastLogLayer {
            log_tx: self.tx.clone(),
            history: Arc::clone(&self.history),
            max_history: self.max_history,
        }
    }
}

/// A captured log entry.
#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String,
    pub target: String,
    pub message: String,
    pub fields: HashMap<String, serde_json::Value>,
}

/// Custom tracing Layer that captures log events into a broadcast channel
/// and a ring buffer for history.
pub struct BroadcastLogLayer {
    log_tx: broadcast::Sender<LogEntry>,
    history: Arc<Mutex<VecDeque<LogEntry>>>,
    max_history: usize,
}

impl<S> Layer<S> for BroadcastLogLayer
where
    S: tracing::Subscriber,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut fields = HashMap::new();
        let mut visitor = JsonFieldVisitor(&mut fields);
        event.record(&mut visitor);

        let message = fields
            .remove("message")
            .map(|v| match v {
                serde_json::Value::String(s) => s,
                other => other.to_string(),
            })
            .unwrap_or_default();

        let entry = LogEntry {
            timestamp: now_iso8601(),
            level: event.metadata().level().to_string(),
            target: event.metadata().target().to_string(),
            message,
            fields,
        };

        // Push to ring buffer (std::sync::Mutex — held ~100ns)
        match self.history.lock() {
            Ok(mut history) => {
                if history.len() >= self.max_history {
                    history.pop_front();
                }
                history.push_back(entry.clone());
            }
            Err(_) => {
                // Lock poisoned — another thread panicked while holding it.
                // Can't use tracing here (we're inside the tracing layer), so just skip.
            }
        }

        // Broadcast (no-op if nobody is listening)
        let _ = self.log_tx.send(entry);
    }
}

/// Visitor that extracts tracing event fields into a JSON map.
struct JsonFieldVisitor<'a>(&'a mut HashMap<String, serde_json::Value>);

impl Visit for JsonFieldVisitor<'_> {
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.0
            .insert(field.name().to_string(), serde_json::json!(value));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0
            .insert(field.name().to_string(), serde_json::json!(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0
            .insert(field.name().to_string(), serde_json::json!(value));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.0
            .insert(field.name().to_string(), serde_json::json!(value));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0
            .insert(field.name().to_string(), serde_json::json!(value));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(
            field.name().to_string(),
            serde_json::json!(format!("{value:?}")),
        );
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn log_broadcast_new() {
        let lb = LogBroadcast::new(64, 100);
        let mut rx = lb.tx.subscribe();

        let entry = LogEntry {
            timestamp: "2026-01-01T00:00:00Z".into(),
            level: "INFO".into(),
            target: "test".into(),
            message: "hello".into(),
            fields: HashMap::new(),
        };

        assert!(lb.tx.send(entry.clone()).is_ok());
        let received = rx.try_recv().unwrap();
        assert_eq!(received.message, "hello");
    }

    #[test]
    fn the_layer_keeps_only_the_newest_entries() {
        use tracing_subscriber::layer::SubscriberExt;

        let lb = LogBroadcast::new(64, 3);
        let subscriber = tracing_subscriber::registry().with(lb.layer());

        tracing::subscriber::with_default(subscriber, || {
            for i in 0..5 {
                tracing::info!("msg {i}");
            }
        });

        let history = lb.history.lock().unwrap();
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].message, "msg 2");
        assert_eq!(history[2].message, "msg 4");
        assert_eq!(history[2].level, "INFO");
    }

    #[test]
    fn log_entry_serializes() {
        let entry = LogEntry {
            timestamp: "2026-01-01T00:00:00Z".into(),
            level: "WARN".into(),
            target: "infrarust_core::proxy".into(),
            message: "connection lost".into(),
            fields: HashMap::from([("player".into(), serde_json::json!("Steve"))]),
        };

        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("connection lost"));
        assert!(json.contains("Steve"));
    }
}

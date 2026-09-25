use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct LogCapture {
    level: tracing::Level,
    lines: Arc<Mutex<Vec<String>>>,
}

impl LogCapture {
    pub fn at(level: tracing::Level) -> Self {
        Self {
            level,
            lines: Arc::default(),
        }
    }

    pub fn lines(&self) -> Vec<String> {
        self.lines.lock().unwrap().clone()
    }

    pub fn matching(&self, needle: &str) -> Vec<String> {
        self.lines()
            .into_iter()
            .filter(|line| line.contains(needle))
            .collect()
    }

    pub fn at_level(&self, level: tracing::Level) -> Vec<String> {
        let prefix = format!("{level} ");
        self.lines()
            .into_iter()
            .filter(|line| line.starts_with(&prefix))
            .collect()
    }
}

struct EventText(String);

impl tracing::field::Visit for EventText {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.push_str(&format!("{}={value:?} ", field.name()));
    }
}

impl tracing::Subscriber for LogCapture {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        *metadata.level() <= self.level
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut text = EventText(format!("{} ", event.metadata().level()));
        event.record(&mut text);
        self.lines.lock().unwrap().push(text.0);
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

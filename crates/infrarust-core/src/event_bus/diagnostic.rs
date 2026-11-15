use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandlerDiagnostic {
    pub owner: Arc<str>,
    pub fired_by: Arc<str>,
    pub event: &'static str,
    pub kind: DiagnosticKind,
    pub elapsed: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticKind {
    Panicked { message: String },
    TimedOut,
    Slow,
    QueueFull,
}

pub(crate) fn short_type_name(full: &'static str) -> &'static str {
    let generics = full.find('<').unwrap_or(full.len());
    let start = full[..generics].rfind("::").map_or(0, |i| i + 2);
    &full[start..]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_type_name_strips_the_module_path() {
        assert_eq!(short_type_name("a::b::PreLoginEvent"), "PreLoginEvent");
        assert_eq!(short_type_name("Plain"), "Plain");
        assert_eq!(short_type_name("a::Wrapper<b::Inner>"), "Wrapper<b::Inner>");
    }
}

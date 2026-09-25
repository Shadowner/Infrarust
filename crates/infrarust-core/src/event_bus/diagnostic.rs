use std::any::Any;
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
}

pub(crate) fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_string()
    }
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

    #[test]
    fn panic_message_reads_string_payloads() {
        let from_str: Box<dyn Any + Send> = Box::new("boom");
        let from_string: Box<dyn Any + Send> = Box::new(String::from("bang"));
        let other: Box<dyn Any + Send> = Box::new(7_u8);
        assert_eq!(panic_message(from_str.as_ref()), "boom");
        assert_eq!(panic_message(from_string.as_ref()), "bang");
        assert_eq!(panic_message(other.as_ref()), "non-string panic payload");
    }
}

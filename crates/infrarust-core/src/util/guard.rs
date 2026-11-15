use std::any::Any;
use std::fmt;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::time::Duration;

use futures_util::FutureExt;

#[derive(Debug)]
pub(crate) enum Unanswered {
    Panicked(String),
    TimedOut(Duration),
}

impl fmt::Display for Unanswered {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Panicked(message) => write!(f, "panicked: {message}"),
            Self::TimedOut(limit) => write!(f, "did not answer within {limit:?}"),
        }
    }
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

pub(crate) async fn guarded<F: Future>(
    limit: Duration,
    start: impl FnOnce() -> F,
) -> Result<F::Output, Unanswered> {
    let run = AssertUnwindSafe(async move { start().await }).catch_unwind();
    match tokio::time::timeout(limit, run).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(payload)) => Err(Unanswered::Panicked(panic_message(payload.as_ref()))),
        Err(_) => Err(Unanswered::TimedOut(limit)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

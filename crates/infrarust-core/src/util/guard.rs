use std::fmt;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::time::Duration;

use futures_util::FutureExt;

use crate::event_bus::diagnostic::panic_message;

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

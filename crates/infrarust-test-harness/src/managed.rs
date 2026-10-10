use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use infrarust_server_manager::{ProviderStatus, ServerManagerError, ServerProvider};
use tokio::sync::watch;

use crate::error::{HarnessError, HarnessResult};

type ProviderFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, ServerManagerError>> + Send + 'a>>;

#[derive(Debug)]
pub struct FakeServerProvider {
    status: watch::Sender<ProviderStatus>,
    starts: watch::Sender<usize>,
    refuses_to_start: bool,
}

impl FakeServerProvider {
    pub fn sleeping() -> Arc<Self> {
        Arc::new(Self::new(false))
    }

    pub fn broken() -> Arc<Self> {
        Arc::new(Self::new(true))
    }

    fn new(refuses_to_start: bool) -> Self {
        Self {
            status: watch::Sender::new(ProviderStatus::Stopped),
            starts: watch::Sender::new(0),
            refuses_to_start,
        }
    }

    pub fn starts(&self) -> usize {
        *self.starts.borrow()
    }

    pub async fn wait_for_start(&self, timeout: Duration) -> HarnessResult<()> {
        let mut starts = self.starts.subscribe();
        tokio::time::timeout(timeout, starts.wait_for(|count| *count > 0))
            .await
            .map_err(|_| HarnessError::timeout("the managed server to be started", timeout))?
            .map_err(|_| HarnessError::Closed("the managed server".to_string()))?;
        Ok(())
    }

    pub fn boot(&self) {
        self.status.send_replace(ProviderStatus::Running);
    }
}

impl ServerProvider for FakeServerProvider {
    fn start(&self) -> ProviderFuture<'_, ()> {
        Box::pin(async move {
            if self.refuses_to_start {
                self.starts.send_modify(|count| *count += 1);
                return Err(ServerManagerError::Provider {
                    server_id: "fake".to_string(),
                    message: "the fake provider refuses to start".to_string(),
                });
            }
            self.status.send_replace(ProviderStatus::Starting);
            self.starts.send_modify(|count| *count += 1);
            Ok(())
        })
    }

    fn stop(&self) -> ProviderFuture<'_, ()> {
        Box::pin(async move {
            self.status.send_replace(ProviderStatus::Stopped);
            Ok(())
        })
    }

    fn check_status(&self) -> ProviderFuture<'_, ProviderStatus> {
        Box::pin(async move { Ok(*self.status.borrow()) })
    }

    fn provider_type(&self) -> &'static str {
        "fake"
    }
}

use infrarust_api::BoxFuture;
use infrarust_api::services::{InstallRequest, ProxyUpdates, UpdateRequestError, UpdateStatus};

use super::service::UpdateService;

impl infrarust_api::__sealed::Sealed for UpdateService {}

impl ProxyUpdates for UpdateService {
    fn status(&self) -> UpdateStatus {
        Self::status(self)
    }

    fn check_now(&self) -> BoxFuture<'_, UpdateStatus> {
        Box::pin(async move {
            if let Err(e) = self.check(None).await {
                tracing::debug!(error = %e, "update check failed");
            }
            Self::status(self)
        })
    }

    fn start_install(&self, request: InstallRequest) -> Result<(), UpdateRequestError> {
        Self::start_install(self, request)
    }
}

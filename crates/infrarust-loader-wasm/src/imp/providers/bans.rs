use std::sync::{Mutex, PoisonError};

use infrarust_api::error::ServiceError;
use infrarust_api::event::BoxFuture;
use infrarust_api::services::ban_service::{
    BanEntry, BanFeatures, BanPage, BanProvider, BanQuery, BanRequest, BanSource, BanTarget,
    BanVerdict, LoginAttempt, UnbanRequest,
};
use wasmtime::Store;

use crate::actor::InstanceRef;
use crate::bindings::Plugin as PluginBindings;
use crate::bindings::infrarust::plugin::ban_service as wb;
use crate::component;
use crate::convert;
use crate::store_state::PluginStoreState;

pub(crate) struct WasmBanProvider {
    instance: InstanceRef,
    features: Mutex<BanFeatures>,
}

impl WasmBanProvider {
    pub(crate) fn new(instance: InstanceRef, features: BanFeatures) -> Self {
        Self {
            instance,
            features: Mutex::new(features),
        }
    }

    pub(crate) fn set_features(&self, features: BanFeatures) {
        *self.features.lock().unwrap_or_else(PoisonError::into_inner) = features;
    }

    fn plugin(&self) -> &str {
        self.instance.plugin_id()
    }

    async fn ask<T, F>(&self, op: &'static str, call: F) -> Result<T, ServiceError>
    where
        T: Send + 'static,
        F: for<'a> FnOnce(
                &'a mut Store<PluginStoreState>,
                &'a PluginBindings,
            ) -> BoxFuture<'a, wasmtime::Result<Result<T, String>>>
            + Send
            + 'static,
    {
        if self.instance.is_upstream() {
            return Err(ServiceError::Unavailable(format!(
                "the ban provider `{}` is running the call that led here; a ban provider cannot use the ban service from inside its own calls",
                self.plugin()
            )));
        }
        match self.instance.call_bounded(op, call).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(message)) => Err(ServiceError::OperationFailed(message)),
            Err(failure) => Err(ServiceError::Unavailable(format!(
                "the ban provider `{}` did not answer: {failure}",
                self.plugin()
            ))),
        }
    }

    fn entry(&self, record: wb::BanRecord) -> Result<BanEntry, ServiceError> {
        convert::ban_record_from_wit(record).map_err(|error| {
            ServiceError::OperationFailed(format!(
                "the ban provider `{}` answered an invalid entry: {}",
                self.plugin(),
                error.message
            ))
        })
    }

    fn verdict(&self, verdict: wb::BanVerdict) -> Result<BanVerdict, ServiceError> {
        let native = BanVerdict::new(self.entry(verdict.entry)?);
        Ok(match verdict.kick_message {
            Some(message) => native.message(component::from_wit_or_fallback(
                &message,
                self.plugin(),
                "ban kick message",
            )),
            None => native,
        })
    }
}

impl BanProvider for WasmBanProvider {
    fn check<'a>(
        &'a self,
        attempt: &'a LoginAttempt,
    ) -> BoxFuture<'a, Result<Option<BanVerdict>, ServiceError>> {
        let attempt = convert::login_attempt_to_wit(attempt);
        Box::pin(async move {
            let verdict = self
                .ask("ban-provider-check", move |store, bindings| {
                    Box::pin(async move {
                        bindings
                            .infrarust_plugin_guest()
                            .call_ban_provider_check(&mut *store, &attempt)
                            .await
                    })
                })
                .await?;
            verdict.map(|verdict| self.verdict(verdict)).transpose()
        })
    }

    fn ban(&self, request: BanRequest) -> BoxFuture<'_, Result<BanEntry, ServiceError>> {
        let source =
            convert::ban_source_to_wit(request.source.as_ref().unwrap_or(&BanSource::System));
        let request = convert::ban_request_to_wit(&request);
        Box::pin(async move {
            let record = self
                .ask("ban-provider-ban", move |store, bindings| {
                    Box::pin(async move {
                        bindings
                            .infrarust_plugin_guest()
                            .call_ban_provider_ban(&mut *store, &request, &source)
                            .await
                    })
                })
                .await?;
            self.entry(record)
        })
    }

    fn unban(
        &self,
        request: UnbanRequest,
    ) -> BoxFuture<'_, Result<Option<BanEntry>, ServiceError>> {
        let request = convert::unban_request_to_wit(&request);
        Box::pin(async move {
            let record = self
                .ask("ban-provider-unban", move |store, bindings| {
                    Box::pin(async move {
                        bindings
                            .infrarust_plugin_guest()
                            .call_ban_provider_unban(&mut *store, &request)
                            .await
                    })
                })
                .await?;
            record.map(|record| self.entry(record)).transpose()
        })
    }

    fn get<'a>(
        &'a self,
        target: &'a BanTarget,
    ) -> BoxFuture<'a, Result<Option<BanEntry>, ServiceError>> {
        let target = convert::ban_target_to_wit(target);
        Box::pin(async move {
            let record = self
                .ask("ban-provider-get", move |store, bindings| {
                    Box::pin(async move {
                        bindings
                            .infrarust_plugin_guest()
                            .call_ban_provider_get(&mut *store, &target)
                            .await
                    })
                })
                .await?;
            record.map(|record| self.entry(record)).transpose()
        })
    }

    fn list(&self, query: BanQuery) -> BoxFuture<'_, Result<BanPage, ServiceError>> {
        let query = convert::ban_query_to_wit(&query);
        Box::pin(async move {
            let page = self
                .ask("ban-provider-list", move |store, bindings| {
                    Box::pin(async move {
                        bindings
                            .infrarust_plugin_guest()
                            .call_ban_provider_list(&mut *store, &query)
                            .await
                    })
                })
                .await?;
            let entries = page
                .entries
                .into_iter()
                .map(|record| self.entry(record))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(BanPage::new(entries, page.next_cursor))
        })
    }

    fn features(&self) -> BanFeatures {
        *self.features.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

use std::sync::Arc;

use infrarust_api::permissions::{Capability, PermissionProvider, PermissionProviderRejected};
use infrarust_api::services::ban_service::{BanFeatures, BanProvider, BanProviderRejected};

use crate::actor::CallKind;
use crate::bindings::infrarust::plugin::ban_service as wb;
use crate::bindings::infrarust::plugin::providers as wpr;
use crate::bindings::infrarust::plugin::types::ErrorKind;
use crate::host_error::{HostResult, host_error, missing_capability};
use crate::providers::{WasmBanProvider, WasmPermissionProvider};
use crate::store_state::PluginStoreState;

impl wpr::Host for PluginStoreState {
    async fn register_ban_provider(
        &mut self,
        features: wb::BanFeatures,
    ) -> wasmtime::Result<HostResult<()>> {
        Ok(self.provide_bans(&features))
    }

    async fn register_permission_provider(&mut self) -> wasmtime::Result<HostResult<()>> {
        Ok(self.provide_permissions())
    }
}

impl PluginStoreState {
    fn provide_bans(&mut self, features: &wb::BanFeatures) -> HostResult<()> {
        self.check(Capability::BanProvider, "providers.register-ban-provider")?;
        let ctx = self.services()?;
        let features = BanFeatures::new()
            .ip_ranges(features.ip_ranges)
            .pagination(features.pagination);
        if let Some(registered) = self.registrations().registered_ban_provider() {
            registered.set_features(features);
            return Ok(());
        }
        let instance = self.instance_ref(CallKind::Event).any_generation();
        let provider = Arc::new(WasmBanProvider::new(instance, features));
        ctx.register_ban_provider(Arc::clone(&provider) as Arc<dyn BanProvider>)
            .map_err(|rejected| match rejected {
                BanProviderRejected::MissingCapability => {
                    missing_capability(Capability::BanProvider)
                }
                other => host_error(ErrorKind::Conflict, other.to_string()),
            })?;
        self.registrations().keep_ban_provider(provider);
        Ok(())
    }

    fn provide_permissions(&mut self) -> HostResult<()> {
        self.check(
            Capability::PermissionProvider,
            "providers.register-permission-provider",
        )?;
        let ctx = self.services()?;
        if self
            .registrations()
            .registered_permission_provider()
            .is_some()
        {
            return Ok(());
        }
        let instance = self.instance_ref(CallKind::Event).any_generation();
        let provider = Arc::new(WasmPermissionProvider::new(instance));
        ctx.register_permission_provider(Arc::clone(&provider) as Arc<dyn PermissionProvider>)
            .map_err(|rejected| match rejected {
                PermissionProviderRejected::MissingCapability => {
                    missing_capability(Capability::PermissionProvider)
                }
                other => host_error(ErrorKind::Conflict, other.to_string()),
            })?;
        self.registrations().keep_permission_provider(provider);
        Ok(())
    }
}

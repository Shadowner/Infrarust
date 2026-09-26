use std::sync::Arc;

use infrarust_api::event::BoxFuture;
use infrarust_api::permissions::{
    PermissionChecker, PermissionProvider, PermissionSnapshot, PermissionSubject,
};

use crate::actor::InstanceRef;
use crate::convert;
use crate::snapshots::snapshot_from_wit;

pub(crate) struct WasmPermissionProvider {
    instance: InstanceRef,
}

impl WasmPermissionProvider {
    pub(crate) fn new(instance: InstanceRef) -> Self {
        Self { instance }
    }

    async fn ask(&self, subject: &PermissionSubject) -> Result<PermissionSnapshot, String> {
        let subject = convert::permission_subject_to_wit(subject);
        let snapshot = self
            .instance
            .call_bounded("permission-snapshot-for", move |store, bindings| {
                Box::pin(async move {
                    bindings
                        .infrarust_plugin_guest()
                        .call_permission_snapshot_for(&mut *store, &subject)
                        .await
                })
            })
            .await
            .map_err(|failure| failure.to_string())?;
        snapshot_from_wit(&snapshot)
    }

    fn held(&self, subject: &PermissionSubject) -> Arc<dyn PermissionChecker> {
        let held = subject
            .player_id()
            .and_then(|player| self.instance.snapshots().live(player));
        match held {
            Some(checker) => checker,
            None => {
                tracing::debug!(
                    plugin = self.instance.plugin_id(),
                    subject = name(subject),
                    "wasm permission provider is running the call that asked for this checker and holds no snapshot for it; the subject gets the node defaults"
                );
                Arc::new(PermissionSnapshot::new())
            }
        }
    }
}

fn name(subject: &PermissionSubject) -> &str {
    subject
        .profile()
        .map_or("console", |profile| profile.username.as_str())
}

impl PermissionProvider for WasmPermissionProvider {
    fn create_checker<'a>(
        &'a self,
        subject: &'a PermissionSubject,
    ) -> BoxFuture<'a, Arc<dyn PermissionChecker>> {
        Box::pin(async move {
            if self.instance.is_upstream() {
                return self.held(subject);
            }
            let snapshot = match self.ask(subject).await {
                Ok(snapshot) => snapshot,
                Err(reason) => {
                    if let Some(suppressed) = self.instance.admit_warning() {
                        tracing::warn!(plugin = self.instance.plugin_id(), subject = name(subject),
                            %reason, suppressed,
                            "wasm permission provider gave no snapshot; the subject gets the node defaults");
                    }
                    PermissionSnapshot::new()
                }
            };
            match subject.player_id() {
                Some(player) => self.instance.snapshots().install(player, snapshot)
                    as Arc<dyn PermissionChecker>,
                None => Arc::new(snapshot),
            }
        })
    }
}

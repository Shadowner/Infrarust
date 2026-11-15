use crate::actor::InstanceRef;

pub(crate) async fn run_scheduled_task(instance: InstanceRef, handler: u64) {
    let _ = instance
        .call_or_none("on-scheduled-task", move |store, bindings| {
            Box::pin(async move {
                bindings
                    .infrarust_plugin_guest()
                    .call_on_scheduled_task(&mut *store, handler)
                    .await
            })
        })
        .await;
}

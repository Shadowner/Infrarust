//! v0.2-B `multi-handler` fixture. Registers several `PostLogin` handlers at
//! different priorities cancels one before any event
//! fires, and appends each handler's tag to a marker file. The host test asserts
//! priority ordering, multi-handler delivery, and independent cancellation.

use std::io::Write;

use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct MultiHandler;

#[plugin(id = "multi-handler", name = "Multi Handler Fixture")]
impl Plugin for MultiHandler {
    fn on_enable(&self, ctx: &Context) -> Result<(), String> {
        ctx.on::<PostLoginEvent>(EventPriority::First, |_| append("A"));
        ctx.on::<PostLoginEvent>(EventPriority::Custom(32), |_| append("B"));
        ctx.on::<PostLoginEvent>(EventPriority::Normal, |_| append("C"));
        let leaked = ctx.on::<PostLoginEvent>(EventPriority::Normal, |_| append("L"));
        ctx.on::<PostLoginEvent>(EventPriority::Last, |_| append("D"));
        leaked.cancel();
        Ok(())
    }
}

fn append(tag: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("multi.marker")
    {
        let _ = f.write_all(tag.as_bytes());
    }
}

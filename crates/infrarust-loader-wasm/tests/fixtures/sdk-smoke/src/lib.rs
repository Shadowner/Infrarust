use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct SdkSmoke;

#[plugin(id = "sdk-smoke", name = "SDK Smoke")]
impl Plugin for SdkSmoke {
    fn on_enable(&self, _ctx: &Context) -> Result<(), String> {
        std::fs::write("sdk-smoke.marker", b"enabled").map_err(|e| e.to_string())
    }
}

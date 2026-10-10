use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct HostCaller;

#[plugin(id = "host-caller", name = "Host Caller Fixture")]
impl Plugin for HostCaller {
    fn on_enable(&self, _ctx: &Context) -> Result<(), PluginError> {
        std::fs::write("count.txt", Players::count().to_string())?;
        if let Ok(Some(greeting)) = Config::get("greeting") {
            std::fs::write("greeting.txt", greeting)?;
        }
        Ok(())
    }
}

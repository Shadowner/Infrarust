use crate::bindings::infrarust::plugin::events as we;
use crate::bindings::infrarust::plugin::types as wt;
use crate::events::{EventDetails, PingDetails};
use crate::store_state::PluginStoreState;

impl PluginStoreState {
    fn ping(&self) -> Option<&PingDetails> {
        match self.event_details()? {
            EventDetails::Ping(ping) => Some(ping),
        }
    }
}

impl we::Host for PluginStoreState {
    async fn ping_description(&mut self) -> wasmtime::Result<Option<wt::Component>> {
        Ok(self.ping().map(PingDetails::description_to_wit))
    }

    async fn ping_favicon(&mut self) -> wasmtime::Result<Option<String>> {
        Ok(self.ping().and_then(|ping| ping.favicon.clone()))
    }

    async fn ping_player_sample(&mut self) -> wasmtime::Result<Vec<we::PingPlayer>> {
        Ok(self
            .ping()
            .map(PingDetails::player_sample_to_wit)
            .unwrap_or_default())
    }
}

#![allow(clippy::all)]

wit_bindgen::generate!({
    world: "plugin",
    path: "../infrarust-plugin-wit/wit",
    generate_all,
    pub_export_macro: true,
    export_macro_name: "export",
    default_bindings_module: "infrarust_plugin_sdk::bindings",
    additional_derives: [PartialEq],
});

pub use exports::infrarust::plugin::{codec_filter, guest};
pub use infrarust::plugin::{
    ban_service, codec_registry, command_manager, config_service, event_bus, events, limbo,
    load_balancer, log, messaging, players, plugin_registry, proxy_info, scheduler, server_manager,
    text, types,
};

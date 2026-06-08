//! The single `wit-bindgen` generation for the whole SDK.
//!
//! `pub_export_macro` keeps this rlib export-free (so any guest crate can link
//! it) and emits the component `export!` shims only at the macro's call site

#![allow(clippy::all)]

wit_bindgen::generate!({
    world: "plugin",
    path: "../infrarust-plugin-wit/wit",
    generate_all,
    pub_export_macro: true,
    export_macro_name: "export",
    default_bindings_module: "infrarust_plugin_sdk::bindings",
});

pub use exports::infrarust::plugin::{codec_filter, guest};
pub use infrarust::plugin::{
    ban_service, codec_registry, command_manager, config_service, event_bus, limbo, log,
    player_registry, scheduler, server_manager, types,
};

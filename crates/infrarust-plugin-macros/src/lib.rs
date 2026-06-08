//! Proc-macros for `infrarust-plugin-sdk`.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::{Expr, ImplItem, ItemImpl, Lit, MetaNameValue, Token, parse_macro_input};

/// Turn an `impl Plugin for MyPlugin` block into a loadable WASM component.
///
/// Generates the WIT `Guest` glue and the component `export!`. `metadata()` is
/// derived from `Cargo.toml` (`CARGO_PKG_*`) unless the impl defines its own,
/// and individual fields can be overridden: `#[plugin(id = "...", name = "...")]`.
/// The plugin type must implement `Default`.
#[proc_macro_attribute]
pub fn plugin(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args =
        parse_macro_input!(attr with Punctuated::<MetaNameValue, Token![,]>::parse_terminated);
    let mut item_impl = parse_macro_input!(item as ItemImpl);

    let overrides = match Overrides::from_args(&args) {
        Ok(o) => o,
        Err(e) => return e.to_compile_error().into(),
    };

    let ty = item_impl.self_ty.clone();

    let has_metadata = item_impl
        .items
        .iter()
        .any(|i| matches!(i, ImplItem::Fn(f) if f.sig.ident == "metadata"));
    if !has_metadata {
        let metadata_fn = generate_metadata_fn(&overrides);
        item_impl.items.push(syn::parse_quote!(#metadata_fn));
    }

    let glue = generate_guest_glue(&ty);

    quote! {
        #item_impl
        #glue
    }
    .into()
}

#[derive(Default)]
struct Overrides {
    id: Option<String>,
    name: Option<String>,
    version: Option<String>,
    description: Option<String>,
    authors: Option<String>,
}

impl Overrides {
    fn from_args(args: &Punctuated<MetaNameValue, Token![,]>) -> syn::Result<Self> {
        let mut out = Self::default();
        for arg in args {
            let key = arg
                .path
                .get_ident()
                .ok_or_else(|| syn::Error::new_spanned(&arg.path, "expected an identifier"))?
                .to_string();
            let value = string_lit(&arg.value)?;
            match key.as_str() {
                "id" => out.id = Some(value),
                "name" => out.name = Some(value),
                "version" => out.version = Some(value),
                "description" => out.description = Some(value),
                "authors" => out.authors = Some(value),
                _ => {
                    return Err(syn::Error::new_spanned(
                        &arg.path,
                        "unknown key (expected id, name, version, description, authors)",
                    ));
                }
            }
        }
        Ok(out)
    }
}

fn string_lit(expr: &Expr) -> syn::Result<String> {
    if let Expr::Lit(lit) = expr
        && let Lit::Str(s) = &lit.lit
    {
        return Ok(s.value());
    }
    Err(syn::Error::new_spanned(expr, "expected a string literal"))
}

fn generate_metadata_fn(o: &Overrides) -> TokenStream2 {
    let id =
        o.id.as_ref()
            .map_or_else(|| quote!(env!("CARGO_PKG_NAME")), |s| quote!(#s));
    let name = o
        .name
        .as_ref()
        .map_or_else(|| quote!(env!("CARGO_PKG_NAME")), |s| quote!(#s));
    let version = o
        .version
        .as_ref()
        .map_or_else(|| quote!(env!("CARGO_PKG_VERSION")), |s| quote!(#s));
    let authors = o
        .authors
        .as_ref()
        .map_or_else(|| quote!(env!("CARGO_PKG_AUTHORS")), |s| quote!(#s));
    let description = match &o.description {
        Some(d) => quote!(::core::option::Option::Some((#d).to_string())),
        None => quote!(match option_env!("CARGO_PKG_DESCRIPTION") {
            ::core::option::Option::Some(d) if !d.is_empty() => {
                ::core::option::Option::Some(d.to_string())
            }
            _ => ::core::option::Option::None,
        }),
    };

    quote! {
        fn metadata(&self) -> ::infrarust_plugin_sdk::PluginMetadata {
            ::infrarust_plugin_sdk::PluginMetadata {
                id: (#id).to_string(),
                name: (#name).to_string(),
                version: (#version).to_string(),
                authors: (#authors)
                    .split(':')
                    .filter(|s| !s.is_empty())
                    .map(::std::string::String::from)
                    .collect(),
                description: #description,
                dependencies: ::std::vec::Vec::new(),
            }
        }
    }
}

fn generate_guest_glue(ty: &syn::Type) -> TokenStream2 {
    quote! {
        #[doc(hidden)]
        struct __InfrarustPluginComponent;

        impl ::infrarust_plugin_sdk::bindings::guest::Guest for __InfrarustPluginComponent {
            fn metadata() -> ::infrarust_plugin_sdk::PluginMetadata {
                <#ty as ::infrarust_plugin_sdk::Plugin>::metadata(
                    &<#ty as ::core::default::Default>::default(),
                )
            }
            fn on_enable() -> ::core::result::Result<(), ::std::string::String> {
                ::infrarust_plugin_sdk::runtime::on_enable::<#ty>()
            }
            fn on_disable() -> ::core::result::Result<(), ::std::string::String> {
                ::infrarust_plugin_sdk::runtime::on_disable()
            }
            fn handle_event(
                listener: u64,
                ev: ::infrarust_plugin_sdk::bindings::guest::Event,
            ) -> ::infrarust_plugin_sdk::bindings::guest::EventOutcome {
                ::infrarust_plugin_sdk::runtime::handle_event(listener, ev)
            }
            fn handle_command(
                callback_id: u64,
                args: ::std::vec::Vec<::std::string::String>,
                player: ::core::option::Option<u64>,
            ) {
                ::infrarust_plugin_sdk::runtime::handle_command(callback_id, args, player)
            }
            fn tab_complete(
                callback_id: u64,
                partial: ::std::vec::Vec<::std::string::String>,
                cursor: u32,
            ) -> ::std::vec::Vec<::std::string::String> {
                ::infrarust_plugin_sdk::runtime::tab_complete(callback_id, partial, cursor)
            }
            fn on_scheduled_task(callback_id: u64) {
                ::infrarust_plugin_sdk::runtime::on_scheduled_task(callback_id)
            }

            fn limbo_on_player_enter(
                _handler: u64,
                _session: &::infrarust_plugin_sdk::bindings::guest::LimboSession,
            ) -> ::infrarust_plugin_sdk::bindings::guest::HandlerResult {
                ::infrarust_plugin_sdk::bindings::guest::HandlerResult::Accept
            }
            fn limbo_on_command(
                _handler: u64,
                _session: &::infrarust_plugin_sdk::bindings::guest::LimboSession,
                _command: ::std::string::String,
                _args: ::std::vec::Vec<::std::string::String>,
            ) {
            }
            fn limbo_on_chat(
                _handler: u64,
                _session: &::infrarust_plugin_sdk::bindings::guest::LimboSession,
                _message: ::std::string::String,
            ) {
            }
            fn limbo_on_disconnect(_handler: u64, _player: u64) {}
            fn permission_level_of(
                _handler: u64,
            ) -> ::infrarust_plugin_sdk::bindings::guest::PermissionLevel {
                ::infrarust_plugin_sdk::bindings::guest::PermissionLevel::Player
            }
            fn check_permission(_handler: u64, _permission: ::std::string::String) -> bool {
                false
            }
        }

        impl ::infrarust_plugin_sdk::bindings::codec_filter::Guest for __InfrarustPluginComponent {
            type FilterInstance = ::infrarust_plugin_sdk::runtime::FilterInstanceProxy;
            fn create(
                factory: u64,
                init: ::infrarust_plugin_sdk::bindings::codec_filter::CodecSessionInit,
            ) -> ::infrarust_plugin_sdk::bindings::codec_filter::FilterInstance {
                ::infrarust_plugin_sdk::bindings::codec_filter::FilterInstance::new(
                    ::infrarust_plugin_sdk::runtime::create_codec_filter::<#ty>(factory, init),
                )
            }
        }

        ::infrarust_plugin_sdk::export!(__InfrarustPluginComponent);
    }
}

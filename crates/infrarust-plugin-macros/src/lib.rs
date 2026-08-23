//! Proc-macros for `infrarust-plugin-sdk`.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::{Expr, ImplItem, ItemImpl, Lit, MetaNameValue, Token};

/// Turn an `impl Plugin for MyPlugin` block into a loadable WASM component.
///
/// Generates the WIT `Guest` glue and the component `export!`. `metadata()` is
/// derived from `Cargo.toml` (`CARGO_PKG_*`) unless the impl defines its own,
/// and individual fields can be overridden: `#[plugin(id = "...", name = "...")]`
/// (overrides cannot be combined with a user-defined `metadata()`).
/// The plugin type must implement `Default`.
#[proc_macro_attribute]
pub fn plugin(attr: TokenStream, item: TokenStream) -> TokenStream {
    expand(attr.into(), item.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn expand(attr: TokenStream2, item: TokenStream2) -> syn::Result<TokenStream2> {
    let args = Punctuated::<MetaNameValue, Token![,]>::parse_terminated.parse2(attr)?;
    let mut item_impl: ItemImpl = syn::parse2(item)?;
    validate_impl(&item_impl)?;

    let ty = item_impl.self_ty.clone();

    let has_metadata = item_impl
        .items
        .iter()
        .any(|i| matches!(i, ImplItem::Fn(f) if f.sig.ident == "metadata"));
    if has_metadata {
        if !args.is_empty() {
            return Err(syn::Error::new_spanned(
                &args,
                "#[plugin] metadata overrides are ignored because this impl defines its own \
                 `metadata()`; remove the attribute arguments or the method",
            ));
        }
    } else {
        let metadata_fn = generate_metadata_fn(&Overrides::from_args(&args)?);
        item_impl.items.push(syn::parse_quote!(#metadata_fn));
    }

    let glue = generate_guest_glue(&ty);

    Ok(quote! {
        #item_impl
        #glue
    })
}

fn validate_impl(item_impl: &ItemImpl) -> syn::Result<()> {
    let is_plugin_trait = item_impl.trait_.as_ref().is_some_and(|(bang, path, _)| {
        bang.is_none() && path.segments.last().is_some_and(|s| s.ident == "Plugin")
    });
    if !is_plugin_trait {
        return Err(syn::Error::new_spanned(
            &item_impl.self_ty,
            "#[plugin] must be applied to an `impl Plugin for MyPlugin` block",
        ));
    }
    if !item_impl.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &item_impl.generics,
            "#[plugin] requires a concrete plugin type; generic impls are not supported",
        ));
    }
    Ok(())
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
            let slot = match key.as_str() {
                "id" => &mut out.id,
                "name" => &mut out.name,
                "version" => &mut out.version,
                "description" => &mut out.description,
                "authors" => &mut out.authors,
                _ => {
                    return Err(syn::Error::new_spanned(
                        &arg.path,
                        "unknown key (expected id, name, version, description, authors)",
                    ));
                }
            };
            if slot.replace(value).is_some() {
                return Err(syn::Error::new_spanned(
                    arg,
                    format!("duplicate key `{key}`"),
                ));
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
                handler: u64,
                session: &::infrarust_plugin_sdk::bindings::guest::LimboSession,
            ) -> ::infrarust_plugin_sdk::bindings::guest::HandlerResult {
                ::infrarust_plugin_sdk::runtime::limbo_on_player_enter(handler, session)
            }
            fn limbo_on_command(
                handler: u64,
                session: &::infrarust_plugin_sdk::bindings::guest::LimboSession,
                command: ::std::string::String,
                args: ::std::vec::Vec<::std::string::String>,
            ) {
                ::infrarust_plugin_sdk::runtime::limbo_on_command(handler, session, command, args)
            }
            fn limbo_on_chat(
                handler: u64,
                session: &::infrarust_plugin_sdk::bindings::guest::LimboSession,
                message: ::std::string::String,
            ) {
                ::infrarust_plugin_sdk::runtime::limbo_on_chat(handler, session, message)
            }
            fn limbo_on_disconnect(handler: u64, player: u64) {
                ::infrarust_plugin_sdk::runtime::limbo_on_disconnect(handler, player)
            }
            fn limbo_on_session_end(
                handler: u64,
                player: u64,
                reason: ::infrarust_plugin_sdk::bindings::guest::SessionEndReason,
            ) {
                ::infrarust_plugin_sdk::runtime::limbo_on_session_end(handler, player, reason)
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn expand_err(attr: TokenStream2, item: TokenStream2) -> String {
        expand(attr, item)
            .expect_err("expansion must fail")
            .to_string()
    }

    #[test]
    fn plugin_impl_accepted() {
        assert!(expand(quote!(), quote!(impl Plugin for Foo {})).is_ok());
        assert!(
            expand(
                quote!(),
                quote!(impl infrarust_plugin_sdk::Plugin for Foo {})
            )
            .is_ok()
        );
    }

    #[test]
    fn overrides_with_user_metadata_rejected() {
        let err = expand_err(
            quote!(id = "x"),
            quote! {
                impl Plugin for Foo {
                    fn metadata(&self) -> PluginMetadata {
                        unimplemented!()
                    }
                }
            },
        );
        assert!(err.contains("metadata()"), "{err}");
    }

    #[test]
    fn inherent_and_foreign_trait_impls_rejected() {
        let err = expand_err(quote!(), quote!(impl Foo {}));
        assert!(err.contains("impl Plugin for"), "{err}");
        let err = expand_err(quote!(), quote!(impl Display for Foo {}));
        assert!(err.contains("impl Plugin for"), "{err}");
    }

    #[test]
    fn generic_impl_rejected() {
        let err = expand_err(
            quote!(),
            quote!(
                impl<T> Plugin for Foo<T> {}
            ),
        );
        assert!(err.contains("generic"), "{err}");
    }

    #[test]
    fn duplicate_key_rejected() {
        let err = expand_err(quote!(id = "a", id = "b"), quote!(impl Plugin for Foo {}));
        assert!(err.contains("duplicate key `id`"), "{err}");
    }
}

//! Proc-macros for `infrarust-plugin-sdk`.

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::{Expr, ImplItem, ItemImpl, Lit, LitStr, MetaNameValue, Token};

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
        let metadata_fn = generate_metadata_fn(&Overrides::from_args(&args)?)?;
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
    id: Option<LitStr>,
    name: Option<String>,
    version: Option<String>,
    description: Option<String>,
    authors: Option<String>,
    depends: Vec<LitStr>,
    soft_depends: Vec<LitStr>,
}

impl Overrides {
    fn from_args(args: &Punctuated<MetaNameValue, Token![,]>) -> syn::Result<Self> {
        let mut out = Self::default();
        let mut seen = Vec::new();
        for arg in args {
            let key = arg
                .path
                .get_ident()
                .ok_or_else(|| syn::Error::new_spanned(&arg.path, "expected an identifier"))?
                .to_string();
            if seen.contains(&key) {
                return Err(syn::Error::new_spanned(
                    arg,
                    format!("duplicate key `{key}`"),
                ));
            }
            match key.as_str() {
                "id" => {
                    let id = string_lit(&arg.value)?;
                    validate_id(&id.value())
                        .map_err(|reason| syn::Error::new_spanned(&id, reason))?;
                    out.id = Some(id);
                }
                "name" => out.name = Some(string_lit(&arg.value)?.value()),
                "version" => out.version = Some(string_lit(&arg.value)?.value()),
                "description" => out.description = Some(string_lit(&arg.value)?.value()),
                "authors" => out.authors = Some(string_lit(&arg.value)?.value()),
                "depends" => out.depends = id_list(&arg.value)?,
                "soft_depends" => out.soft_depends = id_list(&arg.value)?,
                _ => {
                    return Err(syn::Error::new_spanned(
                        &arg.path,
                        "unknown key (expected id, name, version, description, authors, depends, soft_depends)",
                    ));
                }
            }
            seen.push(key);
        }
        Ok(out)
    }
}

fn string_lit(expr: &Expr) -> syn::Result<LitStr> {
    if let Expr::Lit(lit) = expr
        && let Lit::Str(s) = &lit.lit
    {
        return Ok(s.clone());
    }
    Err(syn::Error::new_spanned(expr, "expected a string literal"))
}

fn id_list(expr: &Expr) -> syn::Result<Vec<LitStr>> {
    let Expr::Array(array) = expr else {
        return Err(syn::Error::new_spanned(
            expr,
            "expected a list of plugin ids, like [\"auth\", \"stats\"]",
        ));
    };
    array
        .elems
        .iter()
        .map(|elem| {
            let id = string_lit(elem)?;
            validate_id(&id.value()).map_err(|reason| syn::Error::new_spanned(&id, reason))?;
            Ok(id)
        })
        .collect()
}

const MAX_ID_LEN: usize = 64;

fn validate_id(id: &str) -> Result<(), String> {
    let Some(first) = id.chars().next() else {
        return Err("a plugin id cannot be empty".to_owned());
    };
    if id.len() > MAX_ID_LEN {
        return Err(format!(
            "plugin id `{id}` is longer than {MAX_ID_LEN} characters"
        ));
    }
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return Err(format!(
            "plugin id `{id}` must start with a lowercase letter or a digit"
        ));
    }
    if let Some(bad) = id
        .chars()
        .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-' || *c == '_'))
    {
        return Err(format!(
            "plugin id `{id}` contains `{bad}`; use lowercase letters, digits, `-` and `_`"
        ));
    }
    Ok(())
}

fn generate_metadata_fn(o: &Overrides) -> syn::Result<TokenStream2> {
    let id = match &o.id {
        Some(id) => quote!(#id),
        None => {
            if let Ok(package) = std::env::var("CARGO_PKG_NAME") {
                validate_id(&package).map_err(|reason| {
                    syn::Error::new(
                        Span::call_site(),
                        format!(
                            "{reason}; the id defaults to the package name, set one with #[plugin(id = \"...\")]"
                        ),
                    )
                })?;
            }
            quote!(env!("CARGO_PKG_NAME"))
        }
    };
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
    let hard = o.depends.iter();
    let soft = o.soft_depends.iter();

    Ok(quote! {
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
                dependencies: ::std::vec![
                    #(::infrarust_plugin_sdk::PluginDependency {
                        id: (#hard).to_string(),
                        optional: false,
                    },)*
                    #(::infrarust_plugin_sdk::PluginDependency {
                        id: (#soft).to_string(),
                        optional: true,
                    },)*
                ],
            }
        }
    })
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
            fn on_enable(
                reason: ::infrarust_plugin_sdk::bindings::guest::EnableReason,
            ) -> ::core::result::Result<(), ::std::string::String> {
                ::infrarust_plugin_sdk::runtime::on_enable::<#ty>(reason)
            }
            fn on_disable(
                reason: ::infrarust_plugin_sdk::bindings::guest::DisableReason,
            ) -> ::core::result::Result<(), ::std::string::String> {
                ::infrarust_plugin_sdk::runtime::on_disable(reason)
            }
            fn handle_event(
                listener: u64,
                ev: ::infrarust_plugin_sdk::bindings::guest::Event,
            ) -> ::infrarust_plugin_sdk::bindings::guest::EventOutcome {
                ::infrarust_plugin_sdk::runtime::handle_event(listener, ev)
            }
            fn handle_command(
                handler: u64,
                invocation: ::infrarust_plugin_sdk::bindings::guest::CommandInvocation,
            ) {
                ::infrarust_plugin_sdk::runtime::handle_command(handler, invocation)
            }
            fn tab_complete(
                handler: u64,
                sender: ::infrarust_plugin_sdk::bindings::guest::CommandSender,
                args: ::std::vec::Vec<::std::string::String>,
                cursor: u32,
            ) -> ::std::vec::Vec<::infrarust_plugin_sdk::bindings::guest::Suggestion> {
                ::infrarust_plugin_sdk::runtime::tab_complete(handler, sender, args, cursor)
            }
            fn on_scheduled_task(handler: u64) {
                ::infrarust_plugin_sdk::runtime::on_scheduled_task(handler)
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

            fn ban_provider_check(
                attempt: ::infrarust_plugin_sdk::bindings::ban_service::LoginAttempt,
            ) -> ::core::result::Result<
                ::core::option::Option<::infrarust_plugin_sdk::bindings::ban_service::BanVerdict>,
                ::std::string::String,
            > {
                ::infrarust_plugin_sdk::runtime::ban_provider_check(attempt)
            }
            fn ban_provider_ban(
                request: ::infrarust_plugin_sdk::bindings::ban_service::BanRequest,
                source: ::infrarust_plugin_sdk::bindings::ban_service::BanSource,
            ) -> ::core::result::Result<
                ::infrarust_plugin_sdk::bindings::ban_service::BanRecord,
                ::std::string::String,
            > {
                ::infrarust_plugin_sdk::runtime::ban_provider_ban(request, source)
            }
            fn ban_provider_unban(
                request: ::infrarust_plugin_sdk::bindings::ban_service::UnbanRequest,
            ) -> ::core::result::Result<
                ::core::option::Option<::infrarust_plugin_sdk::bindings::ban_service::BanRecord>,
                ::std::string::String,
            > {
                ::infrarust_plugin_sdk::runtime::ban_provider_unban(request)
            }
            fn ban_provider_get(
                target: ::infrarust_plugin_sdk::bindings::ban_service::BanTarget,
            ) -> ::core::result::Result<
                ::core::option::Option<::infrarust_plugin_sdk::bindings::ban_service::BanRecord>,
                ::std::string::String,
            > {
                ::infrarust_plugin_sdk::runtime::ban_provider_get(target)
            }
            fn ban_provider_list(
                query: ::infrarust_plugin_sdk::bindings::ban_service::BanQuery,
            ) -> ::core::result::Result<
                ::infrarust_plugin_sdk::bindings::ban_service::BanRecordPage,
                ::std::string::String,
            > {
                ::infrarust_plugin_sdk::runtime::ban_provider_list(query)
            }
            fn permission_snapshot_for(
                subject: ::infrarust_plugin_sdk::bindings::permissions::PermissionSubject,
            ) -> ::infrarust_plugin_sdk::bindings::permissions::PermissionSnapshot {
                ::infrarust_plugin_sdk::runtime::permission_snapshot_for(subject)
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

    #[test]
    fn an_invalid_id_is_a_compile_error() {
        let err = expand_err(quote!(id = "My Plugin"), quote!(impl Plugin for Foo {}));
        assert!(err.contains("must start with a lowercase letter"), "{err}");
        let err = expand_err(quote!(id = "auth!"), quote!(impl Plugin for Foo {}));
        assert!(err.contains("contains `!`"), "{err}");
        let err = expand_err(quote!(id = ""), quote!(impl Plugin for Foo {}));
        assert!(err.contains("cannot be empty"), "{err}");
    }

    #[test]
    fn dependencies_are_validated_and_generated() {
        let expanded = expand(
            quote!(
                id = "stats",
                depends = ["auth"],
                soft_depends = ["admin-api"]
            ),
            quote!(impl Plugin for Foo {}),
        )
        .unwrap()
        .to_string();
        assert!(expanded.contains("\"auth\""), "{expanded}");
        assert!(expanded.contains("optional : true"), "{expanded}");
        let err = expand_err(quote!(depends = ["Auth"]), quote!(impl Plugin for Foo {}));
        assert!(err.contains("plugin id `Auth`"), "{err}");
        let err = expand_err(quote!(depends = "auth"), quote!(impl Plugin for Foo {}));
        assert!(err.contains("expected a list of plugin ids"), "{err}");
    }

    #[test]
    fn id_rules() {
        for good in ["stats", "admin-api", "a1_b2", "0day"] {
            assert_eq!(validate_id(good), Ok(()), "{good}");
        }
        for bad in ["-lead", "Upper", "sp ace", "dot.ted", &"x".repeat(65)] {
            assert!(validate_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_glue_targets_the_contract_exports() {
        let expanded = expand(quote!(id = "x"), quote!(impl Plugin for Foo {}))
            .unwrap()
            .to_string();
        for export in [
            "on_enable",
            "EnableReason",
            "DisableReason",
            "CommandInvocation",
            "Suggestion",
            "ban_provider_check",
            "ban_provider_list",
            "permission_snapshot_for",
        ] {
            assert!(expanded.contains(export), "{export} missing");
        }
        assert!(!expanded.contains("check_permission"));
    }
}

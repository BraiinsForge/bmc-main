// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

//! `#[widget]`: the host exports for an `impl Widget for …` block.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::punctuated::Punctuated;
use syn::{Attribute, ImplItem, ImplItemFn, ItemImpl, Meta, Token, Type};

/// The hooks a widget may leave out; the host calls only what the widget exports.
const OPTIONAL_HOOKS: [&str; 7] = [
    "on_wake",
    "on_sleep",
    "on_touch",
    "on_system_update",
    "on_params_update",
    "on_credentials_update",
    "on_network_update",
];

pub(crate) fn expand(attr: TokenStream, item: TokenStream) -> TokenStream {
    match try_expand(attr, item) {
        Ok(expanded) => expanded,
        Err(error) => error.to_compile_error(),
    }
}

fn try_expand(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    if !attr.is_empty() {
        return Err(syn::Error::new_spanned(
            attr,
            "`#[widget]` takes no arguments",
        ));
    }
    let imp: ItemImpl = syn::parse2(item)?;
    check_impl(&imp)?;
    check_cfg_attrs(&imp)?;
    let widget: &Type = &imp.self_ty;
    // Whatever compiles the impl or a hook out must take its exports with it.
    let impl_cfgs = cfgs(&imp.attrs);
    let written = written_hooks(&imp);
    let held = held_hooks(&written)?;
    let hooks = written.into_iter().map(|(hook, method)| {
        let export = format_ident!("{hook}");
        let method_cfgs = cfgs(&method.attrs);
        quote! {
            #(#method_cfgs)*
            #[unsafe(no_mangle)]
            pub extern "C" fn #export() {
                ::bmc_wasm_sdk::widget::hook::<#widget>(<#widget as ::bmc_wasm_sdk::Widget>::#export)
            }
        }
    });

    Ok(quote! {
        #imp

        #(#impl_cfgs)*
        impl ::bmc_wasm_sdk::widget::Root for #widget {
            fn __root() -> &'static ::std::thread::LocalKey<::bmc_wasm_sdk::widget::RootSlot<Self>> {
                ::std::thread_local!(
                    static ROOT: ::bmc_wasm_sdk::widget::RootSlot<#widget> =
                        ::bmc_wasm_sdk::widget::RootSlot::new();
                );
                &ROOT
            }
        }

        #(#impl_cfgs)*
        #[cfg(target_arch = "wasm32")]
        const _: () = {
            #[unsafe(no_mangle)]
            pub extern "C" fn init() {
                ::bmc_wasm_sdk::widget::install::<#widget>()
            }

            #[unsafe(no_mangle)]
            pub extern "C" fn render(delta_ms: u32) {
                ::bmc_wasm_sdk::widget::hook::<#widget>(|widget| {
                    <#widget as ::bmc_wasm_sdk::Widget>::render(widget, delta_ms)
                })
            }

            #(#hooks)*
            #(#held)*

            #[unsafe(no_mangle)]
            pub extern "C" fn unload() {
                ::bmc_wasm_sdk::widget::uninstall::<#widget>()
            }
        };
    })
}

fn check_impl(imp: &ItemImpl) -> syn::Result<()> {
    let is_widget_impl = imp
        .trait_
        .as_ref()
        .and_then(|(_, path, _)| path.segments.last())
        .is_some_and(|segment| segment.ident == "Widget");
    if !is_widget_impl {
        return Err(syn::Error::new_spanned(
            &imp.self_ty,
            "`#[widget]` goes on the `impl Widget for …` block",
        ));
    }
    if !imp.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &imp.generics,
            "a widget is one concrete type; it cannot be generic",
        ));
    }
    if let Some(item) = imp
        .items
        .iter()
        .find(|item| matches!(item, ImplItem::Macro(_)))
    {
        return Err(syn::Error::new_spanned(
            item,
            "`#[widget]` cannot see the hooks a macro writes, so it would export none of them; \
             write the methods out in the impl",
        ));
    }
    Ok(())
}

/// The optional hooks the impl writes, in the order they are listed, each with its method.
///
/// A hook written twice under exclusive `#[cfg]`s appears twice, so whichever compiles is exported.
fn written_hooks(imp: &ItemImpl) -> Vec<(&'static str, &ImplItemFn)> {
    OPTIONAL_HOOKS
        .into_iter()
        .flat_map(|hook| {
            imp.items.iter().filter_map(move |item| {
                if let ImplItem::Fn(method) = item
                    && method.sig.ident == hook
                {
                    Some((hook, method))
                } else {
                    None
                }
            })
        })
        .collect()
}

/// Hold the name of each hook wherever the impl's own export of it compiles out,
/// so a hand-written export of the hook fails to build on the duplicate symbol.
/// The name is held by data, and the host takes a hook only from a function.
fn held_hooks(written: &[(&'static str, &ImplItemFn)]) -> syn::Result<Vec<TokenStream>> {
    let mut held = Vec::new();
    for hook in OPTIONAL_HOOKS {
        let exported_under = written
            .iter()
            .filter(|(written, _)| *written == hook)
            .map(|(_, method)| {
                cfgs(&method.attrs)
                    .into_iter()
                    .map(|attr| Ok(attr.meta.require_list()?.tokens.clone()))
                    .collect::<syn::Result<Vec<_>>>()
            })
            .collect::<syn::Result<Vec<_>>>()?;
        if exported_under.iter().any(Vec::is_empty) {
            continue;
        }
        let exported_under = exported_under
            .iter()
            .map(|predicates| quote!(all(#(#predicates),*)));
        let holder = format_ident!("{}", hook.to_uppercase());
        held.push(quote! {
            #[cfg(not(any(#(#exported_under),*)))]
            #[unsafe(export_name = #hook)]
            static #holder: () = ();
        });
    }
    Ok(held)
}

/// Refuse a `#[cfg_attr]` that applies a `cfg` to the impl or a written hook:
/// the macro gets it unevaluated, so it could not gate the exports to match.
fn check_cfg_attrs(imp: &ItemImpl) -> syn::Result<()> {
    let hooks = written_hooks(imp);
    let gated = imp
        .attrs
        .iter()
        .chain(hooks.iter().flat_map(|(_, method)| &method.attrs))
        .find(|attr| applies_a_cfg(&attr.meta));
    match gated {
        Some(attr) => Err(syn::Error::new_spanned(
            attr,
            "`#[widget]` cannot gate its exports by a `cfg` inside `cfg_attr`; use a plain `#[cfg]`",
        )),
        None => Ok(()),
    }
}

fn applies_a_cfg(meta: &Meta) -> bool {
    let Meta::List(list) = meta else {
        return false;
    };
    list.path.is_ident("cfg_attr")
        && list
            .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
            .is_ok_and(|metas| {
                metas
                    .iter()
                    .skip(1)
                    .any(|meta| meta.path().is_ident("cfg") || applies_a_cfg(meta))
            })
}

fn cfgs(attrs: &[Attribute]) -> Vec<&Attribute> {
    attrs
        .iter()
        .filter(|attr| attr.path().is_ident("cfg"))
        .collect()
}

#[cfg(test)]
mod tests {
    use syn::parse_quote;

    use super::*;

    #[test]
    fn only_the_hooks_written_are_exported() {
        let imp: ItemImpl = parse_quote! {
            impl Widget for Facts {
                fn init(_: &Building<Self>) -> Self { Facts }
                fn render(&mut self, _delta_ms: u32) {}
                fn on_touch(&mut self) {}
                fn on_system_update(&mut self) {}
                fn helper(&self) {}
            }
        };

        let hooks: Vec<_> = written_hooks(&imp)
            .into_iter()
            .map(|(hook, _)| hook)
            .collect();

        assert_eq!(hooks, ["on_touch", "on_system_update"]);
    }

    fn exported_items(impl_block: TokenStream) -> Vec<syn::Item> {
        let expanded =
            try_expand(TokenStream::new(), impl_block).expect("BUG: the widget impl expands");
        let file: syn::File = syn::parse2(expanded).expect("BUG: the expansion is a list of items");
        let exports = file
            .items
            .into_iter()
            .find_map(|item| match item {
                syn::Item::Const(exports) => Some(exports),
                _ => None,
            })
            .expect("BUG: the exports are one const block");
        let syn::Expr::Block(block) = *exports.expr else {
            panic!("the exports are not a block");
        };
        block
            .block
            .stmts
            .into_iter()
            .filter_map(|stmt| match stmt {
                syn::Stmt::Item(item) => Some(item),
                _ => None,
            })
            .collect()
    }

    fn exports(impl_block: TokenStream) -> Vec<syn::ItemFn> {
        exported_items(impl_block)
            .into_iter()
            .filter_map(|item| match item {
                syn::Item::Fn(export) => Some(export),
                _ => None,
            })
            .collect()
    }

    fn held(impl_block: TokenStream) -> Vec<(String, syn::Attribute)> {
        exported_items(impl_block)
            .into_iter()
            .filter_map(|item| match item {
                syn::Item::Static(holder) => Some(holder),
                _ => None,
            })
            .map(|holder| {
                let mut cfg = None;
                let mut name = None;
                for attr in holder.attrs {
                    if attr.path().is_ident("cfg") {
                        cfg = Some(attr);
                    } else if let Ok(syn::MetaNameValue {
                        value:
                            syn::Expr::Lit(syn::ExprLit {
                                lit: syn::Lit::Str(lit),
                                ..
                            }),
                        ..
                    }) = attr.parse_args()
                    {
                        name = Some(lit.value());
                    }
                }
                (
                    name.expect("BUG: a holder carries the hook name"),
                    cfg.expect("BUG: a holder carries the cfg it holds under"),
                )
            })
            .collect()
    }

    fn on_touch_exports(impl_block: TokenStream) -> Vec<Vec<syn::Attribute>> {
        exports(impl_block)
            .into_iter()
            .filter(|export| export.sig.ident == "on_touch")
            .map(|export| export.attrs)
            .collect()
    }

    #[test]
    fn every_hook_is_exported_under_the_name_the_host_calls() {
        let exports = exports(quote! {
            impl Widget for Facts {
                fn init(_: &Building<Self>) -> Self { Facts }
                fn render(&mut self, _delta_ms: u32) {}
                fn on_wake(&mut self) {}
                fn on_sleep(&mut self) {}
                fn on_touch(&mut self) {}
                fn on_system_update(&mut self) {}
                fn on_params_update(&mut self) {}
                fn on_credentials_update(&mut self) {}
                fn on_network_update(&mut self) {}
            }
        });

        let names: Vec<String> = exports
            .iter()
            .map(|export| export.sig.ident.to_string())
            .collect();
        assert_eq!(
            names,
            [
                "init",
                "render",
                "on_wake",
                "on_sleep",
                "on_touch",
                "on_system_update",
                "on_params_update",
                "on_credentials_update",
                "on_network_update",
                "unload",
            ],
            "a misspelt hook name exports nothing the host calls, and nothing fails to build"
        );
    }

    #[test]
    fn a_cfg_on_a_hook_gates_its_export() {
        let gate: syn::Attribute = parse_quote!(#[cfg(feature = "touch")]);

        let exports = on_touch_exports(quote! {
            impl Widget for Facts {
                fn init(_: &Building<Self>) -> Self { Facts }
                fn render(&mut self, _delta_ms: u32) {}
                #gate
                fn on_touch(&mut self) {}
            }
        });

        assert!(
            exports.len() == 1 && exports[0].contains(&gate),
            "with the feature off, the host would still route touches to the widget: {exports:?}"
        );
    }

    #[test]
    fn a_hook_written_under_exclusive_cfgs_is_exported_under_each() {
        let on: syn::Attribute = parse_quote!(#[cfg(feature = "touch")]);
        let off: syn::Attribute = parse_quote!(#[cfg(not(feature = "touch"))]);

        let exports = on_touch_exports(quote! {
            impl Widget for Facts {
                fn init(_: &Building<Self>) -> Self { Facts }
                fn render(&mut self, _delta_ms: u32) {}
                #on
                fn on_touch(&mut self) {}
                #off
                fn on_touch(&mut self) {}
            }
        });

        assert!(
            exports.len() == 2 && exports[0].contains(&on) && exports[1].contains(&off),
            "either way the feature is set, the widget writes on_touch and must export it: {exports:?}"
        );
    }

    #[test]
    fn a_hook_the_impl_leaves_out_has_its_name_held() {
        let held = held(quote! {
            impl Widget for Facts {
                fn init(_: &Building<Self>) -> Self { Facts }
                fn render(&mut self, _delta_ms: u32) {}
                fn on_wake(&mut self) {}
                fn on_touch(&mut self) {}
            }
        });

        let always: syn::Attribute = parse_quote!(#[cfg(not(any()))]);
        let names: Vec<&str> = held.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "on_sleep",
                "on_system_update",
                "on_params_update",
                "on_credentials_update",
                "on_network_update",
            ],
            "a hook left free can be exported by hand, and the host calls it outside the root"
        );
        assert!(held.iter().all(|(_, cfg)| *cfg == always), "{held:?}");
    }

    #[test]
    fn a_hook_gated_by_a_cfg_is_held_where_the_cfg_is_off() {
        let held = held(quote! {
            impl Widget for Facts {
                fn init(_: &Building<Self>) -> Self { Facts }
                fn render(&mut self, _delta_ms: u32) {}
                #[cfg(feature = "touch")]
                #[cfg(target_os = "unknown")]
                fn on_touch(&mut self) {}
            }
        });

        let off: syn::Attribute =
            parse_quote!(#[cfg(not(any(all(feature = "touch", target_os = "unknown"))))]);
        assert!(
            held.iter()
                .any(|(name, cfg)| name == "on_touch" && *cfg == off),
            "with the feature off the widget exports no on_touch, so a hand-written one would build: {held:?}"
        );
    }

    fn expansion_error(impl_block: TokenStream) -> Option<String> {
        try_expand(TokenStream::new(), impl_block)
            .err()
            .map(|error| error.to_string())
    }

    #[test]
    fn a_cfg_inside_cfg_attr_on_a_hook_is_refused() {
        let error = expansion_error(quote! {
            impl Widget for Facts {
                fn init(_: &Building<Self>) -> Self { Facts }
                fn render(&mut self, _delta_ms: u32) {}
                #[cfg_attr(feature = "no_touch", cfg(any()))]
                fn on_touch(&mut self) {}
            }
        });

        assert!(
            error.is_some_and(|error| error.contains("plain `#[cfg]`")),
            "the method would compile out and leave `on_touch` exported to the trait default"
        );
    }

    #[test]
    fn a_cfg_inside_cfg_attr_on_the_impl_is_refused() {
        let error = expansion_error(quote! {
            #[cfg_attr(feature = "headless", cfg(any()))]
            impl Widget for Facts {
                fn init(_: &Building<Self>) -> Self { Facts }
                fn render(&mut self, _delta_ms: u32) {}
            }
        });

        assert!(
            error.is_some_and(|error| error.contains("plain `#[cfg]`")),
            "the impl would compile out and leave its `Root` impl and exports behind"
        );
    }

    #[test]
    fn a_cfg_nested_in_cfg_attr_is_refused() {
        let error = expansion_error(quote! {
            impl Widget for Facts {
                fn init(_: &Building<Self>) -> Self { Facts }
                fn render(&mut self, _delta_ms: u32) {}
                #[cfg_attr(feature = "a", cfg_attr(feature = "b", cfg(any())))]
                fn on_touch(&mut self) {}
            }
        });

        assert!(
            error.is_some_and(|error| error.contains("plain `#[cfg]`")),
            "a cfg two cfg_attrs deep compiles the method out just the same"
        );
    }

    #[test]
    fn a_macro_in_the_impl_is_refused() {
        let error = expansion_error(quote! {
            impl Widget for Facts {
                fn init(_: &Building<Self>) -> Self { Facts }
                fn render(&mut self, _delta_ms: u32) {}
                touch_hook!();
            }
        });

        assert!(
            error.is_some_and(|error| error.contains("a macro writes")),
            "a hook the macro writes would get no export"
        );
    }

    #[test]
    fn a_cfg_attr_without_a_cfg_is_accepted() {
        let error = expansion_error(quote! {
            impl Widget for Facts {
                fn init(_: &Building<Self>) -> Self { Facts }
                fn render(&mut self, _delta_ms: u32) {}
                #[cfg_attr(feature = "fast", inline)]
                fn on_touch(&mut self) {}
            }
        });

        assert_eq!(error, None, "only a `cfg` makes the exports go out of step");
    }

    fn facts_exports() -> Vec<syn::ItemFn> {
        exports(quote! {
            impl Widget for Facts {
                fn init(_: &Building<Self>) -> Self { Facts }
                fn render(&mut self, _delta_ms: u32) {}
                fn on_wake(&mut self) {}
            }
        })
    }

    #[test]
    fn every_export_reaches_the_widget() {
        let bodies: Vec<(String, syn::Block)> = facts_exports()
            .into_iter()
            .map(|export| (export.sig.ident.to_string(), *export.block))
            .collect();

        let expected: Vec<(String, syn::Block)> = vec![
            (
                "init".to_owned(),
                parse_quote! {{ ::bmc_wasm_sdk::widget::install::<Facts>() }},
            ),
            (
                "render".to_owned(),
                parse_quote! {{
                    ::bmc_wasm_sdk::widget::hook::<Facts>(|widget| {
                        <Facts as ::bmc_wasm_sdk::Widget>::render(widget, delta_ms)
                    })
                }},
            ),
            (
                "on_wake".to_owned(),
                parse_quote! {{
                    ::bmc_wasm_sdk::widget::hook::<Facts>(<Facts as ::bmc_wasm_sdk::Widget>::on_wake)
                }},
            ),
            (
                "unload".to_owned(),
                parse_quote! {{ ::bmc_wasm_sdk::widget::uninstall::<Facts>() }},
            ),
        ];
        assert_eq!(
            bodies, expected,
            "an export calling anything else leaves the widget's own code unrun"
        );
    }

    #[test]
    fn render_takes_the_frame_delta_the_host_passes() {
        let render = facts_exports()
            .into_iter()
            .find(|export| export.sig.ident == "render")
            .expect("BUG: render is always exported");

        let expected: Punctuated<syn::FnArg, Token![,]> = parse_quote!(delta_ms: u32);
        assert_eq!(
            render.sig.inputs, expected,
            "the host resolves render as taking one u32; any other signature fails to load"
        );
    }

    #[test]
    fn a_cfg_on_the_impl_gates_everything_generated() {
        let gate: syn::Attribute = parse_quote!(#[cfg(feature = "clock")]);
        let expanded = try_expand(
            TokenStream::new(),
            quote! {
                #gate
                impl Widget for Facts {
                    fn init(_: &Building<Self>) -> Self { Facts }
                    fn render(&mut self, _delta_ms: u32) {}
                }
            },
        )
        .expect("BUG: a gated widget impl expands");
        let file: syn::File = syn::parse2(expanded).expect("BUG: the expansion is a list of items");

        for item in &file.items {
            let attrs = match item {
                syn::Item::Impl(item) => &item.attrs,
                syn::Item::Const(item) => &item.attrs,
                other => panic!("unexpected item in the expansion: {other:?}"),
            };
            assert!(
                attrs.contains(&gate),
                "with the feature off, this item outlives the impl it needs: {}",
                quote!(#item)
            );
        }
    }

    #[test]
    fn an_impl_of_another_trait_is_refused() {
        let item = quote! { impl Default for Facts { fn default() -> Self { Facts } } };

        assert!(try_expand(TokenStream::new(), item).is_err());
    }

    #[test]
    fn a_generic_widget_is_refused() {
        let item = quote! { impl<T> Widget for Facts<T> { fn init(_: &Building<Self>) -> Self { todo!() } fn render(&mut self, _: u32) {} } };

        assert!(try_expand(TokenStream::new(), item).is_err());
    }

    #[test]
    fn arguments_are_refused() {
        let item = quote! { impl Widget for Facts { fn init(_: &Building<Self>) -> Self { Facts } fn render(&mut self, _: u32) {} } };

        assert!(try_expand(quote!(touch), item).is_err());
    }
}

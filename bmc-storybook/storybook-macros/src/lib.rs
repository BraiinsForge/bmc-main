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

//! Proc-macro crate for the storybook framework.
//!
//! Provides `#[story]` attribute macro that registers story functions via `inventory`.

use heck::ToTitleCase;
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{ItemFn, LitStr, ReturnType, Token, parse::Parse, parse::ParseStream, parse_macro_input};

struct StoryArgs {
    name: Option<String>,
    grid: bool,
    default: bool,
}

impl Parse for StoryArgs {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if input.is_empty() {
            return Ok(Self {
                name: None,
                grid: false,
                default: false,
            });
        }

        // Optional string literal for name
        let name = if input.peek(LitStr) {
            let lit: LitStr = input.parse()?;
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
            Some(lit.value())
        } else {
            None
        };

        // Optional keywords: `grid`, `default` (in any order)
        let mut grid = false;
        let mut default = false;
        while !input.is_empty() {
            let kw: syn::Ident = input.parse()?;
            if kw == "grid" {
                grid = true;
            } else if kw == "default" {
                default = true;
            } else {
                return Err(syn::Error::new(kw.span(), "expected `grid` or `default`"));
            }
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(Self {
            name,
            grid,
            default,
        })
    }
}

/// Returns `true` if the return type is the unit type `()` or absent (implicit unit).
fn is_unit_return(ret: &ReturnType) -> bool {
    match ret {
        ReturnType::Default => true,
        ReturnType::Type(_, ty) => {
            if let syn::Type::Tuple(tuple) = ty.as_ref() {
                tuple.elems.is_empty()
            } else {
                false
            }
        }
    }
}

/// Mark a function as a storybook story.
///
/// Supports two signatures:
///
/// - `fn(&mut StoryCtx) -> Node` — **auto-wrap mode**: the macro generates a wrapper
///   that calls the original function and pushes the returned `Node` as a single
///   full-width, auto-height frame via `ctx.ui.div(...)`. Existing stories work
///   unchanged.
///
/// - `fn(&mut StoryCtx)` — **document mode**: the story function directly calls
///   `ctx.ui.div()`, `ctx.ui.header()`, etc. to build a multi-frame document.
///
/// # Keywords
///
/// - `grid` — prefer rendering all size variants side-by-side
/// - `default` — when this is the only story in its group, the sidebar
///   collapses the group to a flat entry and the header shows only the
///   group title
///
/// # Variants
///
/// ```ignore
/// #[story]
/// fn all_styles(ctx: &mut StoryCtx) -> Node { ... }
///
/// #[story("Custom Name")]
/// fn my_story(ctx: &mut StoryCtx) -> Node { ... }
///
/// #[story]
/// fn document_mode(ctx: &mut StoryCtx) {
///     ctx.ui.header("Title", "subtitle");
///     ctx.ui.div(Large, some_widget());
/// }
/// ```
#[proc_macro_attribute]
pub fn story(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input_fn = parse_macro_input!(item as ItemFn);
    let fn_name = &input_fn.sig.ident;

    let args: StoryArgs = if attr.is_empty() {
        StoryArgs {
            name: None,
            grid: false,
            default: false,
        }
    } else {
        match syn::parse::<StoryArgs>(attr) {
            Ok(a) => a,
            Err(e) => return e.to_compile_error().into(),
        }
    };

    let name: String = args
        .name
        .unwrap_or_else(|| fn_name.to_string().to_title_case());

    let grid = args.grid;
    let default = args.default;

    // Detect return type: `-> Node` (auto-wrap) vs `-> ()` / no return (document mode).
    let returns_node = !is_unit_return(&input_fn.sig.output);

    // Source tab uses prettyplease's native output as-is.  Earlier post-processing
    // (halve indent, fix macro colon spacing) was dropped: the `" : " → ": "`
    // substitution silently corrupted any docstring/string literal containing
    // that substring, and the indent halving relied on prettyplease's specific
    // 4-space output. The trade-off is 4-space indent and `key : value` macro
    // arg spacing in the displayed source — visually slightly off from typical
    // project style but unambiguous and parser-safe.
    let source_code = {
        let file: syn::File = syn::parse_quote! { #input_fn };
        prettyplease::unparse(&file)
    };

    let expanded = if returns_node {
        // Auto-wrap: generate a wrapper that calls the original fn, pushes the
        // returned Node as a single full-width auto-height frame.
        let wrapper_name = format_ident!("__story_wrapper_{}", fn_name);

        quote! {
            #input_fn

            fn #wrapper_name(ctx: &mut ::bmc_storybook_api::knobs::StoryCtx) {
                let node = #fn_name(ctx);
                ctx.ui.div(
                    (
                        ::bmc_storybook_api::FrameSize::Full.width(),
                        ::bmc_storybook_api::DivHeight::Auto,
                    ),
                    node,
                );
            }

            ::bmc_storybook_api::inventory::submit! {
                ::bmc_storybook_api::StoryEntry {
                    render_fn: #wrapper_name,
                    name: #name,
                    module_path: ::core::module_path!(),
                    source: #source_code,
                    grid: #grid,
                    default: #default,
                }
            }
        }
    } else {
        // Document mode: use the function directly.
        quote! {
            #input_fn

            ::bmc_storybook_api::inventory::submit! {
                ::bmc_storybook_api::StoryEntry {
                    render_fn: #fn_name,
                    name: #name,
                    module_path: ::core::module_path!(),
                    source: #source_code,
                    grid: #grid,
                    default: #default,
                }
            }
        }
    };

    expanded.into()
}

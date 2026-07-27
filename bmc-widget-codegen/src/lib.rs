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

//! Read a widget [`Manifest`] and emit a typed-accessor `manifest_params.rs`
//! Rust module. The widget developer commits the emitted file alongside the
//! manifest and pulls it in with `mod manifest_params;` like any other module.
//!
//! ## Why a script + committed output, not a `build.rs`
//!
//! `build.rs` emitting into `OUT_DIR` would be the conventional shape for this kind of
//! per-crate code generation. We deliberately picked the committed-output path instead:
//!
//! * **Editor support.** Widget authors get full completion, hover docs, and "go to
//!   definition" against the generated `manifest_params.rs` because it lives in their
//!   source tree. A `build.rs`-into-`OUT_DIR` file is invisible to most editors until
//!   the user re-runs cargo with the LSP attached, and rust-analyzer's `OUT_DIR`
//!   support is still patchy on widget-style cross-target setups.
//! * **Diffable artifact.** Reviewers see the generated code change when the manifest
//!   changes — the wire surface between manifest types and the typed accessors is
//!   visible in PRs.
//! * **Drift is caught.** The `bmc-widget-codegen` tests include a drift-guard that
//!   regenerates every example widget's `manifest_params.rs` from its current
//!   `manifest.json` and compares against the committed file. A stale commit fails
//!   CI; the failure message points at `just wasm::gen <widget>` to regenerate.
//!
//! The trade-off is that adding a param to a widget is a two-step change (edit
//! `manifest.json`, run `just wasm::gen`). We accept this for the IDE ergonomics.
//!
//! ## Shape of the output
//!
//! Each declared param becomes a field on a `Params` struct, typed against
//! the manifest variant:
//! - `String` / `Timezone` → `String` (required) or `Option<String>` (optional)
//! - `Integer` → `i32` / `Option<i32>`
//! - `Double` → `f64` / `Option<f64>`
//! - `Boolean` → `bool` / `Option<bool>`
//! - `enum_values` of any of the above → a generated `enum <FieldPascalCase>`
//!   with `ALL`, `as_manifest_value`, `from_manifest_value`
//!
//! The reads route through [`bmc_wasm_sdk::params::typed::ParamRead`]; enums
//! use the `impl_manifest_{str,i32,f64}_enum!` macros from the SDK so each
//! emitted file stays small.
//!
//! Declared credential slots become a `credentials` module, one nested module per slot, holding a
//! `&'static str` const per field of the slot's type. Each const carries the
//! `{{ credential.<slot>.<field> }}` placeholder the host substitutes at egress — never a secret.
//!
//! ## Determinism
//!
//! The same manifest produces byte-equal output across runs:
//! - fields and `from_snapshot` reads are emitted in `ParamKey::as_str()` order;
//! - enum variants are emitted in manifest-declaration order;
//! - the output goes through `prettyplease` (canonical formatting, no external
//!   `rustfmt` subprocess), then through the project formatter at write time
//!   if invoked through `wasm::gen`.
//!
//! ## Identifier mapping
//!
//! Field names: `ParamKey` (regex `[A-Za-z][A-Za-z0-9_-]*`) → snake_case via
//! [`heck::AsSnakeCase`]. Rust keyword clashes use raw identifiers (`r#type`).
//!
//! Enum variants: string-valued enums derive from each option's `value`
//! (PascalCased via [`heck::AsUpperCamelCase`]); int/double enums use the
//! `label` since their `value` is a number. Identical generated variants in
//! the same enum are a hard error.

use anyhow::{Context as _, Result, anyhow, bail};
use bmc_widget_manifest::{
    CredentialKey, CredentialSlot, Manifest, ParamDefinition, ParamKey, ParamKind, credential,
};
use heck::{AsShoutySnakeCase, AsSnakeCase, AsUpperCamelCase};
use indoc::formatdoc;
use proc_macro2::{Ident, Literal, Span, TokenStream};
use quote::{format_ident, quote};
use std::collections::HashSet;

pub const TOOL_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Generate the formatted `manifest_params.rs` source for `manifest`.
///
/// `manifest_relpath` is recorded in the header comment so a reader of the
/// generated file can find the source manifest. Pass it relative to where
/// the generated file lives (e.g. `../manifest.json`).
///
/// Returns `Err` if the manifest declares neither params nor credentials (the
/// caller should not emit a file in that case) or if name-mapping produces a
/// collision.
pub fn generate(manifest: &Manifest, manifest_relpath: &str) -> Result<String> {
    let mut params: Vec<(&ParamKey, &ParamDefinition)> = manifest.params.iter().collect();
    params.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));

    if params.is_empty() && manifest.credentials.is_empty() {
        bail!("manifest has no params or credentials; do not emit a file");
    }

    let params_block = if params.is_empty() {
        TokenStream::new()
    } else {
        emit_params_block(&params)?
    };
    let credentials_block = emit_credentials_block(manifest)?;

    let body = quote! {
        #params_block
        #credentials_block
    };

    let file: syn::File =
        syn::parse2(body).context("BUG: emitted token stream is not a valid Rust file")?;
    let pretty = prettyplease::unparse(&file);

    // `allow`, not `expect`, as we cannot know what the widgets
    // will use or not. It is fine for a widget to not use all functions
    // from this file, such as not using the previous parameters.
    Ok(formatdoc! {r#"
        // AUTO-GENERATED FROM {manifest_relpath} by `bmc-widget-codegen` v{TOOL_VERSION}.
        // Do not edit by hand. Run `just wasm::gen <widget>` after changing the manifest.

        #![allow(
            dead_code,
            reason = "fields are widget-specific; not every key is used by every render path"
        )]

        {pretty}"#
    })
}

fn emit_params_block(params: &[(&ParamKey, &ParamDefinition)]) -> Result<TokenStream> {
    // Resolve identifiers once so the same name is used consistently across the struct
    // declaration, the helper-enum declarations, and the `from_snapshot` body.
    let resolved: Vec<Resolved> = params
        .iter()
        .map(|(k, d)| Resolved::new(k.as_str(), d))
        .collect::<Result<Vec<_>>>()?;

    let helper_enums: Vec<TokenStream> = resolved
        .iter()
        .filter_map(|r| r.enum_decl.as_ref().map(emit_enum_decl))
        .collect();

    let struct_fields = resolved.iter().map(|r| {
        let field = &r.field_ident;
        let ty = &r.field_ty;
        quote! { pub #field: #ty, }
    });

    let from_snapshot_fields = resolved.iter().map(|r| {
        let field = &r.field_ident;
        let ty = &r.field_ty;
        let key_lit = Literal::string(&r.key);
        if r.is_optional {
            // `Option<T>` field; pass the inner `T` as the type parameter to the
            // explicit-trait UFCS so method resolution is unambiguous regardless
            // of whatever else is in the widget's scope.
            let inner = optional_inner_ty(&r.field_ty);
            quote! { #field: <#inner as ParamRead>::read_optional(snap, #key_lit), }
        } else {
            quote! { #field: <#ty as ParamRead>::read_required(snap, #key_lit), }
        }
    });

    let changed_arms = resolved.iter().map(|r| {
        let field = &r.field_ident;
        let key_lit = Literal::string(&r.key);
        quote! { if self.#field != other.#field { out.push(#key_lit); } }
    });

    Ok(quote! {
        use bmc_wasm_sdk::params as snapshot;
        use bmc_wasm_sdk::params::typed::ParamRead;

        #(#helper_enums)*

        #[derive(Clone, Debug, PartialEq)]
        pub struct Params {
            #(#struct_fields)*
        }

        impl Params {
            /// Materialise a typed snapshot from a dynamic [`snapshot::Params`].
            #[must_use]
            pub fn from_snapshot(snap: &snapshot::Params) -> Self {
                Self {
                    #(#from_snapshot_fields)*
                }
            }

            /// Latest typed snapshot delivered for this widget instance.
            /// Cached per-thread; only re-parses when `snapshot::version()` changes
            /// since the last call.
            #[must_use]
            pub fn current() -> Self {
                thread_local! {
                    static CACHE: core::cell::RefCell<Option<(u64, Params)>> =
                        const { core::cell::RefCell::new(None) };
                }
                let v = snapshot::version();
                CACHE.with(|cell| {
                    let mut cache = cell.borrow_mut();
                    if let Some((cv, ref params)) = *cache
                        && cv == v
                    {
                        return params.clone();
                    }
                    let fresh = Self::from_snapshot(&snapshot::current());
                    *cache = Some((v, fresh.clone()));
                    fresh
                })
            }

            /// Snapshot delivered immediately before [`current`]; `None` until at
            /// least one update has been observed (i.e. during `init` and the
            /// first `render`).
            #[must_use]
            pub fn previous() -> Option<Self> {
                let prev = snapshot::previous();
                if prev.is_empty() {
                    None
                } else {
                    Some(Self::from_snapshot(&prev))
                }
            }

            /// Manifest keys whose value differs between `self` and `other`.
            ///
            /// Intended for `on_params_update` diffing — pass `current()` and the
            /// inside-hook value of `previous()` to get the set of keys to react
            /// to. Field-by-field `PartialEq`; emitted in struct-field order so
            /// the result is deterministic.
            #[must_use]
            pub fn changed_keys(&self, other: &Self) -> Vec<&'static str> {
                let mut out = Vec::new();
                #(#changed_arms)*
                out
            }
        }
    })
}

fn emit_credentials_block(manifest: &Manifest) -> Result<TokenStream> {
    if manifest.credentials.is_empty() {
        return Ok(TokenStream::new());
    }

    let catalog = credential::builtins();
    let mut slots: Vec<(&CredentialKey, &CredentialSlot)> = manifest.credentials.iter().collect();
    slots.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));

    let modules = slots
        .into_iter()
        .map(|(key, slot)| emit_credential_slot(&catalog, key.as_str(), slot))
        .collect::<Result<Vec<_>>>()?;

    Ok(quote! {
        /// Credential slots this widget declares, one module per slot.
        pub mod credentials {
            #(#modules)*
        }
    })
}

fn emit_credential_slot(
    catalog: &[credential::CredentialType],
    slot_key: &str,
    slot: &CredentialSlot,
) -> Result<TokenStream> {
    let cred_type = catalog
        .iter()
        .find(|t| t.id == slot.type_id)
        .ok_or_else(|| {
            anyhow!(
                "BUG: slot {slot_key:?} names unknown credential type {:?}, which \
                 Manifest::validate must have rejected first",
                slot.type_id
            )
        })?;

    let mut field_keys: Vec<&ParamKey> = cred_type.fields.keys().collect();
    field_keys.sort_by(|a, b| a.as_str().cmp(b.as_str()));

    let consts = field_keys.into_iter().map(|field| {
        let name = format_ident!("{}", AsShoutySnakeCase(field.as_str()).to_string());
        let placeholder = Literal::string(&format!(
            "{{{{ credential.{slot_key}.{} }}}}",
            field.as_str()
        ));
        let doc = format!("Placeholder for this slot's `{}` field.", field.as_str());
        quote! {
            #[doc = #doc]
            pub const #name: &str = #placeholder;
        }
    });

    let requiredness = if slot.required {
        "Required — the widget cannot work until an account is bound."
    } else {
        "Optional."
    };
    // One attribute per line: a single multi-line `#[doc]` renders as an unindented `/** */` block.
    let mut doc_lines = vec![format!(
        "{} — a `{}` account. {requiredness}",
        slot.label, slot.type_id
    )];
    if let Some(description) = &slot.description {
        doc_lines.push(String::new());
        doc_lines.push(description.clone());
    }
    let docs = doc_lines.iter().map(|line| quote! { #[doc = #line] });
    let module = field_ident(slot_key);

    Ok(quote! {
        #(#docs)*
        pub mod #module {
            #(#consts)*
        }
    })
}

// ── Resolution: manifest key → emitted identifier set ───────────────

struct Resolved {
    key: String,
    field_ident: Ident,
    field_ty: TokenStream,
    is_optional: bool,
    enum_decl: Option<EnumDecl>,
}

enum EnumDecl {
    Str {
        name: Ident,
        variants: Vec<Variant<String>>,
    },
    I32 {
        name: Ident,
        variants: Vec<Variant<i32>>,
    },
    F64 {
        name: Ident,
        variants: Vec<Variant<f64>>,
    },
}

struct Variant<V> {
    ident: Ident,
    value: V,
    label: String,
}

impl Resolved {
    fn new(key: &str, def: &ParamDefinition) -> Result<Self> {
        let field_ident = field_ident(key);
        let (field_ty, enum_decl) = match &def.kind {
            ParamKind::String { enum_values, .. } if !enum_values.is_empty() => {
                let name = enum_name(key);
                let variants = enum_values
                    .iter()
                    .map(|o| {
                        Ok(Variant {
                            ident: variant_ident(&o.value)?,
                            value: o.value.clone(),
                            label: o.label.clone(),
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                assert_unique_variants(key, variants.iter().map(|v| &v.ident))?;
                let ty = quote! { #name };
                (ty, Some(EnumDecl::Str { name, variants }))
            }
            ParamKind::Integer { enum_values, .. } if !enum_values.is_empty() => {
                let name = enum_name(key);
                let variants = enum_values
                    .iter()
                    .map(|o| {
                        Ok(Variant {
                            ident: variant_ident(&o.label)?,
                            value: o.value,
                            label: o.label.clone(),
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                assert_unique_variants(key, variants.iter().map(|v| &v.ident))?;
                let ty = quote! { #name };
                (ty, Some(EnumDecl::I32 { name, variants }))
            }
            ParamKind::Double { enum_values, .. } if !enum_values.is_empty() => {
                let name = enum_name(key);
                let variants = enum_values
                    .iter()
                    .map(|o| {
                        Ok(Variant {
                            ident: variant_ident(&o.label)?,
                            value: o.value,
                            label: o.label.clone(),
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                assert_unique_variants(key, variants.iter().map(|v| &v.ident))?;
                let ty = quote! { #name };
                (ty, Some(EnumDecl::F64 { name, variants }))
            }
            ParamKind::String { .. } | ParamKind::Timezone { .. } => (quote! { String }, None),
            ParamKind::Integer { .. } => (quote! { i32 }, None),
            ParamKind::Double { .. } => (quote! { f64 }, None),
            ParamKind::Boolean { .. } => (quote! { bool }, None),
        };

        let final_ty = if def.is_optional {
            quote! { Option<#field_ty> }
        } else {
            field_ty
        };

        Ok(Self {
            key: key.to_owned(),
            field_ident,
            field_ty: final_ty,
            is_optional: def.is_optional,
            enum_decl,
        })
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "three near-parallel branches (str/i32/f64); splitting hurts side-by-side review"
)]
fn emit_enum_decl(decl: &EnumDecl) -> TokenStream {
    match decl {
        EnumDecl::Str { name, variants } => {
            let variant_idents = variants.iter().map(|v| &v.ident);
            let all_self = variants.iter().map(|v| {
                let i = &v.ident;
                quote! { Self::#i }
            });
            let to_arms = variants.iter().map(|v| {
                let i = &v.ident;
                let lit = Literal::string(&v.value);
                quote! { Self::#i => #lit }
            });
            let from_arms = variants.iter().map(|v| {
                let i = &v.ident;
                let lit = Literal::string(&v.value);
                quote! { #lit => Some(Self::#i) }
            });
            let label_arms = variants.iter().map(|v| {
                let i = &v.ident;
                let lit = Literal::string(&v.label);
                quote! { Self::#i => #lit }
            });
            quote! {
                #[derive(Clone, Copy, Debug, PartialEq, Eq)]
                pub enum #name {
                    #(#variant_idents,)*
                }

                impl #name {
                    /// Every variant, in manifest-declaration order. Useful when a widget
                    /// wants to render a "pick one" UI or audit the enum exhaustively.
                    pub const ALL: &'static [Self] = &[#(#all_self),*];

                    /// Manifest wire value for this variant.
                    #[must_use]
                    pub fn as_manifest_value(self) -> &'static str {
                        match self { #(#to_arms,)* }
                    }

                    /// Human-readable label declared in the manifest's `enum_values`.
                    #[must_use]
                    pub fn as_manifest_label(self) -> &'static str {
                        match self { #(#label_arms,)* }
                    }

                    #[must_use]
                    pub fn from_manifest_value(s: &str) -> Option<Self> {
                        match s {
                            #(#from_arms,)*
                            _ => None,
                        }
                    }
                }

                bmc_wasm_sdk::impl_manifest_str_enum!(#name);
            }
        }
        EnumDecl::I32 { name, variants } => {
            let variant_idents = variants.iter().map(|v| &v.ident);
            let all_self = variants.iter().map(|v| {
                let i = &v.ident;
                quote! { Self::#i }
            });
            let to_arms = variants.iter().map(|v| {
                let i = &v.ident;
                let lit = Literal::i32_unsuffixed(v.value);
                quote! { Self::#i => #lit }
            });
            let from_arms = variants.iter().map(|v| {
                let i = &v.ident;
                let lit = Literal::i32_unsuffixed(v.value);
                quote! { #lit => Some(Self::#i) }
            });
            let label_arms = variants.iter().map(|v| {
                let i = &v.ident;
                let lit = Literal::string(&v.label);
                quote! { Self::#i => #lit }
            });
            quote! {
                #[derive(Clone, Copy, Debug, PartialEq, Eq)]
                pub enum #name {
                    #(#variant_idents,)*
                }

                impl #name {
                    /// Every variant, in manifest-declaration order. Useful when a widget
                    /// wants to render a "pick one" UI or audit the enum exhaustively.
                    pub const ALL: &'static [Self] = &[#(#all_self),*];

                    /// Manifest wire value for this variant.
                    #[must_use]
                    pub fn as_manifest_value(self) -> i32 {
                        match self { #(#to_arms,)* }
                    }

                    /// Human-readable label declared in the manifest's `enum_values`.
                    #[must_use]
                    pub fn as_manifest_label(self) -> &'static str {
                        match self { #(#label_arms,)* }
                    }

                    #[must_use]
                    pub fn from_manifest_value(v: i32) -> Option<Self> {
                        match v {
                            #(#from_arms,)*
                            _ => None,
                        }
                    }
                }

                bmc_wasm_sdk::impl_manifest_i32_enum!(#name);
            }
        }
        EnumDecl::F64 { name, variants } => {
            let variant_idents = variants.iter().map(|v| &v.ident);
            let all_self = variants.iter().map(|v| {
                let i = &v.ident;
                quote! { Self::#i }
            });
            let to_arms = variants.iter().map(|v| {
                let i = &v.ident;
                let lit = f64_literal(v.value);
                quote! { Self::#i => #lit }
            });
            // `f64` doesn't implement `Eq`, so we can't `match` on it — chained
            // if-let with epsilon comparison is the standard workaround.
            let from_arms = variants.iter().map(|v| {
                let i = &v.ident;
                let lit = f64_literal(v.value);
                quote! { if (v - #lit).abs() < f64::EPSILON { return Some(Self::#i); } }
            });
            let label_arms = variants.iter().map(|v| {
                let i = &v.ident;
                let lit = Literal::string(&v.label);
                quote! { Self::#i => #lit }
            });
            quote! {
                #[derive(Clone, Copy, Debug, PartialEq)]
                pub enum #name {
                    #(#variant_idents,)*
                }

                impl #name {
                    /// Every variant, in manifest-declaration order. Useful when a widget
                    /// wants to render a "pick one" UI or audit the enum exhaustively.
                    pub const ALL: &'static [Self] = &[#(#all_self),*];

                    /// Manifest wire value for this variant.
                    #[must_use]
                    pub fn as_manifest_value(self) -> f64 {
                        match self { #(#to_arms,)* }
                    }

                    /// Human-readable label declared in the manifest's `enum_values`.
                    #[must_use]
                    pub fn as_manifest_label(self) -> &'static str {
                        match self { #(#label_arms,)* }
                    }

                    #[must_use]
                    pub fn from_manifest_value(v: f64) -> Option<Self> {
                        #(#from_arms)*
                        None
                    }
                }

                bmc_wasm_sdk::impl_manifest_f64_enum!(#name);
            }
        }
    }
}

// ── Identifier helpers ──────────────────────────────────────────────

fn field_ident(key: &str) -> Ident {
    let snake = AsSnakeCase(key).to_string();
    if is_rust_keyword(&snake) {
        Ident::new_raw(&snake, Span::call_site())
    } else {
        format_ident!("{snake}")
    }
}

fn enum_name(key: &str) -> Ident {
    let pascal = AsUpperCamelCase(key).to_string();
    format_ident!("{pascal}")
}

fn variant_ident(s: &str) -> Result<Ident> {
    let mut pascal = AsUpperCamelCase(s).to_string();
    if pascal.is_empty() {
        return Err(anyhow!(
            "cannot derive a Rust identifier from {s:?} — no alphanumeric characters"
        ));
    }
    // `heck` doesn't handle leading-digit cases — prefix with `_` so the result
    // is a valid Rust identifier (e.g. `"24h"` → `_24H`, stays unique among
    // siblings even if another variant also started with a digit).
    if pascal.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        pascal.insert(0, '_');
    }
    Ok(format_ident!("{pascal}"))
}

fn assert_unique_variants<'a, I: Iterator<Item = &'a Ident>>(
    field_key: &str,
    variants: I,
) -> Result<()> {
    let mut seen: HashSet<String> = HashSet::new();
    for v in variants {
        let s = v.to_string();
        if !seen.insert(s.clone()) {
            return Err(anyhow!(
                "field {field_key:?}: enum_values produce a duplicate Rust variant identifier \
                 {s:?} — distinct manifest options yielded the same PascalCase name"
            ));
        }
    }
    Ok(())
}

fn is_rust_keyword(s: &str) -> bool {
    // 2024-edition reserved + strict keywords. Wrapping in raw identifiers is
    // harmless on weak keywords (`union`, `dyn`), so the set is generous.
    matches!(
        s,
        "as" | "break"
            | "const"
            | "continue"
            | "crate"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "async"
            | "await"
            | "dyn"
            | "abstract"
            | "become"
            | "box"
            | "do"
            | "final"
            | "macro"
            | "override"
            | "priv"
            | "typeof"
            | "unsized"
            | "virtual"
            | "yield"
            | "try"
            | "union"
    )
}

/// Emit an `f64` literal with an explicit decimal point so the parser never
/// reads it as an integer in context. Avoids `1` being elided in match arms
/// where the f64 literal needs `1.0`.
fn f64_literal(v: f64) -> Literal {
    // Round-trip via a string so whole numbers emit as `1.0` rather than `1`
    // — `Literal::f64_unsuffixed(1.0)` produces a bare `1` token that parses
    // as an integer in match arm context.
    let s = if v.fract() == 0.0 && v.is_finite() {
        format!("{v:.1}")
    } else {
        format!("{v}")
    };
    let parsed: f64 = s
        .parse()
        .expect("BUG: f64::format then parse must round-trip");
    Literal::f64_unsuffixed(parsed)
}

/// Strip the outer `Option<T>` from a token stream we built ourselves to recover
/// the inner type for `read_optional` dispatch. Re-parses since we don't track
/// the inner type alongside the wrapped form.
fn optional_inner_ty(opt: &TokenStream) -> TokenStream {
    // `opt` is always `Option<T>` for our optional fields by construction
    // (`Resolved::new` is the only producer), so parsing as `syn::Type` then
    // pattern-matching is exhaustive in practice.
    let ty: syn::Type = syn::parse2(opt.clone())
        .expect("BUG: optional field type was not produced by Resolved::new");
    let syn::Type::Path(p) = &ty else {
        panic!("BUG: optional field type was not a path type")
    };
    let seg = p
        .path
        .segments
        .last()
        .expect("BUG: optional field type path was empty");
    let syn::PathArguments::AngleBracketed(args) = &seg.arguments else {
        panic!("BUG: optional field type was not `Option<T>`")
    };
    let arg = args
        .args
        .first()
        .expect("BUG: optional field type had no `<T>`");
    quote! { #arg }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snake_case_field_ident() {
        assert_eq!(
            field_ident("refresh-seconds").to_string(),
            "refresh_seconds"
        );
        assert_eq!(field_ident("RefreshSeconds").to_string(), "refresh_seconds");
        assert_eq!(field_ident("free_string").to_string(), "free_string");
    }

    #[test]
    fn keyword_clashes_use_raw_idents() {
        // `Ident::new_raw("type", ...).to_string()` includes the raw prefix
        // (unlike a regular `Ident`), so the round-trip is `r#type`.
        let id = field_ident("type");
        assert_eq!(id.to_string(), "r#type");
        let tokens = quote! { #id };
        assert!(tokens.to_string().contains("r#type"));
    }

    #[test]
    fn pascal_enum_name() {
        assert_eq!(enum_name("theme").to_string(), "Theme");
        assert_eq!(enum_name("string_enum").to_string(), "StringEnum");
        assert_eq!(enum_name("night-mode").to_string(), "NightMode");
    }

    #[test]
    fn variant_handles_digit_prefix() {
        // `heck::AsUpperCamelCase` keeps the lowercase `h` because there's no
        // word-boundary signal in `"24h"`. We just prefix to make it parse.
        assert_eq!(
            variant_ident("24h")
                .expect("BUG: `24h` PascalCases to a valid ident with the digit-prefix rule")
                .to_string(),
            "_24h",
        );
    }

    #[test]
    fn variant_errors_on_empty_input() {
        assert!(variant_ident("").is_err());
        assert!(variant_ident("...").is_err());
    }

    #[test]
    fn assert_unique_variants_flags_collisions() {
        let v = [
            format_ident!("Foo"),
            format_ident!("Bar"),
            format_ident!("Foo"),
        ];
        assert!(assert_unique_variants("k", v.iter()).is_err());
    }

    /// End-to-end sanity check: parse a small manifest, run codegen, verify the
    /// emitted file is valid Rust and carries the expected top-level items.
    #[test]
    fn emitted_module_parses() {
        let json = r#"{
            "uid": "00000000-0000-4000-8000-000000000000",
            "version": "0.1.0",
            "name": "T",
            "description": "T",
            "binary": "t.wasm",
            "supported_viewports": [{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238}],
            "params": {
                "theme": {
                    "name": "Theme",
                    "type": "string",
                    "enum_values": [
                        {"value": "light", "label": "Light"},
                        {"value": "dark",  "label": "Dark"}
                    ],
                    "default_value": "light"
                },
                "ratio": {
                    "name": "Ratio",
                    "type": "double",
                    "optional": true
                }
            }
        }"#;
        let manifest = <Manifest as std::str::FromStr>::from_str(json)
            .expect("BUG: hand-crafted test manifest must parse");
        let src = generate(&manifest, "test://")
            .expect("BUG: test manifest has non-empty params, codegen must produce a file");
        let parsed: syn::File =
            syn::parse_str(&src).expect("BUG: codegen output must be syntactically valid Rust");
        let items: Vec<String> = parsed
            .items
            .iter()
            .filter_map(|it| {
                if let syn::Item::Struct(s) = it {
                    return Some(s.ident.to_string());
                }
                if let syn::Item::Enum(e) = it {
                    return Some(e.ident.to_string());
                }
                None
            })
            .collect();
        assert!(items.contains(&"Params".to_owned()), "items: {items:?}");
        assert!(items.contains(&"Theme".to_owned()), "items: {items:?}");
        // `ratio` is optional, no enum → no helper enum emitted.
        assert!(!items.contains(&"Ratio".to_owned()));
    }

    fn manifest_with_credentials(credentials: &str, params: &str) -> Manifest {
        let json = format!(
            r#"{{
            "uid": "00000000-0000-4000-8000-000000000000",
            "version": "0.1.0",
            "name": "T",
            "description": "T",
            "binary": "t.wasm",
            "supported_viewports": [{{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238}}],
            "params": {params},
            "credentials": {credentials}
        }}"#
        );
        <Manifest as std::str::FromStr>::from_str(&json)
            .expect("BUG: hand-crafted test manifest must parse")
    }

    #[test]
    fn credential_slots_emit_placeholder_consts_for_every_field_of_their_type() {
        let manifest = manifest_with_credentials(
            r#"{"media": {"type": "generic-userpass", "label": "Media server"}}"#,
            "{}",
        );
        let src = generate(&manifest, "test://").expect("BUG: credentials alone must emit a file");
        syn::parse_str::<syn::File>(&src).expect("BUG: codegen output must be valid Rust");

        assert!(src.contains("pub mod credentials"), "{src}");
        assert!(src.contains("pub mod media"), "{src}");
        assert!(
            src.contains(r#"pub const USERNAME: &str = "{{ credential.media.username }}""#),
            "{src}"
        );
        assert!(
            src.contains(r#"pub const PASSWORD: &str = "{{ credential.media.password }}""#),
            "{src}"
        );
    }

    #[test]
    fn a_credentials_only_manifest_emits_no_params_scaffolding() {
        let manifest = manifest_with_credentials(
            r#"{"pool": {"type": "braiins-pool", "label": "Pool"}}"#,
            "{}",
        );
        let src = generate(&manifest, "test://").expect("BUG: credentials alone must emit a file");

        assert!(src.contains("pub mod credentials"), "{src}");
        assert!(
            !src.contains("pub struct Params"),
            "no params declared, so no Params struct: {src}"
        );
        assert!(
            !src.contains("bmc_wasm_sdk"),
            "the SDK import would be unused without params: {src}"
        );
    }

    #[test]
    fn a_slot_key_becomes_a_snake_case_module_while_the_placeholder_keeps_the_manifest_key() {
        let manifest = manifest_with_credentials(
            r#"{"main-pool": {"type": "braiins-pool", "label": "Pool"}}"#,
            "{}",
        );
        let src = generate(&manifest, "test://").expect("BUG: credentials alone must emit a file");
        syn::parse_str::<syn::File>(&src).expect("BUG: codegen output must be valid Rust");

        assert!(src.contains("pub mod main_pool"), "{src}");
        assert!(
            src.contains(r#""{{ credential.main-pool.token }}""#),
            "the placeholder is resolved against the manifest key, not the Rust ident: {src}"
        );
    }

    #[test]
    fn a_manifest_with_neither_params_nor_credentials_emits_nothing() {
        let manifest = manifest_with_credentials("{}", "{}");
        assert!(generate(&manifest, "test://").is_err());
    }
}

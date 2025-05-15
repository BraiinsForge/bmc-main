// Copyright (C) 2024  Braiins Systems s.r.o.

use proc_macro::TokenStream;
use quote::quote;
use syn::{
    Expr, ExprLit, Field, Fields, ItemStruct, Lit, Meta, punctuated::Punctuated, spanned::Spanned,
    token::Comma,
};

const ASSET_ATTR_IDENT: &str = "asset";
const ASSET_TITLE_ATTR_IDENT: &str = "title";

pub fn titled_assets_derive_impl(input: &ItemStruct) -> TokenStream {
    // Get the identifier of the struct
    let struct_ident = &input.ident;

    let Fields::Named(fields) = &input.fields else {
        panic!("Only named fields are supported")
    };

    let field_type = if let Some(field) = fields.named.iter().next() {
        &field.ty
    } else {
        panic!("Struct must have at least one field")
    };

    let fields = fields
        .named
        .iter()
        .map(|field| {
            let ident = &field.ident;
            let title = extract_title_asset(field).unwrap_or_else(|err| {
                panic!(
                    "Failed to extract title for field `{}`: {}",
                    ident.as_ref().unwrap(),
                    err
                )
            });

            quote! { (#title, &self.#ident ) }
        })
        .collect::<Vec<_>>();

    // Generate the iterator implementation
    let expanded = quote! {
        impl #struct_ident {
            pub fn titled_assets(&self) -> std::vec::Vec<(&str, &#field_type)> {
                vec![
                    #(#fields,)*
                ]
            }
        }
    };

    // Convert the generated code into a token stream and return it
    TokenStream::from(expanded)
}

/// Extracts the asset title from the `#[asset(title = "...")]` attribute.
fn extract_title_asset(field: &Field) -> syn::Result<String> {
    for attr in &field.attrs {
        let Meta::List(ref list) = attr.meta else {
            continue;
        };

        if !list.path.is_ident(ASSET_ATTR_IDENT) {
            continue;
        }

        let nested = list.parse_args_with(Punctuated::<Meta, Comma>::parse_terminated)?;

        if let Some(meta) = nested.iter().next() {
            let Meta::NameValue(nv) = meta else {
                return Err(syn::Error::new(meta.span(), "expected name-value pair"));
            };

            if !nv.path.is_ident(ASSET_TITLE_ATTR_IDENT) {
                return Err(syn::Error::new(
                    nv.path.span(),
                    format!("expected `{ASSET_TITLE_ATTR_IDENT}`"),
                ));
            }

            let Expr::Lit(ExprLit {
                lit: Lit::Str(ref cmd_name_lit),
                ..
            }) = nv.value
            else {
                return Err(syn::Error::new(nv.value.span(), "expected string literal"));
            };

            return Ok(cmd_name_lit.value());
        }
    }

    Err(syn::Error::new(
        field.span(),
        format!("missing `{ASSET_ATTR_IDENT}` attribute"),
    ))
}

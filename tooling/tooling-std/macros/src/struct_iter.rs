// Copyright (C) 2024  Braiins Systems s.r.o.

use proc_macro::TokenStream;
use quote::quote;
use std::str::FromStr;
use syn::{Fields, ItemStruct};

pub fn struct_iter_derive_impl(input: &ItemStruct) -> TokenStream {
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

    for field in &fields.named {
        assert!(
            !(field.ty != *field_type),
            "All fields must have the same type"
        );
    }

    let iterator_ident =
        proc_macro2::TokenStream::from_str(&format!("{struct_ident}Iterator")).unwrap();
    let field_matchers = fields
        .named
        .iter()
        .enumerate()
        .map(|(i, field)| {
            let field_name = field.ident.clone().unwrap();
            quote! { #i => &self.inner.#field_name }
        })
        .collect::<Vec<_>>();

    let fields_mut_refs = fields
        .named
        .iter()
        .map(|field| {
            let field_name = field.ident.clone().unwrap();
            quote!( &mut self.#field_name )
        })
        .collect::<Vec<_>>();

    // Generate the iterator implementation
    let expanded = quote! {
        impl #struct_ident {
            pub fn iter(&self) -> #iterator_ident {
                #iterator_ident {
                    inner: self,
                    index: 0,
                }
            }

            pub fn iter_mut(&mut self) -> std::vec::IntoIter<&mut #field_type> {
                vec![
                    #( #fields_mut_refs, )*
                ].into_iter()
            }
        }

        pub struct #iterator_ident<'a> {
            inner: &'a #struct_ident,
            index: usize,
        }

        impl<'a> Iterator for #iterator_ident<'a> {
            type Item = &'a #field_type;

            fn next(&mut self) -> Option<Self::Item> {
                let value = match self.index {
                    #( #field_matchers, )*
                    _ => return None,
                };
                self.index += 1;
                Some(value)
            }
        }
    };

    // Convert the generated code into a token stream and return it
    TokenStream::from(expanded)
}

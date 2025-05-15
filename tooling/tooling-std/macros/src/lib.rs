// Copyright (C) 2024  Braiins Systems s.r.o.

mod struct_iter;
mod titled_assets;

use proc_macro::TokenStream;
use syn::{ItemStruct, parse_macro_input};

#[proc_macro_derive(StructIter)]
pub fn struct_iter_derive(input: TokenStream) -> TokenStream {
    // Parse the input tokens into a syntax tree
    let input = parse_macro_input!(input as ItemStruct);
    struct_iter::struct_iter_derive_impl(&input)
}

#[proc_macro_derive(TitledAssets, attributes(asset))]
pub fn titled_assets_iter_derive(input: TokenStream) -> TokenStream {
    // Parse the input tokens into a syntax tree
    let input = parse_macro_input!(input as ItemStruct);
    titled_assets::titled_assets_derive_impl(&input)
}

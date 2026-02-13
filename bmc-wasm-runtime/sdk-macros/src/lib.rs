// Copyright (C) 2026  Braiins Systems s.r.o.

//! Proc macros for the WASM widget SDK.
//!
//! Provides `include_icon!` which compiles SVG files into compact binary path
//! data at build time using usvg.

use proc_macro::TokenStream;
use quote::quote;
use syn::{LitStr, parse_macro_input};

/// Embed a PNG (or other raster image) file as a `Bitmap` at compile time.
///
/// The raw file bytes are included directly; the host decodes on first registration.
/// Cargo tracks the file for recompilation when it changes.
///
/// # Usage
///
/// ```ignore
/// const FALCON_9: Bitmap = include_bitmap!("assets/falcon-9.png");
/// ```
///
/// The path is relative to the crate's `CARGO_MANIFEST_DIR`.
#[proc_macro]
pub fn include_bitmap(input: TokenStream) -> TokenStream {
    let path_lit = parse_macro_input!(input as LitStr);
    let rel_path = path_lit.value();

    // Verify the file exists at compile time for a clear error message
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .unwrap_or_else(|_| panic!("CARGO_MANIFEST_DIR not set"));
    let full_path = std::path::Path::new(&manifest_dir).join(&rel_path);
    if !full_path.exists() {
        panic!("bitmap file not found: {}", full_path.display());
    }

    let expanded = quote! {
        bmc_wasm_sdk::Bitmap {
            data: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/", #rel_path))
        }
    };

    expanded.into()
}

/// Compile an SVG file into compact binary path data at build time.
///
/// The SVG is parsed by usvg which simplifies all elements (rects, circles,
/// transforms, CSS, etc.) into absolute bezier paths. The result is a compact
/// binary format that the host runtime converts into FemtoVG `Path` objects.
///
/// # Usage
///
/// ```ignore
/// const STAR: Icon = include_icon!("assets/star.svg");
/// ```
///
/// The path is relative to the crate's `CARGO_MANIFEST_DIR`. Cargo automatically
/// tracks the SVG file for recompilation when it changes.
#[proc_macro]
pub fn include_icon(input: TokenStream) -> TokenStream {
    let path_lit = parse_macro_input!(input as LitStr);
    let rel_path = path_lit.value();

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .unwrap_or_else(|_| panic!("CARGO_MANIFEST_DIR not set"));
    let full_path = std::path::Path::new(&manifest_dir).join(&rel_path);

    let svg_data = std::fs::read_to_string(&full_path)
        .unwrap_or_else(|e| panic!("failed to read SVG `{}`: {e}", full_path.display()));

    let compiled = bmc_icon_compiler::compile_svg(&svg_data);

    // Emit const-compatible expression.
    // The include_bytes! ensures Cargo recompiles when the SVG file changes.
    let expanded = quote! {
        {
            const _TRACK: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/", #rel_path));
            bmc_wasm_sdk::Icon { data: &[#(#compiled),*] }
        }
    };

    expanded.into()
}

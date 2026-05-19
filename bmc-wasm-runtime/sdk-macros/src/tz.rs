// Copyright (C) 2026  Braiins Systems s.r.o.

//! `tz!("IANA/Name")` proc macro — compile-time-validated timezone literal.
//!
//! Validates the supplied IANA name against
//! `bmc_shared_time::timezone_variants_raw::TIMEZONE_VARIANTS_RAW`,
//! which is the deck's authoritative supported-timezone list sourced
//! from openwrt/LuCI's `zoneinfo.uc`. Unknown names yield a `compile_error!`
//! at the call site.

use bmc_shared_time::timezone_variants_raw::TIMEZONE_VARIANTS_RAW;
use proc_macro2::TokenStream;
use quote::quote;
use syn::{LitStr, parse2};

pub fn expand(input: TokenStream) -> TokenStream {
    let lit: LitStr = match parse2(input) {
        Ok(lit) => lit,
        Err(e) => return e.to_compile_error(),
    };
    let name = lit.value();
    if !TIMEZONE_VARIANTS_RAW.iter().any(|(iana, _)| *iana == name) {
        let msg = format!(
            "tz!: '{name}' is not in the deck's supported timezone list \
             (see bmc-shared-time::timezone_variants_raw, sourced from \
             openwrt/LuCI zoneinfo.uc)",
        );
        return syn::Error::new(lit.span(), msg).to_compile_error();
    }
    quote! { ::bmc_wasm_sdk::Tz::from_static_validated(#lit) }
}

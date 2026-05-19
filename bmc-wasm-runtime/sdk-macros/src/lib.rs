// Copyright (C) 2026  Braiins Systems s.r.o.

//! Proc macros for the WASM widget SDK.
//!
//! Provides `json!` for compile-time JSON templates and `fmt!` backing macro.
//!
//! Asset macros (`include_icon!`, `include_bitmap!`, `include_mesh!`,
//! `include_nine_patch!`, `include_skin!`, `include_audio!`) live in
//! `bmc-render-macros`.

mod fmt_capture;
mod json;
mod tz;

use proc_macro::TokenStream;

/// Compile-time JSON template that emits a `fmt!(...)` call.
///
/// Literal JSON structure is validated at compile time and baked into the
/// format string with `{{`/`}}` escaping already applied.
///
/// - `#(expr)` — raw interpolation (numbers, booleans, pre-built JSON fragments)
/// - `#s(expr)` — string interpolation (value wrapped in JSON quotes)
///
/// # Examples
///
/// ```ignore
/// let body = json!({
///     "jsonrpc": "2.0",
///     "method": #s(method),
///     "params": { "playerid": #(pid) },
///     "id": #(id)
/// });
/// ```
#[proc_macro]
pub fn json(input: TokenStream) -> TokenStream {
    json::expand(input.into()).into()
}

/// Proc macro backing the `fmt!` macro — rewrites captured variable syntax
/// (e.g. `{year}`, `{val:x}`) into positional placeholders for `ufmt::uwrite!`.
///
/// Emits a block that allocates a `String`, calls `ufmt::uwrite!` with the
/// rewritten format string + args, and evaluates to the `String`.
#[proc_macro]
pub fn fmt_impl(input: TokenStream) -> TokenStream {
    fmt_capture::expand(input.into()).into()
}

/// Compile-time-validated IANA timezone literal.
///
/// Validates the supplied name against the deck's supported-timezone
/// list (`bmc_shared_time::timezone_variants_raw::TIMEZONE_VARIANTS_RAW`,
/// sourced from openwrt/LuCI's `zoneinfo.uc`).
///
/// Unknown names yield a compile error at the call site.
///
/// # Examples
///
/// ```ignore
/// let la = tz!("America/Los_Angeles");
/// // tz!("Bogus/Name"); // compile_error!: not in the supported list
/// ```
#[proc_macro]
pub fn tz(input: TokenStream) -> TokenStream {
    tz::expand(input.into()).into()
}

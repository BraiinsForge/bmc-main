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

//! Proc macros for the WASM widget SDK.
//!
//! Provides `json!` for compile-time JSON templates and `fmt!` backing macro.
//!
//! Asset macros (`include_svg!`, `include_bitmap!`, `include_mesh!`,
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

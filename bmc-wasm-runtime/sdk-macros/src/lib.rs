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
//! Provides `json!` for compile-time JSON templates, the `fmt!` backing macro,
//! `tz!` for timezone literals, `#[wasm_test]` for tests that run on both targets,
//! and `#[widget]`, which exports a widget struct's hooks.
//!
//! Asset macros (`include_svg!`, `include_bitmap!`, `include_mesh!`,
//! `include_nine_patch!`, `include_skin!`, `include_audio!`) live in
//! `bmc-render-macros`.

mod fmt_capture;
mod json;
mod tz;
mod wasm_test;
mod widget;

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

/// A test that runs natively under libtest and on wasm32 under the wasm test runner.
///
/// Write `#[should_panic]` and `#[ignore]` after it, as with `#[test]`.
/// The body runs as a host export: a method callback may deliver into
/// a `Part`, and work deferred to the export's end runs once the body returns.
///
/// ```ignore
/// use bmc_wasm_sdk::wasm_test;
///
/// #[wasm_test]
/// #[should_panic(expected = "month must be 1..=12")]
/// fn month_abbr_asserts_on_zero() {
///     month_abbr(0);
/// }
/// ```
///
/// A test heavier than the runner's per-test fuel names its own budget, and says why:
///
/// ```ignore
/// #[wasm_test(fuel = 250e6, reason = "parses 2,048 candles")]
/// fn full_history_parses() { … }
/// ```
#[proc_macro_attribute]
pub fn wasm_test(attr: TokenStream, item: TokenStream) -> TokenStream {
    wasm_test::expand(attr.into(), item.into()).into()
}

/// The host exports for a widget written as a struct.
///
/// Put it on the `impl Widget for …` block. It exports `init`, `render` and
/// `unload`, plus each optional hook the impl writes, and lets the type pass
/// `self` to run its own methods as callbacks (`Timer::start(self, period, Self::tick)`).
/// A hand-written export of a hook it leaves out fails to build for wasm32: it holds the name.
///
/// ```ignore
/// use bmc_wasm_sdk::{Building, Widget, widget};
///
/// struct RandomFact { fact: Option<String> }
///
/// #[widget]
/// impl Widget for RandomFact {
///     fn init(_: &Building<Self>) -> Self { Self { fact: None } }
///     fn render(&mut self, _delta_ms: u32) { /* … */ }
/// }
/// ```
#[proc_macro_attribute]
pub fn widget(attr: TokenStream, item: TokenStream) -> TokenStream {
    widget::expand(attr.into(), item.into()).into()
}

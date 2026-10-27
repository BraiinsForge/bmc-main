# bmc-wasm-sdk-macros

Proc macros for the WASM widget SDK: `json!`, `fmt_impl!`, `tz!`, `#[wasm_test]` and `#[widget]`.

## Purpose

Provides the proc macros used by widget authors:

- `json!` -- compile-time JSON template macro. Validates literal JSON structure at build time and bakes it into a format
  string. Supports `#(expr)` for raw interpolation and `#s(expr)` for string-escaped interpolation.
- `fmt_impl!` -- backing macro for `fmt!`. Rewrites captured variable syntax (e.g. `{year}`, `{val:x}`) into positional
  `ufmt::uwrite!` calls, avoiding `core::fmt` overhead in WASM binaries.
- `tz!` -- compile-time IANA timezone literal, refused unless the deck supports the zone.
- `#[wasm_test]` -- a test that runs natively under libtest and on wasm32 under the wasm test runner, its body inside an
  SDK export as a hook's would be.
- `#[widget]` -- on `impl Widget for T`, exports `init`, `render`, `unload` and the hooks the impl writes, all running
  on the one SDK-owned instance.

Asset macros (`include_icon!`, `include_bitmap!`, `include_mesh!`, `include_nine_patch!`, `include_skin!`,
`include_audio!`) used to live here but have moved to `bmc-render-macros`.

## Boundaries

**IS its responsibility:**

- `json!` compile-time JSON template expansion
- `fmt_impl!` captured-variable format string rewriting
- `tz!` timezone validation against the supported list
- `#[wasm_test]` test expansion for both targets
- `#[widget]` export generation for a widget struct

**IS NOT its responsibility:**

- Asset packaging (that is `bmc-render-macros`)
- Runtime string formatting (that is `ufmt`, invoked by the generated code)
- Wire format types (that is `bmc-wasm-protocol`)

# bmc-wasm-sdk-macros

Proc macros for the WASM widget SDK: `json!` and `fmt_impl!`.

## Purpose

Provides two proc macros used by widget authors:

- `json!` -- compile-time JSON template macro. Validates literal JSON structure at build time and bakes it into a format
  string. Supports `#(expr)` for raw interpolation and `#s(expr)` for string-escaped interpolation.
- `fmt_impl!` -- backing macro for `fmt!`. Rewrites captured variable syntax (e.g. `{year}`, `{val:x}`) into positional
  `ufmt::uwrite!` calls, avoiding `core::fmt` overhead in WASM binaries.

Asset macros (`include_icon!`, `include_bitmap!`, `include_mesh!`, `include_nine_patch!`, `include_skin!`,
`include_audio!`) used to live here but have moved to `bmc-render-macros`.

## Boundaries

**IS its responsibility:**

- `json!` compile-time JSON template expansion
- `fmt_impl!` captured-variable format string rewriting

**IS NOT its responsibility:**

- Asset packaging (that is `bmc-render-macros`)
- Runtime string formatting (that is `ufmt`, invoked by the generated code)
- Wire format types (that is `bmc-wasm-protocol`)

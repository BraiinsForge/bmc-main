# bmc-render-macros

Proc macros that compile and embed assets at build time.

## Purpose

Provides the following proc macros:

- `include_icon!` -- compiles SVG into compact binary path data (via usvg)
- `include_bitmap!` -- embeds a PNG/raster image as raw bytes
- `include_mesh!` -- parses a `.glb` mesh, validates constraints, quantizes vertices, and packs into an optimized binary
  format
- `include_nine_patch!` -- decodes `.9.png` border, strips it, embeds inner bitmap + insets
- `include_skin!` -- loads a skin zip or directory of `.png`/`.9.png` files with a `skin.toml` manifest, embeds all
  assets with parsed metadata
- `include_audio!` -- embeds an audio file (WAV, OGG, MP3) as raw bytes

All processing (SVG simplification, 9-patch parsing, mesh validation, texture compression) happens at compile time.
Cargo tracks source files for recompilation.

## Boundaries

**IS its responsibility:**

- Build-time asset processing and validation
- Emitting `const`-compatible Rust expressions referencing `bmc_wasm_sdk` types
- Compile-time error reporting for invalid assets (bad mesh counts, missing normals, non-power-of-2 textures, malformed
  9-patches)

**IS NOT its responsibility:**

- Runtime rendering (that is `bmc-render`)
- Runtime bitmap registration (that is `bmc-render-skin`)
- SDK-specific macros like `json!` or `fmt_impl!` (that is `bmc-wasm-sdk-macros`)

# bmc-render-macros

Proc macros that compile and package static assets at build time.

## Purpose

Provides the following proc macros:

- `include_icon!` -- compiles SVG into compact binary path data (via usvg)
- `include_bitmap!` -- packages a PNG/raster image and emits its static descriptor
- `include_mesh!` -- parses a `.glb` mesh, validates constraints, quantizes vertices, and packs into an optimized binary
  format
- `include_nine_patch!` -- decodes a `.9.png` border and packages the inner bitmap + insets
- `include_skin!` -- loads a skin zip or directory and packages its processed bitmap descriptors assets with parsed
  metadata
- `include_audio!` -- packages an audio file (WAV, OGG, MP3)

All processing (SVG simplification, 9-patch parsing, mesh validation, texture compression) happens at compile time.
Cargo tracks source files for recompilation. WASM builds emit package records that the Nix widget build extracts from
the final module; native builds retain embedded payloads for storybook rendering.

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

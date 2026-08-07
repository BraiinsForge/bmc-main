# bmc-wasm-sdk

Thin glue layer for building WASM widgets on Braiins Deck.

## Purpose

Provides host FFI bindings, tree serialization, and UI primitives for WASM widget authors. Re-exports types from
multiple crates into a single ergonomic surface:

- Tree-building API: `col`, `row`, `text`, `canvas`, `button!`, `progress_bar!`, `number_input!`, `props!`, `style!`
- Asset macros (from `bmc-render-macros`): `include_icon!`, `include_bitmap!`, `include_mesh!`, `include_nine_patch!`,
  `include_skin!`, `include_audio!`
- String formatting: `fmt!`, `json!` (from `bmc-wasm-sdk-macros`)
- Skin types (from `bmc-render-skin`): `NinePatch`, `Skin`, `ButtonSkin`
- Host services (wasm32 only): WebSocket, HTTP listener, mDNS, SSDP, KV store, LED control, calendar, UDP broadcast

When compiled for native targets (non-wasm32), FFI-dependent modules are gated out. The tree-building API and pure types
remain available for the gallery.

Optional `math-3d` feature re-exports `glam` for widgets doing 3D math (~6 KB added to WASM binary).

## Boundaries

**IS its responsibility:**

- Host FFI bindings (WASM imports/exports)
- Tree node construction and binary serialization
- Ergonomic macros for widget authors (`button!`, `props!`, `style!`, `fmt!`)
- Re-exporting SDK surface from underlying crates

**IS NOT its responsibility:**

- Rendering the widget tree (that is `bmc-render`)
- Asset compilation (that is `bmc-render-macros`, invoked at build time)
- Wire format definitions (that is `bmc-wasm-protocol`)
- WASM interpreter / host runtime (that is `bmc-wasm-runtime`)

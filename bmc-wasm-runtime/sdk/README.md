# bmc-wasm-sdk

Thin glue layer for building WASM widgets on Braiins Deck.

## Purpose

Provides host FFI bindings, tree serialization, and UI primitives for WASM widget authors. Re-exports types from
multiple crates into a single ergonomic surface:

- Tree-building API (from `bmc-ui`): `col`, `row`, `text`, `canvas`, `button!`, `progress_bar!`, `number_input!`,
  `props!`, `style!`
- Asset macros wrapping `bmc-render-macros`: `include_svg!`, `include_bitmap!`, `include_mesh!`, `include_nine_patch!`,
  `include_skin!`, `include_audio!`
- String formatting: `fmt!`, `json!` (from `bmc-wasm-sdk-macros`)
- Skin types (from `bmc-ui`, registered through `assets`): `NinePatch`, `Skin`, `ButtonSkin`
- Host services (wasm32 only): WebSocket, HTTP listener, mDNS, SSDP, KV store, LED control, calendar, UDP broadcast

When compiled for native targets (non-wasm32), FFI-dependent modules are gated out. The tree-building API and pure types
remain available for the gallery.

Optional `math-3d` feature re-exports `glam` for widgets doing 3D math (~6 KB added to WASM binary).

## Boundaries

**IS its responsibility:**

- Host FFI bindings (WASM imports/exports)
- Submitting the encoded tree and registering assets with the host
- Ergonomic macros for widget authors (`fmt!`, the asset macros)
- Re-exporting SDK surface from underlying crates

**IS NOT its responsibility:**

- The tree, its builders, components and keyword macros (that is `bmc-ui`)
- Rendering the widget tree (that is `bmc-render`)
- Asset compilation (that is `bmc-render-macros`, invoked at build time)
- Wire format definitions (that is `bmc-wasm-protocol`)
- WASM interpreter / host runtime (that is `bmc-wasm-runtime`)

# bmc-render

Rendering engine for the Braiins Deck display. Owns GPU rendering, layout and interaction handling for the tree `bmc-ui`
decodes.

## Purpose

Platform-independent rendering pipeline used by both the device runtime and the gallery dev tool. Provides:

- `Renderer` trait and `gpu::FemtoVgRenderer` (femtovg/glow) implementation
- Layout of the widget tree `bmc-ui`'s codec decodes, re-exported as `bmc_render::ui`
- Layout computation via taffy
- Drawing primitives: shapes, text (cosmic-text), icons, bitmaps, 9-patch, 3D mesh, sphere projection
- Interaction state: hit testing, touch/click event routing
- Animation and transition interpolation
- The `include_*!` asset macros for native code, wrapping `bmc-render-macros`, and the registration of their assets with
  a renderer (`assets`)

Renderer-specific assets (icons, 9-patch PNGs, mesh GLBs) live in `bmc-render/assets/`. Workspace fonts live at
`assets/fonts/` (top level) and are loaded by the host. Scene files (`*.scene.rs`) live alongside source files,
documenting renderer capabilities in the gallery.

## Boundaries

**IS its responsibility:**

- All GPU drawing (femtovg canvas, glow mesh pipeline)
- Taffy layout tree construction and computation
- Keying the static half of a decoded tree for the cached layer
- Interaction state management (touch tracking, hit testing)
- Registering compile-time assets with a renderer for native consumers: overlays, keyboard, gallery scenes
- Hosting `*.scene.rs` files that exercise rendering features

**IS NOT its responsibility:**

- Widget business logic (that is `bmc-wasm-sdk` / individual WASM widgets)
- The tree, its codec and the skin types (that is `bmc-ui`)
- Asset compilation proc macros (that is `bmc-render-macros`)
- WASM host runtime / interpreter (that is `bmc-wasm-runtime`)

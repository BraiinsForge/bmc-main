# bmc-render

Rendering engine for the Braiins Deck display. Owns GPU rendering, layout, tree deserialization, and interaction
handling.

## Purpose

Platform-independent rendering pipeline used by both the device runtime and the gallery dev tool. Provides:

- `Renderer` trait and `gpu::FemtoVgRenderer` (femtovg/glow) implementation
- Tree deserialization from the binary wire format produced by widgets
- Layout computation via taffy
- Drawing primitives: shapes, text (cosmic-text), icons, bitmaps, 9-patch, 3D mesh, sphere projection
- Interaction state: hit testing, touch/click event routing
- Animation and transition interpolation

Renderer-specific assets (icons, 9-patch PNGs, mesh GLBs) live in `bmc-render/assets/`. Workspace fonts live at
`assets/fonts/` (top level) and are loaded by the host. Scene files (`*.scene.rs`) live alongside source files,
documenting renderer capabilities in the gallery.

## Boundaries

**IS its responsibility:**

- All GPU drawing (femtovg canvas, glow mesh pipeline)
- Taffy layout tree construction and computation
- Deserializing the widget tree binary into renderable nodes
- Interaction state management (touch tracking, hit testing)
- Hosting `*.scene.rs` files that exercise rendering features

**IS NOT its responsibility:**

- Widget business logic (that is `bmc-wasm-sdk` / individual WASM widgets)
- Skin type definitions (that is `bmc-render-skin`)
- Asset compilation proc macros (that is `bmc-render-macros`)
- WASM host runtime / interpreter (that is `bmc-wasm-runtime`)

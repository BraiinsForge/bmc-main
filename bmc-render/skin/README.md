# bmc-render-skin

Lightweight skin and 9-patch types shared between native rendering and WASM widgets.

## Purpose

Defines `NinePatch`, `NinePatchAsset`, `Skin`, `SkinAsset`, `ButtonSkin`, `SliderSkin`, and `SkinPalette` types. Also
provides the `parse_nine_patch_insets` utility that reads stretch markers from `.9.png` borders.

Intentionally keeps its dependency footprint minimal (only `bmc-wasm-protocol`) so that both heavy-weight consumers
(`bmc-render` with GPU deps) and lightweight consumers (`bmc-wasm-sdk` compiled to WASM) can depend on it without
pulling in rendering libraries.

## Boundaries

**IS its responsibility:**

- Skin, 9-patch, button skin, and slider skin type definitions
- Color palette types for skin theming
- 9-patch inset parsing from pixel data (pure logic, no image decoding)
- Runtime bitmap registration via a pluggable callback (`init()`)
- Hex color string parsing utility

**IS NOT its responsibility:**

- Image decoding (done by proc macros at build time)
- Actual rendering of 9-patches or skins (done by `bmc-render`)
- Asset embedding macros (done by `bmc-render-macros`)

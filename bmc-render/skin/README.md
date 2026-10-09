# bmc-render-skin

Skin and 9-patch registration with the host.

## Purpose

The skin types (`NinePatch`, `NinePatchAsset`, `Skin`, `SkinAsset`, `ButtonSkin`, `SliderSkin`) and the `.9.png` inset
parser live in `bmc-ui` and are re-exported here. This crate adds the one impure step: turning a compile-time asset into
a registered `BitmapId` through a registrar the host installs with `init()`, as `ensure_nine_patch_registered` and the
`SkinRegistry` extension trait on `Skin`.

Depends only on `bmc-ui`, so both `bmc-render` (GPU deps) and `bmc-wasm-sdk` (compiled to WASM) can use it without
pulling in rendering libraries.

## Boundaries

**IS its responsibility:**

- Runtime bitmap registration via a pluggable callback (`init()`)
- Registering a skin's 9-patches and a `NinePatchAsset` with the host

**IS NOT its responsibility:**

- The skin and 9-patch types themselves, and inset parsing (that is `bmc-ui`)
- Image decoding (done by proc macros at build time)
- Actual rendering of 9-patches or skins (done by `bmc-render`)
- Asset embedding macros (done by `bmc-render-macros`)

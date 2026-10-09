# bmc-ui

Design-system vocabulary and tree-wire primitives shared by the native renderer (`bmc-render`) and the widget SDK
(`bmc-wasm-sdk`).

## Purpose

Holds the pure data both sides of the widget tree agree on: the `Node` and `Draw` tree with its builders, the one codec
that encodes and decodes it, the static/dynamic classification the decoder records, colours, text styles, fills, arcs,
animation and progress descriptors, skin and 9-patch types, mesh views, the `u16` renderer resource ids and the
package-asset descriptors the `include_*!` macros emit.

Pure on every target: no host calls and no `target_arch` gates (the `no-target-cfg-in-ui` ast-grep rule enforces the
latter), so native code and wasm widgets compile the very same crate.

## Boundaries

**IS its responsibility:**

- The tree (`Node`, `Draw`), its builders and pure draw constructors
- The tree wire format: `encode` and `decode`, byte for byte
- Which nodes and draws are dynamic (`partition`)
- Colour, typography, fill, arc, animation, progress, skeleton and relative-time types
- Skin, 9-patch and mesh-view data, and `Orientation`
- Renderer resource ids (`SvgId`, `BitmapId`, `MeshId`, `AudioId`)
- Package asset identity (`PackageAssetId`, `PackageAssetRef`, `StaticAssetSource`)

**IS NOT its responsibility:**

- The guest↔host contract beyond the tree: params, system snapshots, fetch, versioning (`bmc-wasm-protocol`)
- Host-side id pools and the package record format (`bmc-wasm-protocol`)
- Registering assets with a host (`bmc-render-skin`, `bmc-wasm-sdk`)
- Keying the static half for a cached layer, layout and drawing (`bmc-render`)

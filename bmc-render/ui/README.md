# bmc-ui

Design-system vocabulary and tree-wire primitives shared by the native renderer (`bmc-render`) and the widget SDK
(`bmc-wasm-sdk`).

## Purpose

Holds the pure data both sides of the widget tree agree on: colours, text styles, fills, arcs, animation and progress
descriptors, node and draw tags, the `u16` renderer resource ids and the package-asset descriptors the `include_*!`
macros emit.

Pure on every target: no host calls and no `target_arch` gates (the `no-target-cfg-in-ui` ast-grep rule enforces the
latter), so native code and wasm widgets compile the very same crate.

## Boundaries

**IS its responsibility:**

- Colour, typography, fill, arc, animation, progress, skeleton and relative-time types
- Node and draw command tags of the tree wire format
- Renderer resource ids (`SvgId`, `BitmapId`, `MeshId`, `AudioId`)
- Package asset identity (`PackageAssetId`, `PackageAssetRef`, `StaticAssetSource`)

**IS NOT its responsibility:**

- The guest↔host contract beyond the tree: params, system snapshots, fetch, versioning (`bmc-wasm-protocol`)
- Host-side id pools and the package record format (`bmc-wasm-protocol`)
- Layout and drawing (`bmc-render`) or host bindings (`bmc-wasm-sdk`)

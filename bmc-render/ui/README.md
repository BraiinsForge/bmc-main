# bmc-ui

Design-system vocabulary and tree-wire primitives shared by the native renderer (`bmc-render`) and the widget SDK
(`bmc-wasm-sdk`).

## Purpose

Holds the pure data both sides of the widget tree agree on: the `Node` and `Draw` tree with its builders, the
design-system components and keyword macros (`props!`, `style!`, `button!`, …) built over them, the one codec that
encodes and decodes the tree, the static/dynamic classification the decoder records, colours, text styles and
typographic characters, fills, arcs, animation and progress descriptors, skin and 9-patch types, mesh views, the `u16`
renderer resource ids and the asset descriptors the `include_*!` macros emit.

Pure on every target: no host calls and no `target_arch` gates (the `no-target-cfg-in-ui` ast-grep rule enforces the
latter), so native code and wasm widgets compile the very same crate. A builder that needs a registered asset takes its
id; one that needs the time takes a `SystemTime`. The SDK supplies both from the host.

## Boundaries

**IS its responsibility:**

- The tree (`Node`, `Draw`), its builders and the draw constructors that take a registered id
- The components composed from the tree (modal, tag, notification, progress bar, skeleton, switcher, number input,
  status overlay, text) and the keyword-argument macros over the builders
- The tree wire format: `encode` and `decode`, byte for byte
- Which nodes and draws are dynamic (`partition`)
- Colour, typography, fill, arc, animation, progress, skeleton and relative-time types
- `SystemTime`, the instant a tree anchors its host-driven time labels to
- Skin, 9-patch and mesh-view data, and `Orientation`
- Renderer resource ids (`SvgId`, `BitmapId`, `MeshId`, `AudioId`)
- Package asset identity (`PackageAssetId`, `PackageAssetRef`, `StaticAssetSource`) and the `Svg`, `Bitmap` and `Audio`
  descriptors

**IS NOT its responsibility:**

- The guest↔host contract beyond the tree: params, system snapshots, fetch, versioning (`bmc-wasm-protocol`)
- Host-side id pools and the package record format (`bmc-wasm-protocol`)
- Registering assets with a host, reading its clock or its touch input (`bmc-render`, `bmc-wasm-sdk`)
- Keying the static half for a cached layer, layout and drawing (`bmc-render`)

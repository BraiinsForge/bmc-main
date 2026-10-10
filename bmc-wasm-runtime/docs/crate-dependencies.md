# WASM Crate Dependency Graph

Path dependencies among the wasm-side crates, dependant → dependencies. Shared libraries outside this set (`bmc-led`,
`bmc-platform`, `bmc-shared-time`, the widget manifest crates) are omitted.

```text
bmc-gallery         → bmc-render, bmc-wasm-sdk, bmc-render-keyboard, the overlays and widgets it stages
bmc-wasm-runtime    → bmc-render, bmc-wasm-assets, bmc-wasm-protocol
bmc-render          → bmc-ui, bmc-render-macros, bmc-wasm-protocol; at build time bmc-svg-compiler
bmc-wasm-sdk        → bmc-ui, bmc-render-macros, bmc-wasm-sdk-macros, bmc-wasm-protocol
bmc-render-macros   → bmc-ui, bmc-wasm-assets, bmc-svg-compiler
bmc-wasm-assets     → bmc-wasm-protocol
bmc-wasm-protocol   → bmc-ui
bmc-wasm-sdk-macros → (none)
bmc-svg-compiler    → (none)
bmc-ui              → (none)
```

## Crate roles

| Crate                 | Role                                                     | Location                         |
| --------------------- | -------------------------------------------------------- | -------------------------------- |
| `bmc-ui`              | Tree, components, macros, codec, design vocabulary       | `bmc-render/ui/`                 |
| `bmc-wasm-protocol`   | Guest↔host contract beyond the tree; re-exports `bmc-ui` | `bmc-wasm-runtime/protocol/`     |
| `bmc-svg-compiler`    | SVG → compact binary path compilation                    | `bmc-wasm-runtime/svg-compiler/` |
| `bmc-wasm-assets`     | Package asset records: digest, record format, extraction | `bmc-wasm-assets/`               |
| `bmc-render-macros`   | `include_*!` asset proc macros (svg, bitmap, mesh, …)    | `bmc-render/macros/`             |
| `bmc-wasm-sdk-macros` | Compile-time `json!` / `fmt!` template proc macros       | `bmc-wasm-runtime/sdk-macros/`   |
| `bmc-wasm-sdk`        | Widget authoring API: host bindings over `bmc-ui`        | `bmc-wasm-runtime/sdk/`          |
| `bmc-render`          | Render engine: FemtoVG, layout, interaction, animation   | `bmc-render/`                    |
| `bmc-wasm-runtime`    | WASM interpreter, host state, network, testbed/capture   | `bmc-wasm-runtime/`              |
| `bmc-gallery`         | Dev tool — visual catalog of SDK components              | `bmc-gallery/`                   |

## Leaf crates

`bmc-ui`, `bmc-svg-compiler` and `bmc-wasm-sdk-macros` are the leaves — no path dependencies of their own.
`bmc-wasm-protocol` depends only on `bmc-ui`; `bmc-render`, `bmc-wasm-sdk` and `bmc-render-macros` take `bmc-ui`
directly as well, for the tree and its builders and for the descriptor types the asset macros expand to. The proc macros
name those types through a crate root passed as their first argument, which the SDK's and bmc-render's `include_*!`
wrappers supply as `$crate`, so the overlays, the keyboard and the macro fixture build without the SDK. The protocol and
SVG-compiler crates live under `bmc-wasm-runtime/` for historical reasons; the paths work and there's no benefit to
moving them.

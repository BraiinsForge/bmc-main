# WASM Crate Dependency Graph

```
                       ┌─────────────────┐
                       │   bmc-gallery   │
                       └─┬──────┬──────┬─┘
                         │      │      │
                         ▼      ▼      ▼
            ┌────────────┐ ┌──────────────┐ ┌──────────────────┐
            │ bmc-render │ │ bmc-wasm-sdk │ │ bmc-render-skin  │
            └─┬───┬──────┘ └─┬───┬───┬──┬─┘ └────┬─────────────┘
              │   │          │   │   │  │        │
              │   ▼          │   │   ▼  └────────┤
              │ ┌──────────────────────┐         │
              │ │   bmc-render-macros  │         │
              │ └─┬───────────┬─────┬──┘         │
              │   │           │     │            │
              │   ▼           ▼     ▼            ▼
              │ ┌────────────────┐ ┌─────────────────────┐
              │ │ bmc-wasm-sdk-  │ │  bmc-wasm-protocol  │ ──→ bmc-ui ◄── leaf (no deps)
              │ │     macros     │ │                     │
              │ └────────┬───────┘ └─────────────────────┘
              │          │                ▲
              │          └────────────────┤
              └───────────────────────────┘

    ┌───────────────────┐
    │ bmc-icon-compiler │ ◄── leaf (no deps)
    └───────────────────┘
         ▲         ▲
    (build-dep)  (dep)
         │         │
    bmc-render   bmc-render-macros

    ┌──────────────────┐
    │ bmc-wasm-runtime │ ──→ bmc-render, bmc-wasm-protocol
    └──────────────────┘
```

## Crate roles

| Crate                 | Role                                                                                                                              | Location                          |
| --------------------- | --------------------------------------------------------------------------------------------------------------------------------- | --------------------------------- |
| `bmc-ui`              | The widget tree, its codec and design-system vocabulary: colours, text styles, skins, renderer ids, asset descriptors             | `bmc-render/ui/`                  |
| `bmc-wasm-protocol`   | Guest↔host contract beyond the tree: params, system snapshots, fetch, versioning; re-exports `bmc-ui`                             | `bmc-wasm-runtime/protocol/`      |
| `bmc-icon-compiler`   | SVG → compact binary icon compilation                                                                                             | `bmc-wasm-runtime/icon-compiler/` |
| `bmc-render-skin`     | Skin and 9-patch registration with the host (types from `bmc-ui`)                                                                 | `bmc-render/skin/`                |
| `bmc-render-macros`   | Asset proc macros (`include_icon!`, `include_bitmap!`, `include_mesh!`, `include_audio!`, `include_skin!`, `include_nine_patch!`) | `bmc-render/macros/`              |
| `bmc-wasm-sdk-macros` | Compile-time `json!` / `fmt!` template proc macros                                                                                | `bmc-wasm-runtime/sdk-macros/`    |
| `bmc-wasm-sdk`        | Widget authoring API (tree builders, host bindings)                                                                               | `bmc-wasm-runtime/sdk/`           |
| `bmc-render`          | Rendering engine (FemtoVG, layout, interaction, components, animation)                                                            | `bmc-render/`                     |
| `bmc-wasm-runtime`    | WASM interpreter, host state, network, testbed/capture bins                                                                       | `bmc-wasm-runtime/`               |
| `bmc-gallery`         | Dev tool — visual catalog of SDK components                                                                                       | `bmc-gallery/`                    |

## Leaf crates

`bmc-ui` and `bmc-icon-compiler` are the two true leaves — no path dependencies of their own. `bmc-wasm-protocol`
depends only on `bmc-ui`. The protocol and icon-compiler crates live under `bmc-wasm-runtime/` for historical reasons;
the paths work and there's no benefit to moving them.

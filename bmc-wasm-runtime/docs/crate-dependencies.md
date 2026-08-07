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
              │ │ bmc-wasm-sdk-  │ │  bmc-wasm-protocol  │ ◄── leaf (no deps)
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
| `bmc-wasm-protocol`   | Wire-format types, color system, node constants                                                                                   | `bmc-wasm-runtime/protocol/`      |
| `bmc-icon-compiler`   | SVG → compact binary icon compilation                                                                                             | `bmc-wasm-runtime/icon-compiler/` |
| `bmc-render-skin`     | Skin/theme definitions (9-patch, palettes)                                                                                        | `bmc-render/skin/`                |
| `bmc-render-macros`   | Asset proc macros (`include_icon!`, `include_bitmap!`, `include_mesh!`, `include_audio!`, `include_skin!`, `include_nine_patch!`) | `bmc-render/macros/`              |
| `bmc-wasm-sdk-macros` | Compile-time `json!` / `fmt!` template proc macros                                                                                | `bmc-wasm-runtime/sdk-macros/`    |
| `bmc-wasm-sdk`        | Widget authoring API (tree builders, host bindings)                                                                               | `bmc-wasm-runtime/sdk/`           |
| `bmc-render`          | Rendering engine (FemtoVG, layout, interaction, components, animation)                                                            | `bmc-render/`                     |
| `bmc-wasm-runtime`    | WASM interpreter, host state, network, testbed/capture bins                                                                       | `bmc-wasm-runtime/`               |
| `bmc-gallery`         | Dev tool — visual catalog of SDK components                                                                                       | `bmc-gallery/`                    |

## Leaf crates

`bmc-wasm-protocol` and `bmc-icon-compiler` are the two true leaves — no path dependencies of their own. Both live under
`bmc-wasm-runtime/` for historical reasons; the paths work and there's no benefit to moving them.

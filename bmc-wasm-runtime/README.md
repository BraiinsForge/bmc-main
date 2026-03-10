# WASM Widget Runtime

WebAssembly runtime for Braiins Deck remote widget overlays. Widgets are compiled to WASM and rendered with
GPU-accelerated host-side flex layout (Taffy), text shaping (cosmic-text), and rendering (FemtoVG / OpenGL).

## Architecture

```
┌─────────────────┐     ┌──────────────────────┐
│   WASM Widget   │     │        Host           │
│  (bmc-wasm-sdk) │────▶│  (bmc-wasm-runtime)   │
│                 │     │                      │
│  - UI tree      │     │  - Deserialize tree  │
│  - Anim decl    │     │  - Taffy flex layout │
│  - State        │     │  - FemtoVG rendering │
│                 │     │  - cosmic-text text  │
└─────────────────┘     └──────────────────────┘
```

Widgets build a declarative UI tree that gets serialized and sent to the host for layout and rendering. Animations and
transitions are declared in the tree and computed host-side, keeping WASM binaries small by offloading text shaping,
layout, and animation math to native code.

See [GPU Rendering](docs/devlog/hosted-wasm/gpu-rendering.md) and [SVG Icons](docs/devlog/hosted-wasm/svg-icons.md) for
architecture details.

## Quick Start

```rust
use bmc_wasm_sdk::*;

#[unsafe(no_mangle)]
pub extern "C" fn init(width: u32, height: u32) { /* store dimensions */ }

#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {
    let root = col(props!(padding: 24.0, gap: 16.0, background: BLACK), [
        text("Hello", style!(size: 32, weight: 600)),
        row(props!(gap: 12.0), [
            button!("Click me", style: Primary),
            button!("Cancel", style: Secondary),
        ]),
        spacer(1.0),
        text(
            &format_duration(remaining_secs, true),
            style!(size: 20, color: GRAY_30),
        ),
    ]);
    render_ui(1280, 480, root);
    request_frame_after(1_000);
}
```

For the full SDK API reference (all layout primitives, canvas drawing, animations, transitions, modals, colors,
formatting, macros), run:

```bash
make docs
```

## Development

```bash
# Run testbed with hot reload (preview mode: all 4 sizes)
make dev EXAMPLE=spacex-launch

# Build and run release (preview mode)
make run EXAMPLE=spacex-launch

# Single-size mode (default hello-widget)
make dev

# CPU profile with samply + memory stats
make profile EXAMPLE=spacex-launch ARGS=--preview

# Check WASM binary size
make size EXAMPLE=spacex-launch

# Browse SDK API docs
make docs
```

`make dev` and `make run` include `--preview` by default, which renders all 4 widget size variants (Full, Large, Medium,
Small) simultaneously in a masonry layout with a performance overlay. Pass extra flags via `ARGS=...`.

## Crate Structure

- `bmc-wasm-runtime` — Host runtime: WASM execution (wasmi), flex layout (taffy), GPU rendering (FemtoVG), text shaping
  (cosmic-text)
- `bmc-wasm-sdk` — Widget SDK (compiled to WASM)
- `bmc-wasm-protocol` — Shared types and constants

## Example Widgets

- `examples/hello-widget` — Minimal skeleton (default for `make dev`)
- `examples/spacex-launch` — SpaceX next launch countdown with network fetching and JSON parsing
- `examples/iss-position` — ISS tracker with 3D globe, SGP4 orbital ground track, and day/night terminator overlay

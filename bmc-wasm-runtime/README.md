# WASM Widget Runtime

WebAssembly runtime for Braiins Deck remote widget overlays. Widgets are compiled to WASM and rendered with
GPU-accelerated host-side flex layout (Taffy), text shaping (cosmic-text), and rendering (FemtoVG / OpenGL).

## Architecture

```
┌─────────────────┐     ┌──────────────────────┐
│   WASM Widget   │     │        Host          │
│  (bmc-wasm-sdk) │────▶│  (bmc-wasm-runtime)  │
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

When a widget calls `request_frame_after(ms)`, that schedules the next full WASM recompute. The host may still wake
earlier at its animation cadence to replay cached-tree transitions smoothly, without rerunning WASM on every animation
frame.

See [GPU Rendering](docs/devlog/hosted-wasm/gpu-rendering.md) and [SVG Icons](docs/devlog/hosted-wasm/svg-icons.md) for
architecture details.

## Quick Start

```rust
use bmc_wasm_sdk::*;

#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {
    let WidgetSize { width: w, height: h, .. } = widget_size();
    let root = col(props!(padding: 24.0, gap: 16.0, background: BLACK), [
        text("Hello", style!(size: 32, weight: 600)),
        row(props!(gap: 12.0), [
            button!("Click me", style: Primary),
            button!("Cancel", style: Secondary),
        ]),
        spacer(1.0),
    ]);
    render_ui(w, h, root);
    request_frame_after(1_000);
}
```

`init` / `on_params_update` / `on_system_update` / `unload` are all optional — define them only when you need them. The
full widget lifecycle (when each hook fires, which host imports are legal in each phase, the trap-vs-soft-fail guard
matrix) lives in the [`bmc_wasm_sdk`](sdk/src/lib.rs) crate-level rustdoc — render it locally with `just wasm::docs`.

## Development

Local tasks run through the `wasm` just module; `just wasm::` lists them all. The flake is the source of truth for
builds (`nix build .#wasm-capture`) — the recipes are local-iteration shortcuts.

```bash
just wasm::dev <widget>      # hot-reload testbed (preview: all sizes)
just wasm::run <widget>      # release preview
just wasm::profile <widget>  # CPU profile (samply) + memory stats
just wasm::size <widget>     # WASM binary size
just wasm::docs              # browse SDK API docs
```

## Widgets

Widgets live in two workspaces:

- `../widgets-wasm-examples/` — SDK demos (`hello-widget`, `metronome`, …)
- `widgets-wasm/` — production widgets (`clock`, `weather`, `iss-position`, `spacex-launch`, `mining-info`, …)

Developer guides for writing widgets — best practices, params, system settings, display geometry, and the regression
workflow — live in [`docs/devel/wasm-widgets/`](../docs/devel/wasm-widgets/README.md). Read Best Practices before
writing or changing a widget.

## Visual Regression Testing

Pixel-level regression testing uses headless EGL capture and [odiff](https://github.com/dmtrKovalenko/odiff). The
widget-author workflow (opting in, recording fixtures, setting and refreshing baselines) is documented in
[`docs/devel/wasm-widgets/regression-testing.md`](../docs/devel/wasm-widgets/regression-testing.md); the host-side
internals are in [`docs/regression-testing.md`](docs/regression-testing.md).

```bash
just wasm::record <widget>            # record fetch/event fixtures
just wasm::capture <widget>           # capture frames
just wasm::preview <widget>           # review the captured mp4
just wasm::update-baselines <widget>  # set/refresh the baseline
just wasm::verify <widget>            # capture + diff against baseline
just wasm::verify-all                 # all widgets
```

## Crate Structure

- `bmc-wasm-runtime` — host runtime: WASM execution (wasmi), flex layout (taffy), GPU rendering (FemtoVG), text shaping
  (cosmic-text)
- `bmc-wasm-sdk` (`sdk/`) — widget SDK, compiled to WASM
- `bmc-wasm-sdk-macros` (`sdk-macros/`) — SDK procedural macros
- `bmc-wasm-protocol` (`protocol/`) — shared types and constants
- `bmc-svg-compiler` (`svg-compiler/`) — build-time SVG → draw-call compiler

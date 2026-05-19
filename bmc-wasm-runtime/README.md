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
matrix) lives in the [`bmc_wasm_sdk`](sdk/src/lib.rs) crate-level rustdoc — render it locally with:

```bash
just wasm::docs
```

For the full SDK API reference (layout primitives, canvas drawing, animations, transitions, modals, colors, formatting,
macros) browse the same generated docs.

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

## Visual Regression Testing

Pixel-level visual regression testing using headless EGL capture and [odiff](https://github.com/dmtrKovalenko/odiff)
comparison. See docs/devlog/hosted-wasm/visual-regression-testing.md for architecture details.

```bash
make regression-test EXAMPLE=hello-widget   # capture + diff one widget
make regression-test-all                     # capture + diff all widgets
```

### Adding a new widget

```bash
# 1. Generate a capture config
capture init examples/my-widget
#    → edit examples/my-widget/capture/config.toml

# 2. Record fixtures (if the widget fetches data or uses events)
make record EXAMPLE=my-widget SIZE=full
#    → uncomment [fixtures] section in config.toml

# 3. Capture, review, and set the baseline
make capture EXAMPLE=my-widget
make preview EXAMPLE=my-widget              # review captures/my-widget/preview_full.mp4
make update-baselines EXAMPLE=my-widget     # compress into baselines.7z

# 4. Verify the baseline passes
make regression-test EXAMPLE=my-widget
```

### Accepting visual changes

When a widget intentionally changes its rendering:

```bash
# 1. Run regression test — review diffs in captures/report.html
make regression-test EXAMPLE=my-widget

# 2. If the changes look correct, update the baseline
make update-baselines EXAMPLE=my-widget

# 3. Verify and commit
make regression-test EXAMPLE=my-widget
git add examples/my-widget/capture/baselines.7z
```

If the widget's network calls changed, re-record fixtures first:

```bash
make record EXAMPLE=my-widget SIZE=full
make update-baselines EXAMPLE=my-widget
```

## Crate Structure

- `bmc-wasm-runtime` — Host runtime: WASM execution (wasmi), flex layout (taffy), GPU rendering (FemtoVG), text shaping
  (cosmic-text)
- `bmc-wasm-sdk` — Widget SDK (compiled to WASM)
- `bmc-wasm-protocol` — Shared types and constants

## Example Widgets

- `examples/hello-widget` — Minimal skeleton (default for `make dev`)
- `examples/spacex-launch` — SpaceX next launch countdown with network fetching and JSON parsing
- `examples/iss-position` — ISS tracker with 3D globe, SGP4 orbital ground track, and day/night terminator overlay

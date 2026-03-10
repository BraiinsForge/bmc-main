# Phase 6: Perf Overlay Extraction

The stats panel (FPS counter, stacked timing chart, legend) lived inline in `testbed.rs` — ~200 lines of drawing code,
`FpsTracker`, `FrameSample`, and 5 color constants. The on-device host needed the same overlay but couldn't use it
without pulling in testbed dependencies.

Extracted everything into `src/perf_overlay.rs` behind a `perf-overlay` cargo feature. The `testbed` feature implies it,
so existing builds are unchanged. On-device hosts opt in independently.

## What moved

| From `testbed.rs`                 | To `perf_overlay.rs`         |
| --------------------------------- | ---------------------------- |
| `FpsTracker` struct + impl        | `PerfOverlay` (public)       |
| `FrameSample` struct              | `FrameSample` (private)      |
| `COL_WASM` / `COL_TREE` / etc.    | Module-level constants       |
| FPS text + chart + legend drawing | `PerfOverlay::draw()` method |

## What stays in testbed

- Reload button (`draw_button`, `InteractionState`)
- `write_perf_report` (CLI perf automation with `--perf-report`)
- Stats FBO setup + compositing

The testbed's `draw_stats_panel` is now a thin wrapper: draws the reload button, then calls `overlay.draw()` with a
y-offset.

## Feature gate

```toml
# Cargo.toml
[features]
perf-overlay = []
testbed = ["perf-overlay", ...]
```

```rust
// lib.rs
#[cfg(feature = "perf-overlay")]
pub mod perf_overlay;
```

## On-device integration

```toml
bmc-wasm-runtime = { path = "...", features = ["perf-overlay"] }
```

```rust
use bmc_wasm_runtime::perf_overlay::PerfOverlay;

let mut overlay = PerfOverlay::new();

// each frame:
overlay.tick(frame_us, rendered, runtime.last_timings());
overlay.draw(renderer, w, h, 0.0);  // last arg = y_offset in px
```

`draw()` uses only `&mut dyn Renderer` trait methods (`fill_rect`, `draw_text`, `measure_text`) and colors from
`bmc_wasm_runtime::colors`. No GL, no windowing, no testbed deps.

## Public API

| Method                           | Purpose                                      |
| -------------------------------- | -------------------------------------------- |
| `PerfOverlay::new()`             | Create with 120-sample ring buffer           |
| `tick(us, rendered, timings)`    | Record a frame. Call every loop iteration.   |
| `draw(renderer, w, h, y_offset)` | Render FPS text + stacked bar chart + legend |
| `avg_render_us()`                | Average frame time in microseconds           |
| `avg_timings()`                  | Average per-component `FrameTimings`         |
| `display_fps()`                  | FPS counter (updates once/sec)               |
| `last_sample_timings()`          | Most recent sample's `FrameTimings`          |

## What `draw()` renders

- **Row 1:** `{avg_ms}ms {fps}fps` — green if \<16ms, red otherwise
- **Chart:** 120-bar stacked chart with 5 timing segments (WASM/Tree/Layout/Render/GPU) scaled to 20ms, gridlines at
  4/8/16ms
- **Legend:** Per-segment averages in ms

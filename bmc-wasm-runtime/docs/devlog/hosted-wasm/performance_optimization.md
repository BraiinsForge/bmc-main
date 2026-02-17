# BDK-287: WASM Runtime Performance Optimization

## Context

The WASM widget runtime burns excessive CPU on both desktop (fans spin up on Ryzen 9 7950X) and the real Braiins Deck
device (only ~25-30fps for a trivial demo). The testbed runs an uncapped render loop with vsync disabled, renders 4
tiles per frame unconditionally, and the pipeline rebuilds all data structures from scratch every frame. Before we can
optimize effectively, we need proper instrumentation to see where time actually goes.

## Phase 1: Instrumentation — measure before optimizing ✅

Add per-component frame timing so the stats panel shows where time is spent.

**Status:** Complete. All instrumentation landed and baseline captured.

### 1a. `FrameTimings` struct in `host_api.rs` ✅

```rust
#[derive(Debug, Clone, Copy, Default)]
pub struct FrameTimings {
    pub wasm_us: u32,        // WASM interpreter execution
    pub deserialize_us: u32, // tree binary deserialization
    pub layout_us: u32,      // Taffy build + compute_layout
    pub render_us: u32,      // render_taffy_node + modals
    pub flush_us: u32,       // FemtoVG canvas.flush()
}
```

Added `pub last_timings: FrameTimings` to `HostState`. Re-exported from `lib.rs`.

### 1b. Instrumented `process_tree()` in `tree.rs` ✅

Split into `process_tree()` (full pipeline) and `layout_and_render()` (layout+render only). Both return `FrameTimings`
with `deserialize_us`, `layout_us`, `render_us` populated via `Instant::now()` measurements.

### 1c. Instrumented WASM execution in `runtime.rs` ✅

`render()` wraps `render_func.call()` to measure `wasm_us`. `host_submit_tree` and `render_cached_tree` capture and
store `FrameTimings` from `process_tree`/`layout_and_render`.

### 1d. Instrumented flush in `testbed.rs` ✅

`renderer().flush()` wrapped with timing per tile, stored as `flush_us`.

### 1e. Stats panel with timing breakdown ✅

Stacked colored bar chart per component. Legend uses colored text (no squares): `WASM`, `Tree`, `Lay`, `RNDR`, `GPU`.
Gridlines at 4/8/16ms with axis tick labels on black backgrounds. Bars snap to pixel grid (no subpixel gaps). Inline
colors replaced with design system constants (`GRAY_*`, `RED_50`, `GREEN_30`, etc).

### 1f. File-based performance report ✅

`--perf-report=<path>` and `--perf-frames=N` CLI flags. Dumps JSON with averages, percentiles (p50/p95/p99),
`animation_only_pct`, and per-frame samples array.

**Baseline captured** (`reports/phase1-baseline.json`):

- 263 fps (uncapped, no vsync)
- 3.8ms avg frame, WASM 3.66ms, Layout 1.09ms, RNDR 305us, GPU flush 143us, Tree 6us

**Files changed:** `host_api.rs`, `tree.rs`, `runtime.rs`, `testbed.rs`, `lib.rs`

---

## Phase 2: Fix testbed render loop

**Status:** Not started.

The single biggest CPU waste on desktop.

### 2a. Enable VSync

Change `SwapInterval::DontWait` to `SwapInterval::Wait(NonZeroU32::new(1).unwrap())`. Caps frame rate at monitor refresh
(~60fps) and gives the CPU idle time.

### 2b. Stop unconditional redraw when idle

Remove `window.request_redraw()` from the idle/else branch in `about_to_wait`. The `WaitUntil(100ms)` timeout is only
needed for hot-reload polling; only request redraw on actual file change.

### 2c. Respect `frame_delay_ms` from widgets

When a widget calls `request_frame_after(delay_ms)`, use that delay for `WaitUntil` scheduling. Compute the earliest
tile delay and use it in `about_to_wait()`.

### 2d. Only render tiles that need updating

Check `tile.runtime.wants_next_frame()` per tile. Skip `begin_frame/render/flush/blit` for tiles with no pending work —
the FBO already holds the last good frame.

**Files:** `testbed.rs`

---

## Phase 3: Eliminate redundant work on animation-only frames

**Status:** Not started.

Currently `render_cached_tree()` clones the raw bytes and runs the full `process_tree()` pipeline (deserialize + layout
\+ render). For animation-only frames, we should skip deserialization and ideally skip layout too (animations only affect
draw command values, not layout).

### 3a. Cache deserialized `TreeNode` instead of raw bytes

In `host_api.rs`, change `cached_tree_data: Option<(Vec<u8>, f32, f32)>` to `cached_tree: Option<(TreeNode, f32, f32)>`.
`TreeNode` is an enum of owned data (Strings, Vecs), no lifetime issues.

### 3b. Split `process_tree()` into two functions

Already done in Phase 1b: `process_tree()` for full pipeline, `layout_and_render()` for layout+render.

### 3c. Update callers in `runtime.rs`

- `host_submit_tree`: call `process_tree()`, then cache the `TreeNode` (not raw bytes)
- `render_cached_tree`: borrow cached `TreeNode`, call `layout_and_render()` directly — no clone, no deserialization

**Files:** `host_api.rs`, `tree.rs`, `runtime.rs`

---

## Phase 4: Reduce per-frame allocations

**Status:** Not started.

### 4a. Reuse `TaffyTree` across frames

Add `pub taffy: TaffyTree<NodeContext>` to `HostState` (initialized with `TaffyTree::with_capacity(64)`).

In `layout_and_render()`, accept `&mut TaffyTree<NodeContext>`, call `taffy.clear()` at the start instead of
`TaffyTree::new()`. Taffy 0.9.2 has `clear()` which resets the tree while keeping internal allocations.

### 4b. Cache `full_text` and `span_offsets` in paragraph layout cache

In `gpu/text.rs`, the paragraph draw method allocates `full_text: String` and `span_offsets: Vec<...>` every call. Move
these into `ParagraphLayoutEntry` so they're computed once during `shape_paragraph()` and reused on cache hits.

### 4c. Short-circuit color interpolation

In `animation.rs`, add early returns to `lerp_color_oklab` and `lerp_color_oklch`:

- `if from == to { return from; }`
- `if t <= 0.0 { return from; }`
- `if t >= 1.0 { return to; }`

**Files:** `host_api.rs`, `tree.rs`, `gpu/text.rs`, `animation.rs`

---

## Phase 5: Extract perf overlay as a reusable component

**Status:** Not started.

The stats panel (`draw_stats_panel`, `FpsTracker`, `FrameTimings`) currently lives inline in `testbed.rs`. Extract it
into a standalone module so it can be dropped into both the testbed and the on-device overlay compositor without
duplication.

### 5a. Create `perf_overlay` module

Move `FpsTracker`, `FrameSample`, the `COL_*` timing constants, and `draw_stats_panel()` into a new module
`src/perf_overlay.rs`. The module depends only on `FrameTimings` and the `Renderer` trait — no winit, glutin, or
testbed-specific types.

Public API:

```rust
pub struct PerfOverlay {
    fps: FpsTracker,
    interaction: InteractionState,
}

impl PerfOverlay {
    pub fn new() -> Self;
    /// Record a frame. Returns whether a reload was requested.
    pub fn tick(&mut self, render_us: u32, rendered: bool, timings: FrameTimings);
    /// Draw the overlay. Returns true if the reload button was clicked.
    pub fn draw(&mut self, renderer: &mut dyn Renderer, w: f32, h: f32) -> bool;
    /// Access collected samples for perf report export.
    pub fn samples(&self) -> &[FrameSample];
}
```

### 5b. Gate behind a cargo feature

Add a `perf-overlay` feature to `Cargo.toml` (enabled by the `testbed` feature and available to the on-device crate).
The on-device crate can enable it for dev/debug builds and disable it for production.

### 5c. Update testbed to use the extracted module

Replace the inline stats panel code in `testbed.rs` with `PerfOverlay` calls. The testbed becomes a thin consumer.

### 5d. Wire into on-device crate

In the device-side overlay compositor, conditionally instantiate `PerfOverlay` when the feature is enabled. Render it
into a corner of the screen using the same `Renderer` trait the device already implements.

**Files:** `src/perf_overlay.rs`, `Cargo.toml`, `testbed.rs`, device crate

---

## Verification

After each phase, capture a perf report and compare:

```bash
cd bmc-wasm-runtime

# Baseline (after Phase 1 instrumentation is in):
make run EXAMPLE=hello-widget ARGS="--perf-report=reports/phase1-baseline.json --perf-frames=600"

# After Phase 2 (render loop fix):
make run EXAMPLE=hello-widget ARGS="--perf-report=reports/phase2-loop.json --perf-frames=600"

# After Phase 3 (cached tree):
make run EXAMPLE=hello-widget ARGS="--perf-report=reports/phase3-cache.json --perf-frames=600"

# After Phase 4 (allocations):
make run EXAMPLE=hello-widget ARGS="--perf-report=reports/phase4-alloc.json --perf-frames=600"
```

What to check per phase:

- **Phase 1** ✅: Stats panel shows timing breakdown; report file is generated with plausible numbers
- **Phase 2**: avg_frame_us should drop significantly; CPU usage near-zero when idle (`top`/`htop`)
- **Phase 3**: `avg_tree_us` drops to ~0 on animation-only frames; `animation_only_pct` shows what fraction benefits
- **Phase 4**: `avg_layout_us` and `avg_render_us` decrease; fewer allocations visible in `make profile`
- **Phase 5**: Perf overlay works identically in testbed and on-device builds

On the real device (if available): measure FPS and CPU load with `top`.

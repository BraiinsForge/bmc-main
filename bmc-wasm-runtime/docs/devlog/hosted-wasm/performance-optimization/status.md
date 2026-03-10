# BDK-287: WASM Runtime Performance Optimization

## Context

The WASM widget runtime burns excessive CPU on both desktop (fans spin up on Ryzen 9 7950X) and the real Braiins Deck
device (only ~25-30fps for a trivial demo). The testbed runs an uncapped render loop with vsync disabled, renders 4
tiles per frame unconditionally, and the pipeline rebuilds all data structures from scratch every frame. Before we can
optimize effectively, we need proper instrumentation to see where time actually goes.

## Current state

Phases 1–6 complete. Reports and samply profiles collected in `reports/` (per-phase directories).

### Perf report comparison (hello-widget, 600 frames, desktop)

| Metric | 02-cached-tree | 03-reduce-allocs | 04-wasmi-tuning | Δ 03→04 |
| ------ | -------------- | ---------------- | --------------- | ------- |
| FPS    | 465            | 465              | 516             | ~0%\*   |
| Frame  | 2.1ms          | 2.2ms            | 1.9ms           | ~0%\*   |
| WASM   | 2.0ms          | 2.0ms            | 1.8ms           | ~0%\*   |
| Layout | 1.1ms          | 1.1ms            | 1.1ms           | ~0%     |
| Render | 0.4ms          | 0.5ms            | 0.3ms           | ~0%\*   |
| p95    | 2.6ms          | 2.4ms            | 2.1ms           | ~0%\*   |

\*Phase 04 was measured in a different session; the FPS/WASM jump vs Phase 03 is likely due to different system load
conditions, not the config changes. Within-session A/B testing showed \<2% variation.

No `perf.json` for baseline or Phase 2 — the `--perf-report` flag was added as part of those commits.

### Samply profile comparison (inclusive crate breakdown)

| Crate            | 00-baseline | 01-optimise-cpu | 02-cached-tree |
| ---------------- | ----------- | --------------- | -------------- |
| wasmi            | 80.6%       | 80.3%           | 76.0%          |
| bmc_wasm_runtime | 46.5%       | 46.6%           | 43.6%          |
| taffy            | 28.2%       | 27.7%           | 26.3%          |
| cosmic_text      | 21.8%       | 21.9%           | 20.0%          |
| femtovg          | 20.5%       | 21.1%           | 23.3%          |
| alloc            | 14.5%       | 14.3%           | 20.7%          |
| Samples          | 7 822       | 8 255           | 4 765          |

The sample count drop (8 255 → 4 765) reflects less total CPU work after the render loop fix — the profiler captures
fewer samples per unit time because the CPU is idle more often.

### Remaining bottleneck

**wasmi at 76% inclusive** — the WASM interpreter dominates. Host-side optimizations (Phases 2–4) reduced everything
around it, making wasmi an even larger proportion of remaining work. Further host-side gains are diminishing returns.

---

## Phase 1: Instrumentation ✅

`FrameTimings` struct, per-component timing in `process_tree`/`layout_and_render`/`render()`, stacked bar chart stats
panel, `--perf-report=<path> --perf-frames=N` CLI for automated benchmarking.

**Commit:** `500d4c0 wasm: Optimise CPU load #BDK-287` (bundled with Phase 2)

---

## Phase 2: Fix testbed render loop ✅

VSync enabled, idle redraw removed, `frame_delay_ms` respected, per-tile skip when no work pending. Eliminated 100% CPU
spin on desktop.

**Commit:** `500d4c0 wasm: Optimise CPU load #BDK-287`

---

## Phase 3: Cache deserialized tree ✅

`cached_tree: Option<(TreeNode, f32, f32)>` in `HostState`. `render_cached_tree()` calls `layout_and_render()` directly
on the cached `TreeNode` — no clone, no deserialization. `process_tree()` split from `layout_and_render()` in Phase 1b.

**Commit:** `ea7b713 wasm: Cache deserialized tree and restructure reports #BDK-287`

---

## Phase 4: Reduce per-frame allocations ✅

- `TaffyTree` persisted in `HostState`, `clear()` instead of `new()` each frame
- `full_text` and `span_offsets` cached in `ParagraphLayoutEntry` (no per-draw String/Vec)
- Early-return in `lerp_color_oklab`/`lerp_color_oklch`/`lerp_color_linear_rgb` when `from == to` or `t` at boundaries
- Removed dead fields (`forward`, `drag_start_y`, `drag_start_offset`), removed blanket `#[expect(dead_code)]`

**Commit:** `4b226be wasm: Reuse TaffyTree, cache paragraph text, short-circuit color lerp #BDK-287`

---

## Phase 5: wasmi interpreter tuning ✅

**Status:** Benchmarked on desktop. Config tuning applied, no measurable runtime improvement on x86. Needs armv7
validation.

wasmi is 76% inclusive in profiles. The register-based engine (wasmi 1.0.9) is already active — there is no faster
backend to switch to within wasmi.

### Config changes applied

- `set_max_cached_stacks(4)` — cache more execution stacks for reuse across render calls (default: 2)
- Disabled unused Wasm proposals: `tail_call`, `multi_memory`, `memory64`, `extended_const`, `custom_page_sizes`,
  `wide_arithmetic`
- Fuel metering kept enabled — safety is worth the minimal overhead

### Desktop benchmark results (hello-widget, 600 frames)

All config variations produce \<2% difference on desktop (Ryzen 9 7950X) — firmly within noise.

| Variant                        | avg_wasm_us | Δ vs baseline |
| ------------------------------ | ----------- | ------------- |
| Fuel enabled (baseline)        | 1.8ms       | —             |
| Fuel disabled                  | 1.8ms       | ~1% (noise)   |
| Stacks=4                       | 1.8ms       | \<1% (noise)  |
| Stacks=4 + proposals disabled  | 1.8ms       | \<1% (noise)  |
| Full tuning + Lazy compilation | 1.8ms       | \<1% (noise)  |

**Conclusion:** On fast x86 hardware the interpreter is fast enough that config tuning is invisible. The real test is on
the armv7 target device. See `wasmi-tuning.md` for the detailed optimization guide.

### Not pursued (yet)

- **`Lazy` compilation** — skips validation, startup-only benefit, not worth the safety trade-off
- **WAMR AOT** — would require C FFI integration and a build pipeline for `.aot` binaries. Only warranted if armv7 can't
  meet frame budget.

---

## Phase 6: Extract perf overlay as a reusable component ✅

Moved `FpsTracker` → `PerfOverlay`, `FrameSample`, overlay color constants, and all chart/legend drawing from
`testbed.rs` into `src/perf_overlay.rs` behind a `perf-overlay` cargo feature (`testbed` implies it). The testbed's
`draw_stats_panel` is now a thin wrapper: reload button + `overlay.draw()`.

On-device hosts opt in with `features = ["perf-overlay"]` and call `PerfOverlay::new()` / `tick()` / `draw()` — no
testbed deps required. See `perf-overlay-extraction.md` for the full integration guide.

**Commit:** `7f1857d wasm: Extract perf overlay as reusable, feature-gated module #BDK-287`

---

## Tooling

### Profiling

```bash
make profile                           # samply record + addr2line symbolication
make profile ARGS="--perf-report=..."  # samply + internal timing
```

Produces `profile.json.gz` + `symbols.json`. View with `samply load profile.json.gz`.

### Analysis scripts (`tools/`)

- `perf_analyze.py <profile.json.gz>` — crate breakdown + hot functions from samply profile
- `perf_compare.py <dir1> <dir2> ...` — side-by-side perf.json + profile comparison across phases
- `perf_symbolicate.py <profile.json.gz> <binary>` — batch addr2line (run by `make profile`)

### Reports directory

```
reports/
  00-baseline/        — profile.json.gz, symbols.json
  01-optimise-cpu/    — profile.json.gz, symbols.json
  02-cached-tree/     — profile.json.gz, symbols.json, perf.json
  03-reduce-allocs/   — perf.json
  04-fuel-enabled/    — perf.json (fuel benchmark: with fuel)
  04-fuel-disabled/   — perf.json (fuel benchmark: without fuel)
  04-wasmi-tuning/    — perf.json (final: stacks=4, proposals disabled)
```

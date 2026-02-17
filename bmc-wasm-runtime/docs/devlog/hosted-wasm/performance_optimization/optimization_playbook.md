# WASM Widget Runtime — Performance Optimization Playbook

Standalone guide for optimizing a wasmi-based WASM widget runtime that calls a guest `render()` function every frame.
Derived from measured results on a real codebase (Rust host + Rust-compiled WASM widgets, wasmi 1.0, Taffy layout,
FemtoVG/cosmic-text rendering). All recommendations are ordered by measured or expected impact.

**Assumes:** wasmi 1.0.x, frame-by-frame rendering (16–33ms budget), fuel metering for safety, Rust-compiled `.wasm`
widgets with a tree-based UI model.

## TL;DR — what actually moved the needle

| Optimization                               | Impact (desktop x86)     | Effort |
| ------------------------------------------ | ------------------------ | ------ |
| Fix render loop (VSync, skip idle frames)  | Eliminated 100% CPU spin | Low    |
| Cache deserialized tree across frames      | -10% WASM time, -5% p95  | Low    |
| Reuse layout tree allocation across frames | Marginal (within noise)  | Low    |
| Cache shaped text data                     | Marginal (within noise)  | Low    |
| wasmi config tuning (all knobs)            | **Zero** (\<2%, noise)   | Low    |

The host-side render loop and caching changes had real impact. wasmi config tuning had none on desktop. Config tuning
may matter on weaker hardware (armv7) — measure on the target device.

---

## Phase 0: Instrument before optimizing

You cannot optimize what you cannot measure. Add per-component timing before touching anything else.

### 0a. Per-frame timing struct

Track time spent in each pipeline stage separately:

```rust
#[derive(Debug, Clone, Copy, Default)]
pub struct FrameTimings {
    pub wasm_us: u32,        // WASM interpreter execution (outer envelope)
    pub deserialize_us: u32, // binary → tree deserialization
    pub layout_us: u32,      // layout tree build + computation
    pub render_us: u32,      // draw command execution
    pub flush_us: u32,       // GPU flush
}
```

Wrap each stage with `Instant::now()` / `.elapsed().as_micros()`. Store the result in host state so it's accessible for
display and export.

### 0b. Automated benchmark export

Add a `--perf-report=<path> --perf-frames=N` CLI mode that runs N frames, writes a JSON report (averages, percentiles,
per-frame samples), and exits. This enables before/after comparison with a script.

```bash
# Collect baseline
cargo run --release -- --perf-report=reports/00-baseline/perf.json --perf-frames=600
# Apply change, collect again
cargo run --release -- --perf-report=reports/01-change/perf.json --perf-frames=600
# Compare
python3 tools/perf_compare.py reports/00-baseline reports/01-change
```

### 0c. CPU profiling

Use [samply](https://github.com/mstange/samply) for flamegraph-style CPU profiles:

```bash
samply record -- cargo run --release
```

Write a script to parse the samply JSON and produce a crate-level inclusive breakdown. This tells you which crate
dominates CPU time — critical for deciding where to focus.

**Important:** When computing inclusive time per crate from stack samples, deduplicate by crate per stack, not per
function. A stack like `wasmi::engine → wasmi::executor → wasmi::fuel` should count as 1 sample for wasmi, not 3.

---

## Phase 1: Fix the render loop

The single biggest win. A broken render loop can burn 100% CPU even when nothing changes on screen.

### 1a. Enable VSync

```rust
// Before: spins as fast as possible
surface.swap_buffers(SwapInterval::DontWait);

// After: blocks until next vblank (~60fps)
surface.swap_buffers(SwapInterval::Wait(NonZeroU32::new(1).unwrap()));
```

### 1b. Stop redrawing when idle

Only request a redraw when there's actual work: user input, animation tick, data update. Remove any unconditional
`window.request_redraw()` from idle handlers.

### 1c. Respect widget-requested delays

When a widget calls `request_frame_after(delay_ms)`, use that delay for the next wake time instead of polling
continuously. Compute the earliest pending delay across all active widgets.

### 1d. Skip unchanged tiles

If you render multiple independent tiles/widgets, check each one for pending work before rendering it. A tile with no
pending events, no active animations, and no data changes can be skipped — the previous frame's output is still valid in
the FBO.

---

## Phase 2: Cache across frames

Avoid re-doing work that hasn't changed since the last frame.

### 2a. Cache the deserialized tree

If your pipeline is `binary → deserialize → layout → render`, cache the deserialized tree and skip deserialization on
frames where only animations need updating (no new WASM execution).

```rust
// In host state:
pub cached_tree: Option<(TreeNode, f32, f32)>,
pub animation_only_frame: bool,

// On full WASM frame: cache the tree after processing
state.cached_tree = Some((tree_node, width, height));

// On animation-only frame: reuse cached tree, skip WASM + deserialize
if animation_only && cached_tree.is_some() {
    layout_and_render(cached_tree, ...);  // no WASM call, no deserialization
    return;
}
```

Split your processing pipeline into `process_tree()` (full: deserialize + layout + render) and `layout_and_render()`
(partial: layout + render from existing tree).

### 2b. Reuse layout tree allocations

If you use a layout library (Taffy, Yoga, etc.), persist the tree object across frames and call `clear()` instead of
allocating a new one each frame. Most layout libraries keep internal allocations on `clear()`.

```rust
// In host state (initialized once):
pub taffy: TaffyTree<NodeContext>,

// Each frame:
taffy.clear();  // resets tree, keeps Vec allocations
let root = build_taffy_node(&mut taffy, tree_node)?;
taffy.compute_layout(root, available_space)?;
```

### 2c. Cache shaped text

Text shaping (cosmic-text, fontdue, etc.) is expensive. Cache the shaped buffer keyed by
`(text_content, font_properties, max_width)`. Also cache any derived data (concatenated full text, span byte offsets) in
the same cache entry so drawing doesn't re-derive them.

### 2d. Short-circuit interpolation

For animation interpolation functions (color lerp, etc.), add early returns:

```rust
fn lerp_color(from: u32, to: u32, t: f32) -> u32 {
    if from == to || t <= 0.0 { return from; }
    if t >= 1.0 { return to; }
    // ... expensive color space conversion ...
}
```

---

## Phase 3: wasmi configuration tuning

These are all zero-cost to apply but **produced no measurable impact on desktop x86** (\<2% variation, within noise).
Apply them anyway — they're free and may help on weaker hardware.

### 3a. Stack caching

```rust
config.set_max_cached_stacks(4);  // default is 2
```

Avoids reallocating the value stack on each `render_func.call()`. The effect scales with call frequency.

### 3b. Disable unused Wasm proposals

```rust
config.wasm_tail_call(false);
config.wasm_multi_memory(false);
config.wasm_memory64(false);
config.wasm_extended_const(false);
config.wasm_custom_page_sizes(false);
config.wasm_wide_arithmetic(false);

// Keep enabled: bulk_memory, reference_types, mutable_globals, sign_extension,
// saturating_float_to_int, multi_value (used by standard Rust-compiled Wasm)
```

Note: `wasm_simd()` and `wasm_relaxed_simd()` are behind a compile-time feature gate in wasmi 1.0.9.

### 3c. Fuel metering

Fuel metering adds per-instruction overhead. On desktop x86 it was \<1% (within noise). On ARM it may be more
significant. Benchmark with `config.consume_fuel(false)` and compare `avg_wasm_us`. If >5%, consider making fuel
configurable (enable for untrusted widgets, disable for trusted first-party ones).

Guard all `store.set_fuel()` calls when fuel is disabled — they'll error if the engine wasn't configured with
`consume_fuel(true)`.

### 3d. Lazy compilation (startup only)

```rust
config.compilation_mode(wasmi::CompilationMode::Lazy);
```

Defers both validation and translation. Up to 27x faster startup. Trade-off: malformed functions that are never called
won't be detected. Only safe for trusted first-party widgets.

---

## Phase 4: Optimize the Wasm binary itself

The widget's Rust code compiles to Wasm bytecode that wasmi interprets. Fewer instructions = faster interpretation.

### 4a. Cargo release profile (widget crate)

```toml
[profile.release]
opt-level = "z"     # size-optimized: fewer instructions for interpreter
lto = true          # cross-crate dead code elimination
codegen-units = 1   # maximum optimization opportunity
panic = "abort"     # no unwinding overhead
strip = true        # smaller binary
```

`opt-level = "z"` often produces faster-interpreting code than `opt-level = 3` because it avoids loop unrolling and
excessive inlining that helps native CPUs but just means more bytecode for an interpreter.

### 4b. Post-process with wasm-opt (binaryen)

```bash
wasm-opt -O3 -o optimized.wasm input.wasm
wasm-opt --metrics input.wasm  # check instruction counts
```

This runs Wasm-level optimizations (constant folding, dead code elimination, block merging) that LLVM's Wasm backend
doesn't always catch.

### 4c. Profile the widget code

If the widget is doing unnecessary work per frame (allocating strings, rebuilding data structures, recomputing unchanged
values), no amount of interpreter tuning helps. Profile the WASM execution itself — look at which host functions are
called most frequently and whether the widget re-derives data it could cache.

---

## Phase 5: Alternative runtimes (major change)

Only pursue this if the above is insufficient on the target device.

| Runtime              | Type                 | armv7        | Speedup vs wasmi | Effort           |
| -------------------- | -------------------- | ------------ | ---------------- | ---------------- |
| **wasm3**            | C interpreter        | Yes          | ~comparable      | Moderate (C FFI) |
| **WAMR interpreter** | C interpreter        | Yes          | ~1.5-2x          | Moderate (C FFI) |
| **WAMR AOT**         | Ahead-of-time native | Yes          | ~10-50x          | High             |
| **wasmtime**         | JIT (Cranelift)      | aarch64 only | ~10-100x         | N/A for armv7    |

**WAMR AOT** is the only path to dramatically faster execution on armv7. Requires:

1. Compiling each `.wasm` to a native `.aot` binary using `wamrc` during build/submission
2. Integrating WAMR's C runtime via FFI (replacing wasmi entirely)
3. Maintaining host function bindings in C or via `wamr-rust-sdk`

---

## Measurement checklist

Before and after each change, collect:

- `avg_wasm_us` — interpreter time per frame (**primary metric**)
- `avg_frame_us` — end-to-end frame time
- `p95_frame_us` — tail latency (sensitive to GC pauses, translation stalls, allocation spikes)
- CPU profile (samply) — crate-level inclusive breakdown

Run the same widget, same frame count, same conditions. Compare reports side-by-side. Ignore \<3% differences — that's
noise. Only trust measurements from the **target device**, not a desktop proxy.

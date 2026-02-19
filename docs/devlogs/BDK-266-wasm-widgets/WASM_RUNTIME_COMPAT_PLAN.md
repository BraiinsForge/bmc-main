# Plan: Update wayland widget host for current bmc-wasm-runtime

The `bmc-wasm-runtime` on `jku/wasm` has had ~11 commits of API changes and
performance optimizations since the wayland widget host was written. This plan
describes the minimal changes needed on `jca/BDK-266/wasm-runtime-wayland-deck`
so that `.wasm` files built against the current SDK can run on the real device.

**Constraint:** The branches stay separate. `jku/wasm` tracks `master` and must
not be rebased onto corinthia. The runtime is consumed via the existing
`path = "../../bmc-wasm-runtime"` dependency, resolved at build time through a
symlink.

______________________________________________________________________

## 1. Update workspace `Cargo.toml` dependencies

File: `/Cargo.toml` (repo root)

The runtime now uses `wasmi 1.0` (was `0.45`) and requires a few workspace deps
that the wayland workspace doesn't have yet.

### 1a. Bump existing deps

```
# BEFORE
wasmi = "0.45"

# AFTER
wasmi = "1.0"
```

### 1b. Add missing workspace deps

These are used by `bmc-wasm-runtime/Cargo.toml` with `.workspace = true` but
don't exist in the wayland workspace yet:

```toml
imgref = "1.10"
rgb = "0.8"
ureq = "3"
usvg = "0.45"          # used by bmc-icon-compiler (build dep)
```

> `formato`, `image`, `notify`, `chrono`, `serde_json` are already present in
> the wayland workspace — just verify versions match.

### 1c. Regenerate `Cargo.lock`

After the above changes, run `cargo check` to regenerate the lock file. This is
more reliable than `cargo update -p` when new workspace deps have been added:

```bash
cargo check -p bmc-widget-wasm
```

______________________________________________________________________

## 2. Update `widgets/wasm/src/wayland.rs`

### 2a. Add imports

```rust
use bmc_wasm_runtime::RenderStatus;
use bmc_wasm_protocol::FormatPreferences;
```

The `bmc_wasm_protocol` crate is pulled in transitively via `bmc-wasm-runtime`,
but for `FormatPreferences` you need to reference it directly. Add to
`widgets/wasm/Cargo.toml`:

```toml
bmc-wasm-protocol = { path = "../../bmc-wasm-runtime/protocol" }
```

Or alternatively, since `FormatPreferences` implements `Default`, you can avoid
the extra dep:

Just pass `FormatPreferences::default()` inline — it gives Metric / Celsius / SpaceComma.

### 2b. Constructor signature change

The `WasmWidgetRuntime::new()` constructor now requires two additional parameters:

| Parameter        | Type                | Purpose                                                                             |
| ---------------- | ------------------- | ----------------------------------------------------------------------------------- |
| `fuel_per_frame` | `u64`               | Instruction budget per frame (use `WasmWidgetRuntime::FUEL_PER_FRAME` = 10,000,000) |
| `prefs`          | `FormatPreferences` | Number/unit/temperature formatting prefs                                            |

**Location:** `run_wasm_mode()`, around line 324.

BEFORE (line 324-332):

```
let mut runtime = unsafe {
    WasmWidgetRuntime::new(
        &wasm_bytes,
        |symbol| smithay::backend::egl::get_proc_address(symbol),
        self.state.width,
        self.state.height,
        fbo_id,
    )?
};
```

AFTER:

```
let mut runtime = unsafe {
    WasmWidgetRuntime::new(
        &wasm_bytes,
        |symbol| smithay::backend::egl::get_proc_address(symbol),
        self.state.width,
        self.state.height,
        fbo_id,
        WasmWidgetRuntime::FUEL_PER_FRAME,
        FormatPreferences::default(),
    )?
};
```

> Later, `FormatPreferences` can be wired to device settings via `DECK_PARAMS`
> JSON, but `default()` (Metric/Celsius/SpaceComma) is fine for now.

### 2c. Handle `render()` return type

`render()` now returns `Result<RenderStatus>` instead of `Result<()>`.
`RenderStatus` is:

```rust
pub enum RenderStatus {
    Ok,             // frame rendered within fuel budget
    FuelExhausted,  // over budget this frame, last good frame shown
    Dead,           // killed after repeated overages, error overlay shown
}
```

**First frame** (around line 355):

BEFORE: `runtime.render(0)?;`

AFTER:

```
match runtime.render(0)? {
    RenderStatus::Ok => {}
    status => tracing::warn!("First frame render status: {status:?}"),
}
```

**Main loop** (around line 410):

BEFORE:

```
if let Err(e) = runtime.render(delta_ms) {
    tracing::error!("WASM render error: {}", e);
}
```

AFTER:

```
match runtime.render(delta_ms) {
    Ok(RenderStatus::Ok) => {}
    Ok(RenderStatus::FuelExhausted) => {
        tracing::warn!("Widget exceeded fuel budget");
    }
    Ok(RenderStatus::Dead) => {
        tracing::error!("Widget killed (repeated fuel overages), exiting");
        self.state.running = false;
    }
    Err(e) => {
        tracing::error!("WASM render error: {e}");
    }
}
```

> When a widget enters `Dead` state, the runtime has already rendered the error
> overlay to the current frame. We set `running = false` so the process exits
> cleanly after committing this last frame. The compositor can then respawn the
> widget if configured to do so.

### 2d. Add fetch response delivery

Widgets using `fetch()` (e.g. ISS Position) need their HTTP responses delivered
before each render call. Without this, fetch-based widgets hang forever.

**Before the `runtime.render()` call in the main loop** (~line 409), add:

```
runtime.deliver_fetch_responses();
```

> The first-frame block does not need this — no fetches have been initiated yet
> because `render()` hasn't been called (the first `render(0)` is what triggers
> widget init, which is where `fetch()` calls are registered).

### 2e. Frame scheduling: delay + pending fetches

The widget can request a delayed frame via `request_frame_after(ms)` instead of
immediate vsync. On embedded hardware this prevents busy-looping at 60fps when
a widget only needs updates every few seconds.

Additionally, `has_pending_fetches()` must be checked — a widget that only uses
`host_fetch_after()` without separately calling `request_frame()` would never
get its delayed fetches fired, because `deliver_fetch_responses()` (which drains
the delayed fetch queue) only runs inside the render block.

**After the `wants_next_frame()` check** (around line 449):

BEFORE:

```
if runtime.wants_next_frame() {
    self.state.needs_render = true;
}
```

AFTER:

```
if runtime.wants_next_frame() || runtime.has_pending_fetches() {
    if let Some(delay_ms) = runtime.next_frame_delay() {
        // Cap sleep to 100ms so Wayland events (shutdown, buffer release,
        // resize) are still processed promptly. The remaining delay is
        // handled by re-entering the dispatch loop.
        let capped = delay_ms.min(100);
        std::thread::sleep(std::time::Duration::from_millis(u64::from(capped)));
    }
    self.state.needs_render = true;
}
```

> **Why cap the sleep?** An unbounded `thread::sleep(30_000)` (e.g. ISS Position
> polling every 30s) would block the entire Wayland event loop — compositor
> shutdown events, buffer releases, and resizes would all stall. The 100ms cap
> keeps the loop responsive while still avoiding busy-looping.
>
> A more sophisticated approach would use `calloop` (already a workspace dep)
> with a timerfd instead of sleeping the event loop thread at all.

______________________________________________________________________

## 3. Update `deploy_corinthia.py`

The deploy script currently hardcodes an external path to find `.wasm` files:

```
BDK266_ROOT = PROJECT_ROOT.parent.parent / "jku" / "BDK-266-wasm"
```

Since the runtime is now symlinked into the repo root, update to:

```
BDK266_ROOT = PROJECT_ROOT / "bmc-wasm-runtime"
```

The `WASM_ASSETS` list can stay as-is — it builds the path to
`examples/hello-widget/target/wasm32-unknown-unknown/release/hello_widget.wasm`
from `BDK266_ROOT`.

> **Note:** The symlink currently points to
> `../BDK-266-wasm-runtime-gpu-fb-fix/bmc-wasm-runtime/`. The `.wasm` example
> must be built **inside the symlink target**, not in some other checkout. See
> section 7 for the build command.

______________________________________________________________________

## 4. Summary of files changed

| File                          | Change                                                                               |
| ----------------------------- | ------------------------------------------------------------------------------------ |
| `Cargo.toml` (root)           | `wasmi` 0.45→1.0, add `imgref`, `rgb`, `ureq`, `usvg`                                |
| `widgets/wasm/Cargo.toml`     | Add `perf-overlay` feature, (optional) `bmc-wasm-protocol` path dep                  |
| `widgets/wasm/src/wayland.rs` | Constructor args, `RenderStatus` + Dead exit, fetch delivery, frame delay + pending fetches, perf overlay |
| `deploy_corinthia.py`         | Fix `BDK266_ROOT` path to use symlink                                                |
| `Cargo.lock`                  | Regenerated                                                                          |

______________________________________________________________________

## 5. Enable the performance overlay

The runtime includes a reusable `PerfOverlay` module (feature-gated behind
`perf-overlay`) that draws a stacked timing chart directly on top of the widget.
This is extremely useful for on-device profiling — it shows per-frame breakdowns
of WASM execution, tree deserialization, layout, rendering, and GPU flush.

### 5a. Enable the cargo feature

In `widgets/wasm/Cargo.toml`, enable the feature:

BEFORE:

```toml
bmc-wasm-runtime = { path = "../../bmc-wasm-runtime" }
```

AFTER:

```toml
bmc-wasm-runtime = { path = "../../bmc-wasm-runtime", features = ["perf-overlay"] }
```

### 5b. Add overlay to the render loop

In `widgets/wasm/src/wayland.rs`, add imports and state:

```rust
use bmc_wasm_runtime::FrameTimings;
use bmc_wasm_runtime::perf_overlay::PerfOverlay;
```

Add `PerfOverlay` and env-var toggle to `run_wasm_mode()` local state (after
runtime creation):

```
let show_perf = std::env::var("DECK_PERF_OVERLAY").is_ok_and(|v| v == "1" || v == "true");
let mut perf_overlay = PerfOverlay::new();
```

### 5c. Instrument the render loop

> **Important:** This section is a **complete replacement** for the main-loop
> render block. It supersedes the changes from sections 2c and 2d for the main
> loop. Do not apply 2c/2d main-loop changes separately if you are also applying
> section 5. (The first-frame changes from 2c still apply independently.)

The overlay needs to be ticked every loop iteration (not just rendered frames)
and drawn after the WASM render but before flush.

**Replace the main-loop render block** (the `begin_frame` through `flush`
sequence, around lines 407-413) with:

```
let frame_start = std::time::Instant::now();

runtime.deliver_fetch_responses();
runtime
    .renderer()
    .begin_frame(self.state.width, self.state.height);

let rendered = match runtime.render(delta_ms) {
    Ok(RenderStatus::Ok) => true,
    Ok(RenderStatus::FuelExhausted) => {
        tracing::warn!("Widget exceeded fuel budget");
        true
    }
    Ok(RenderStatus::Dead) => {
        tracing::error!("Widget killed (repeated fuel overages), exiting");
        self.state.running = false;
        true
    }
    Err(e) => {
        tracing::error!("WASM render error: {e}");
        false
    }
};

if show_perf {
    let frame_us = frame_start.elapsed().as_micros() as u32;
    let timings = runtime.last_timings();
    perf_overlay.tick(frame_us, rendered, timings);
    perf_overlay.draw(
        runtime.renderer(),
        self.state.width as f32,
        self.state.height as f32,
        0.0,
    );
}

runtime.renderer().flush();
```

> Note: `begin_frame` and `flush` are already present in the original code.
> This block replaces them — do not duplicate.

### 5d. What the overlay shows

```
┌─────────────────────────────────┐
│ 4.2ms  30fps                    │  ← avg frame time + rendered FPS
│ ┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃  │  ← 120-frame stacked bar chart
│ ┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃  │     (height = frame time)
│ ┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃┃  │     gridlines at 4ms, 8ms, 16ms
│ WASM 2.1 Tree 0.3 Lay 0.2 ...  │  ← legend with avg per-component ms
└─────────────────────────────────┘
```

Bar colors:

- **Blue** — WASM interpreter execution
- **Orange** — tree deserialization
- **Yellow** — Taffy layout computation
- **Green** — tree rendering (draw calls)
- **Purple** — GPU flush
- **Gray** — frames where WASM was skipped (animation-only cached frames)

### 5e. `FrameTimings` struct

```rust
pub struct FrameTimings {
    pub wasm_us: u32,         // outer WASM execution envelope
    pub deserialize_us: u32,  // tree deserialization from wire format
    pub layout_us: u32,       // Taffy flexbox layout
    pub render_us: u32,       // render_taffy_node + modal rendering
    pub flush_us: u32,        // FemtoVG canvas.flush()
}
```

Access via `runtime.last_timings()` after each `render()` call.

### 5f. Toggle via environment variable

The `show_perf` flag (declared in 5b) and the `if show_perf` gate (in 5c)
make the overlay opt-in at runtime without recompilation.

Enable on device with: `DECK_PERF_OVERLAY=1`

______________________________________________________________________

## 6. What does NOT need changing (transparent to host)

All internal runtime optimizations are transparent to the host:

- Cached tree / TaffyTree reuse
- Animation-only frame skipping
- Color lerp short-circuit
- Paragraph text caching
- wasmi config tuning (disabled unused wasm proposals)
- Path drawing (DRAW_PATH wire command)
- ButtonSize support (protocol change, handled in tree.rs)
- `host_log` / `host_format_*` host functions (registered internally in `new()`)

______________________________________________________________________

## 7. Testing

1. Build a `.wasm` via the symlink (resolves to the `BDK-266-wasm-runtime-gpu-fb-fix`
   checkout). The build must happen inside this path so `deploy_corinthia.py` can
   find the output artifact:

   ```bash
   cd bmc-wasm-runtime/examples/hello-widget
   cargo build --target wasm32-unknown-unknown --release
   ```

   Verify the artifact exists at
   `bmc-wasm-runtime/examples/hello-widget/target/wasm32-unknown-unknown/release/hello_widget.wasm`.

1. Deploy to device using the updated `deploy_corinthia.py`

1. Verify widget renders correctly and logs show:

   - `WASM runtime initialized, SDK version X.Y.Z`
   - No `FuelExhausted` warnings on normal widgets
   - HTTP-fetching widgets (ISS Position) receive data

1. Test `Dead` state recovery: verify that a fuel-exhausted widget exits
   cleanly (process exits after rendering the error overlay frame)

______________________________________________________________________

## Reference: current `WasmWidgetRuntime` public API

```
WasmWidgetRuntime::new(wasm_bytes, load_fn, width, height, fbo_id, fuel_per_frame, prefs)
WasmWidgetRuntime::FUEL_PER_FRAME  -> u64 = 10_000_000

render(delta_ms)               -> Result<RenderStatus>     # CHANGED return type
deliver_fetch_responses()                                   # NEW — call before render
renderer()                     -> &mut FemtoVgRenderer

wants_next_frame()             -> bool
next_frame_delay()             -> Option<u32>               # NEW
has_pending_fetches()          -> bool                      # NEW

reset_fuel_state()                                          # NEW — revive dead widget
sdk_version()                  -> (u16, u16, u16)
last_timings()                 -> FrameTimings              # NEW — for perf overlay
```

```
RenderStatus::Ok              — frame rendered within fuel budget
RenderStatus::FuelExhausted   — over budget, last good frame shown
RenderStatus::Dead            — killed after repeated overages
```

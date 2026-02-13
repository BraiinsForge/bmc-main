# BDK-292: Test host fuel-limiter & graceful degradation

## Context

The WASM widget runtime has fuel metering enabled (wasmi `consume_fuel`), but when a widget exhausts its budget the
error just bubbles up as a generic anyhow error logged to stderr. There's no graceful degradation, no visual feedback,
and no way for the host to distinguish fuel exhaustion from other crashes. This task creates a stress-test widget, makes
the fuel budget configurable, and adds a degradation path so blown widgets don't just disappear.

## Scope (POC quality — BDK-266 exploration)

### 1. Make fuel budget a constructor parameter

**File:** `bmc-wasm-runtime/src/runtime.rs`

- Add `fuel_per_frame: u64` field to `WasmWidgetRuntime`
- Add it as a parameter to `new()` (after `fbo_id`)
- Keep `FUEL_PER_FRAME` as the default constant
- Use `self.fuel_per_frame` everywhere `Self::FUEL_PER_FRAME` is currently used (lines 109, 728, 833, 864)
- Update testbed call site to pass the constant

### 2. Detect fuel exhaustion & add render status

**File:** `bmc-wasm-runtime/src/runtime.rs`

- Change `render()` return type from `Result<()>` to `Result<RenderStatus>`
- Add enum:
  ```
  pub enum RenderStatus {
      Ok,
      FuelExhausted,
  }
  ```
- In the full-frame branch (line 726-729), catch the wasmi error:
  - `error.as_trap_code() == Some(TrapCode::OutOfFuel)` → return `Ok(RenderStatus::FuelExhausted)` (fall back to cached
    tree render if available)
  - Other errors → propagate as before
- On `FuelExhausted`: if `cached_tree_data` exists, re-render from cache (same path as animation-only) so the last good
  frame shows

### 3. Add strike counter & dead state to the runtime

**File:** `bmc-wasm-runtime/src/runtime.rs`

- Add fields to `WasmWidgetRuntime`:
  - `fuel_strikes: u32` (consecutive fuel-out count)
  - `fuel_dead: bool` (widget killed after N strikes)
  - `max_fuel_strikes: u32` (configurable, default 5)
- In `render()`:
  - If `fuel_dead` → skip WASM, render cached frame + error overlay, return `RenderStatus::Dead`
  - On `FuelExhausted` → increment `fuel_strikes`, if >= max → set `fuel_dead = true`
  - On successful render → reset `fuel_strikes` to 0
- Add `RenderStatus::Dead` variant
- Add `pub fn reset_fuel_state(&mut self)` → resets strikes and dead flag (for testbed reset)

### 4. Draw error overlay on fuel exhaustion

**File:** `bmc-wasm-runtime/src/runtime.rs` (inside `render()`)

When rendering a degraded/dead frame, use the host renderer directly to draw a simple overlay:

- Semi-transparent dark scrim over the last-good-frame
- "Budget exceeded" text centered (use existing `renderer.draw_text()`)
- For `FuelExhausted` (not yet dead): subtle indicator — small red bar at top edge
- For `Dead`: full overlay with message

This is host-side drawing, no WASM involved — uses the same `FemtoVgRenderer` the runtime owns.

### 5. Create stress-test widget

**New directory:** `bmc-wasm-runtime/examples/stress-test/`

Files:

- `Cargo.toml` — same pattern as hello-widget, depends on `bmc-wasm-sdk`
- `src/lib.rs` — widget with mode toggle via button clicks:
  - **Mode 0 (Normal):** Simple render, well within budget — baseline
  - **Mode 1 (CPU burn):** Tight computation loop (busy math in a `for` loop) that exceeds fuel
  - **Mode 2 (Draw spam):** Thousands of `rect()` draw calls in a canvas that exhaust fuel
  - Display current mode name + button to cycle modes
  - Each frame shows fuel mode label so the testbed user knows what's active

### 6. Testbed: handle RenderStatus + reset

**File:** `bmc-wasm-runtime/src/bin/testbed.rs`

- Update render loop (line 479) to match on `RenderStatus` instead of just logging errors:
  - `Ok(RenderStatus::Ok)` → normal
  - `Ok(RenderStatus::FuelExhausted)` → log warning with tile label
  - `Ok(RenderStatus::Dead)` → log once, stop requesting frames for that tile
  - `Err(e)` → log error as before
- On hot-reload (line 438-463): call `reset_fuel_state()` on all tiles — this is the natural reset point when the user
  changes stress mode and saves, the widget reloads and resets

## Files to modify

| File                                               | Change                                                              |
| -------------------------------------------------- | ------------------------------------------------------------------- |
| `bmc-wasm-runtime/src/runtime.rs`                  | Fuel param, RenderStatus, strike logic, error overlay, reset method |
| `bmc-wasm-runtime/src/bin/testbed.rs`              | Handle RenderStatus, reset on reload                                |
| `bmc-wasm-runtime/examples/stress-test/Cargo.toml` | **New** — widget crate                                              |
| `bmc-wasm-runtime/examples/stress-test/src/lib.rs` | **New** — stress test widget                                        |

## Verification

1. Build stress-test widget: `cargo build -p stress-test --target wasm32-unknown-unknown`
2. Run testbed with it: `cargo run -p bmc-wasm-runtime --bin testbed -- path/to/stress_test.wasm`
3. Click through modes in the widget:
   - Normal mode renders fine
   - CPU burn mode triggers fuel exhaustion → last good frame + red bar → after 5 frames → dead overlay
   - Draw spam mode same behavior
4. Save the widget source (trigger hot-reload) → dead state resets, widget recovers
5. Verify non-stress widgets (hello-widget, spacex-launch) are unaffected

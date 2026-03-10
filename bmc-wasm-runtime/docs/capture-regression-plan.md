# BDK-331: Headless Screenshot Capture & Visual Regression Testing

## Context

The WASM widget runtime renders widgets via FBOs + FemtoVG but has no way to export rendered frames as images. Visual
regressions can only be caught manually. We need a dedicated capture host that renders widgets deterministically and
exports frames as PNGs, plus odiff integration for automated baseline comparison.

## Key Design Decisions

1. **New binary** (`capture`) — not bolting onto testbed. Capture is single-purpose: load WASM, render one size, save
   frames, exit.

2. **One size per invocation** — the binary renders at a single resolution. Makefile loops over sizes. WASM compilation
   and binary build happen once.

3. **Host-provided time** — the runtime never calls `Instant::now()` or `Local::now()` for frame logic. `HostState`
   exposes `system_time` and `monotonic_ms` that the host sets before each `render()`. Testbed sets from real clocks.
   Capture binary increments by fixed 16ms. Runtime code identical in both cases.

4. **Widget-colocated fixtures** — widgets can ship a `fixtures/` directory with pre-recorded fetch responses. The
   capture host serves from fixtures by default when present. Escape hatch: `--live-network`. Recording:
   `--record-fixtures` captures real responses to the fixtures dir.

5. **Widget-colocated capture config** (`capture.toml`) — widget author can specify: settle delay (frames to wait after
   I/O), explicit capture frames, start time, sizes to skip, timeout override. Loaded automatically from widget
   directory. CLI flags override config values.

6. **Auto-readiness via I/O completion** — no new host API needed. The capture host renders frames until all I/O
   resolves (fetches, websockets, sockets), then waits `settle_delay` additional frames (from config, default 0), then
   captures. Widget can keep requesting frames — irrelevant. With fixtures, readiness is reached within 2 frames.

7. **Feature reorganization** — extract GL windowing deps into shared `host` feature.

8. **odiff from nixpkgs** — shell script in `tools/` for comparison orchestration.

9. **Always xvfb-run** in Makefile capture targets.

## Output directory structure

```
captures/<widget_name>/<size>/frame_0001.png
```

Sizes: `full` (1280x480), `large` (638x480), `medium` (638x238), `small` (317x238).

## Files to modify/create

| Action | File                          | Purpose                                                   |
| ------ | ----------------------------- | --------------------------------------------------------- |
| Create | `src/bin/capture.rs`          | New capture binary                                        |
| Create | `tools/regression_compare.sh` | odiff comparison wrapper                                  |
| Create | `captures/.gitignore`         | Ignore current/diff dirs                                  |
| Modify | `Cargo.toml`                  | Feature reorg + new `[[bin]]`                             |
| Modify | `Makefile`                    | New targets                                               |
| Modify | `src/host_api.rs`             | Host-provided time fields                                 |
| Modify | `src/runtime_wasmi.rs`        | Use host-provided time; fixture intercept in `host_fetch` |
| Modify | `src/bin/testbed.rs`          | Set time fields before render()                           |

All paths relative to `bmc-wasm-runtime/`.

---

## Step 1: Host-Provided Time

**File: `src/host_api.rs`**

Add to `HostState`:

```rust
/// Current wall-clock time, set by host before each render().
/// Used by host_get_system_time() — runtime never calls Local::now().
pub system_time: chrono::DateTime<chrono::Local>,

/// Monotonic clock in ms, set by host before each render().
/// Used for deferred timer checks and wasm_delta.
pub monotonic_ms: u64,

/// Monotonic ms at last full WASM render (replaces last_wasm_render_at: Instant).
pub last_wasm_render_at_ms: u64,
```

Change `deferred_wasm_render_at: Option<Instant>` -> `deferred_wasm_render_at_ms: Option<u64>`.

Remove `last_wasm_render_at: Instant`.

**File: `src/runtime_wasmi.rs`**

- `host_get_system_time` (line 654): `let now = caller.data().system_time;`
- `host_request_frame_after` (line 290):
  `state.deferred_wasm_render_at_ms = Some(state.monotonic_ms + u64::from(delay_ms));`
- `render()` (line 1789): `if state.monotonic_ms >= deadline_ms`
- `render()` (lines 1806-1808):
  `let wasm_delta = (state.monotonic_ms - state.last_wasm_render_at_ms) as u32; state.last_wasm_render_at_ms = state.monotonic_ms;`
- `deliver_fetch_responses` (line 2003): `let now_ms = self.store.data().monotonic_ms;` then compare `df.fire_at_ms`
  instead of `Instant`
- Keep `Instant`-based timing only for perf measurement (`wasm_t0.elapsed()` line 1815) — measures real CPU time.

**File: `src/bin/testbed.rs`**

Before tile render calls in `render_preview()`:

```rust
let host = tile.runtime.host_state_mut();
host.system_time = Local::now();
host.monotonic_ms = state.start_instant.elapsed().as_millis() as u64;
```

Add `start_instant: Instant` to `PreviewState`, initialized in `init()`.

Need to verify/add `host_state_mut()` accessor on `WasmWidgetRuntime`.

## Step 2: Fixture System

**File: `src/runtime_wasmi.rs`**

Add to `HostState`:

```rust
/// Fixture responses keyed by "METHOD URL".
/// When present, host_fetch serves from here instead of network.
pub fixtures: Option<HashMap<String, FixtureResponse>>,

/// If set, record real fetch responses to this directory for fixture generation.
pub fixture_record_dir: Option<PathBuf>,
```

```rust
pub struct FixtureResponse {
    pub status: u32,
    pub body: Vec<u8>,
}
```

In `host_fetch` (line 707), before the `thread::spawn`:

```rust
let fixture_key = format!("{method} {url}");
if let Some(fixtures) = &state.fixtures {
    if let Some(fixture) = fixtures.get(&fixture_key) {
        let _ = tx.send(CompletedFetch {
            request_id,
            status: fixture.status,
            body: fixture.body.clone(),
        });
        return request_id;
    }
}
// ... existing thread::spawn for real network
```

For recording, wrap the real `do_fetch` result: after the fetch completes, if `fixture_record_dir` is set, write the
response to a JSON file.

**Fixture file format**: `examples/<widget>/fixtures/fetch_responses.json`

```json
{
  "GET https://api.example.com/data": {
    "status": 200,
    "body_base64": "eyJpdGVtcyI6IFtdfQ=="
  },
  "POST https://api.example.com/auth": {
    "status": 200,
    "body_base64": "eyJ0b2tlbiI6ICJhYmMifQ=="
  }
}
```

Base64 for body to handle binary responses. Load on startup in capture binary.

**Fixture loading** — the capture binary:

```rust
fn load_fixtures(wasm_path: &Path) -> Option<HashMap<String, FixtureResponse>> {
    // Walk up from WASM file to find fixtures/fetch_responses.json
    // Same pattern as seed_kv_from_secrets (testbed.rs line 1219)
}
```

## Step 3: Widget Capture Config

**File format**: `examples/<widget>/capture.toml` (colocated with widget source)

```toml
# Optional — all fields have defaults

# Frames to wait after I/O settles before capturing (default: 0)
settle_delay = 5

# Explicit frame numbers to capture (overrides auto-settlement)
# frames = [60, 120]

# Override start time for deterministic rendering
# time = "2026-06-15T14:30:00"

# Sizes to skip (not meaningful for this widget)
# skip_sizes = ["small"]

# Settlement timeout in frames (default: 300)
# timeout = 500
```

**Loading**: capture binary walks up from WASM path to find `capture.toml` (same pattern as `secrets.ini` in testbed.rs
line 1219). Parsed with basic TOML parsing — add `toml` crate as optional dep under `capture` feature, or hand-parse the
simple key-value format.

**Precedence**: CLI flags > `capture.toml` > defaults. E.g., `--frames=1,30` overrides config's `frames` field.

**In Makefile**: the `capture` target reads `skip_sizes` from config to skip irrelevant size invocations, or the capture
binary itself skips and exits 0 when told to render a skipped size.

## Step 4: Feature Reorganization

**File: `Cargo.toml`**

```toml
[features]
default = []
perf-overlay = []
host = ["glutin", "glutin-winit", "raw-window-handle", "tracing-subscriber", "winit"]
testbed = ["host", "perf-overlay", "notify"]
capture = ["host"]

[[bin]]
name = "testbed"
path = "src/bin/testbed.rs"
required-features = ["testbed"]

[[bin]]
name = "capture"
path = "src/bin/capture.rs"
required-features = ["capture"]
```

## Step 5: Capture Binary

**File: `src/bin/capture.rs`** (~250 lines)

CLI:

```
capture <wasm_file> --size=<WxH> --output=<dir>
    [--frames=1,30,60]
    [--time=2026-01-15T12:00:00]
    [--live-network]
    [--record-fixtures]
```

- `--size=1280x480` — render resolution (required)
- `--output=<dir>` — output directory for PNGs
- `--frames=1,30,60` — explicit frame numbers to capture (overrides auto-settlement)
- `--time=...` — start time for `system_time` (default: `2026-01-01T12:00:00`)
- `--live-network` — bypass fixtures, use real network
- `--record-fixtures` — make real requests AND save responses to widget's `fixtures/` dir

### Auto-readiness (default when `--frames` not specified)

Readiness = all initial I/O resolved. Independent of whether widget keeps requesting frames (clocks, animations, etc.
always do).

```rust
let ready = frame_count > 0
    && !runtime.has_pending_fetches()
    && !runtime.has_active_websockets()
    && !runtime.has_active_sockets()
    && !runtime.has_active_mdns_browses()
    && !runtime.has_active_ssdp_searches();
```

Once I/O settles, wait `settle_delay` additional frames (from `capture.toml`, default 0), then capture. With fixtures,
I/O readiness is frame 2 at latest. Without fixtures, waits for real network.

Safety timeout: `timeout` frames from config (default 300, ~5s virtual). If not ready by then, capture anyway and warn.

### Frame loop

```
1. Parse args, load fixtures if present and --live-network not set
2. Create glutin window (sized to WxH, ControlFlow::Poll, no vsync)
3. Create FBO at WxH with stencil
4. Create WasmWidgetRuntime with fixtures loaded
5. Set initial system_time and monotonic_ms = 0
6. Loop (DELTA_MS = 16):
   a. Set host.system_time += 16ms, host.monotonic_ms += 16
   b. Deliver async I/O
   c. runtime.render(DELTA_MS)
   d. renderer.flush()
   e. Check settlement or explicit frame number -> read_fbo -> save PNG
   f. swap_buffers()
   g. Exit when all captures done or settled
```

### Helper functions

```rust
fn read_fbo_pixels(gl: &glow::Context, fbo: glow::Framebuffer, w: u32, h: u32) -> Vec<u8>
fn save_screenshot(pixels: Vec<u8>, w: u32, h: u32, path: &Path) -> Result<()>
fn create_fbo(gl: &glow::Context, w: u32, h: u32, stencil: bool) -> Result<(Framebuffer, Texture)>
```

### TTY-aware output

Use `std::io::IsTerminal`:

- **TTY**: overwrite line with `\r` — `"  frame 30  [waiting for settlement...]"`
- **Non-TTY**: one line per capture — `"Captured frame 3 -> path/frame_0003.png"`

## Step 6: Comparison Script

**File: `tools/regression_compare.sh`**

```bash
#!/usr/bin/env bash
# Usage: regression_compare.sh --baseline=<dir> --current=<dir> --diff=<dir> [--threshold=0.1]
```

- Walk baseline dir for `*.png`, find matching in current dir
- Run `odiff "$baseline" "$current" "$diff" --threshold "$threshold"` per pair
- Print per-file PASS/DIFF/MISSING summary
- Exit 1 on any regression

## Step 7: Makefile Targets

**File: `Makefile`**

```makefile
SIZES := full:1280x480 large:638x480 medium:638x238 small:317x238
CAPTURE_DIR := captures/$(EXAMPLE)

.PHONY: capture
capture:
	cd $(EXAMPLE_DIR) && cargo build --release --target $(WASM_TARGET)
	@cargo build --features capture --bin capture
	@for entry in $(SIZES); do \
		name=$${entry%%:*}; dim=$${entry#*:}; \
		xvfb-run -a cargo run --features capture --bin capture -- \
			$(subst /debug/,/release/,$(WASM_FILE)) \
			--size=$$dim --output=$(CAPTURE_DIR)/current/$(EXAMPLE)/$$name \
			$(ARGS); \
	done

.PHONY: update-baselines
update-baselines: capture
	rm -rf $(CAPTURE_DIR)/baselines
	cp -r $(CAPTURE_DIR)/current $(CAPTURE_DIR)/baselines

.PHONY: regression-test
regression-test: capture
	tools/regression_compare.sh \
		--baseline=$(CAPTURE_DIR)/baselines \
		--current=$(CAPTURE_DIR)/current \
		--diff=$(CAPTURE_DIR)/diff
```

## Step 8: .gitignore

**File: `captures/.gitignore`** — ignore `*/current/` and `*/diff/`, commit `*/baselines/`.

---

## Verification

1. `make capture EXAMPLE=hello-widget` — PNGs at expected paths
2. Verify auto-settlement: capture binary prints settlement frame number
3. `make update-baselines EXAMPLE=hello-widget`
4. `make regression-test EXAMPLE=hello-widget` — passes
5. Modify widget -> `make regression-test` -> fails with diff images
6. Run capture twice with same `--time` -> byte-identical PNGs (determinism)
7. `make validate-wasm` — clippy/fmt clean
8. Test fixture recording: `make capture EXAMPLE=calendar ARGS="--record-fixtures"` -> `fixtures/fetch_responses.json`
   created
9. Test fixture playback: `make capture EXAMPLE=calendar` -> uses fixtures, no network

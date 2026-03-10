# BDK-285: SpaceX Launch Widget (WASM) + SDK Improvements

**Status: Completed.** All phases implemented and running. This document is the original design plan, lightly updated to
reflect the final implementation. Where the code drifted from the plan, the code is the source of truth — see
`examples/spacex-launch/src/lib.rs` and the SDK sources.

## Context

BDK-285 asks us to port the existing SpaceX launch widget (Chrome/HTML-based, in
`deckfeeder/assets/widgets/spacex-launch`) to the WASM runtime. The real goal is **stress-testing the WASM host API
against a real-world use case** — discovering which SDK primitives are missing, which are awkward, and how the API needs
to evolve.

The existing Chrome widget displays upcoming SpaceX launches from thespacedevs.com LL2 API with countdown timers, launch
metadata, and rocket imagery (3 PNGs: falcon-9, falcon-heavy, unknown ~50KB each). It has 4 size variants and a "Show
Seconds" config param.

**Approach:** Build the widget incrementally, adding SDK features as each gap is hit. Each phase produces a working demo
and surfaces API feedback for BDK-266.

---

## Phase 1: Bitmap Rendering Support

**Gap:** No way to render raster images. The widget needs 3 rocket PNGs.

### SDK side (widget-facing API)

Extend the existing icon pattern (compile-time embedding + lazy host registration + opaque ID).

**New type and macro in `sdk/src/tree.rs` and `sdk-macros/src/lib.rs`:**

```rust
pub struct Bitmap {
    pub data: &'static [u8]
}
const FALCON_9: Bitmap = include_bitmap!("assets/falcon-9.png");
```

**Lazy registration** (same pattern as `ensure_registered` for icons):

```rust
fn ensure_bitmap_registered(bmp: &Bitmap) -> u16 { ... }
```

**Canvas draw command:**

```
Draw::bitmap(x, y, w, h, &FALCON_9)  // renders at (x,y) scaled to (w,h)
```

### Host side (runtime)

**New host function:** `host_register_bitmap(ptr, len) -> u32` returns bitmap_id.

Host decodes PNG once, uploads to GPU texture via FemtoVG `create_image_from_rgba()`. Texture stays in VRAM — zero
per-frame cost. `BitmapRegistry` alongside existing `IconRegistry` in `gpu/`.

**Tree deserialization** in `src/tree.rs`: new `DRAW_BITMAP` command type referencing bitmap ID + rect.

### Size/performance notes

- **WASM module size:** ~150KB extra for 3 compressed PNGs. Acceptable for POC. Production would use external asset
  loading.
- **VRAM:** Host decodes to RGBA → ~600KB per 320×480 texture. 3 textures = ~1.8MB. Acceptable.
- **Decode:** One-time PNG decode during `register_bitmap()`. On embedded ARM \<50ms per image.

### Files to modify

- `sdk-macros/src/lib.rs` — `include_bitmap!` proc macro (embed raw PNG bytes)
- `sdk/src/tree.rs` — `Bitmap` type, `ensure_bitmap_registered()`, `Draw::bitmap()`, serialization
- `sdk/src/host.rs` — `host_register_bitmap` extern + safe wrapper
- `protocol/src/lib.rs` — `DRAW_BITMAP` constant
- `src/runtime.rs` — register `host_register_bitmap` host function
- `src/gpu/bitmap.rs` (new) — `BitmapRegistry` (PNG decode + FemtoVG texture upload)
- `src/tree.rs` — deserialize `DRAW_BITMAP`, render via FemtoVG `draw_image()`

All paths relative to `bmc-wasm-runtime/`.

---

## Phase 2: Widget UI with String Literals

Build the rendering functions for each layout section with hardcoded string literals. No data model, no formatting SDK,
no API mocking — just get pixels on screen matching the Figma design.

### Widget crate

`bmc-wasm-runtime/examples/spacex-launch/`

```
spacex-launch/
├── Cargo.toml
├── assets/
│   ├── falcon-9.png      (from deckfeeder/assets/widgets/spacex-launch/assets/rockets/)
│   ├── falcon-heavy.png
│   └── unknown.png
└── src/
    └── lib.rs
```

### Layout reference (from Figma + HTML implementation)

**FULL (1280×480):**

```
┌─────────────────────────────────────────────────────────────────────┐
│ Space X   Next Launch                                     ┌────────┐│
│                                                           │        ││
│ Starlink Group 7-3                                        │ Rocket ││
│ Mission name                                              │ PNG    ││
│                                                           │        ││
│ Scheduled    30d 02h 14m 32s    Landing    Not confirmed  │        ││
│ Status       Upcoming           Booster    12× flown      │        ││
│ Rocket       Falcon 9           Payload    Starlink       │        ││
│ Place        Cape Canaveral…    Spacecraft Dragon         └────────┘│
└─────────────────────────────────────────────────────────────────────┘
```

- 24px left/bottom padding, 16px top padding
- Header: "Space X" (gray ~14px) + "Next Launch" (white bold ~14px)
- Mission: large bold ~28px, "Mission name" gray label below
- Two data tables side by side, 4 rows each, labels gray, values white bold
- Right: rocket image on dark blue starfield, ~320px wide

**LARGE (638×480):** No rocket image, same header + mission + two tables side by side

**MEDIUM (638×238):** "Space X" + mission name on same line (no "Next Launch", no "Mission name" label), two tables side
by side

**SMALL (317×238):** Mission name as title only, single table (Scheduled, Status, Rocket, Place), no second table

### Rendering functions (lib.rs structure)

As implemented, `render()` pre-computes countdown and status text and passes them into each variant:

```rust
fn render_full(height: u32, data: &LaunchData, countdown: &str, status: &str) -> Node { ... }
fn render_large(data: &LaunchData, countdown: &str, status: &str) -> Node { ... }
fn render_medium(data: &LaunchData, countdown: &str, status: &str) -> Node { ... }
fn render_small(data: &LaunchData, countdown: &str, status: &str) -> Node { ... }
```

### Make target

Uses the generic `Makefile` target: `make dev EXAMPLE=spacex-launch`.

### Files to create

- `examples/spacex-launch/Cargo.toml`
- `examples/spacex-launch/src/lib.rs`
- `examples/spacex-launch/assets/` (copy PNGs from deckfeeder)

---

## Phase 3: Data Fetching + Live Countdown

Two sub-parts: (a) live countdown with system time, (b) network data.

### 3a: Live Countdown + SDK Formatting Module

Replace static countdown text with real computation using a new SDK formatting module.

**New SDK addition:** `sdk/src/format.rs` — first piece of the formatting utility layer (mirrors JS SDK's `sdk.format.*`
from deckfeeder). The JS SDK had to duplicate proto enum definitions because it lives in a separate repository. The WASM
SDK can reference `bmc-wasm-protocol` directly, avoiding that duplication.

**`format_duration(remaining_secs, show_seconds)`** — compact countdown string:

- `show_seconds: false` → `"30d 02h 14m"`
- `show_seconds: true` → `"30d 02h 14m 05s"`
- Returns `"T-0"` when remaining ≤ 0

Zero-pads h/m/s to 2 digits, days not padded. Countdown is timezone-independent (pure UTC unix arithmetic). Timezone
matters for date/time formatting (Phase 3b) — will be delivered from device preferences via a host function, using the
IANA timezone string (not just the numeric offset from `SystemTime`).

**Widget changes:**

- Hardcoded `LAUNCH_UNIX` constant (Phase 3b replaces with live API data)
- `render()` calls `SystemTime::now()`, computes remaining seconds, formats with `format_duration`
- `request_frame_after(1_000)` for per-second updates
- Dynamic status: "Upcoming" vs "Launched" based on countdown sign

**Files:**

- `sdk/src/format.rs` (new) — `format_duration`
- `sdk/src/lib.rs` — re-export
- `examples/spacex-launch/src/lib.rs` — live countdown

### 3b: Network Fetching + JSON Parsing (major SDK addition)

**Gap:** Widgets can't make network requests or parse structured data.

**Platform constraint:** wasmi is strictly synchronous — no async I/O. The host must fetch in the background and deliver
results to WASM via exported callbacks.

#### Design: Export-based response delivery

**Why not function pointers / table indices:** Hot-reload drops the entire wasmi Store and Instance. Function table
references become invalid.

**Why not polling:** Requires manual request-ID juggling in widget code.

**Chosen approach:** The widget exports a named callback function. The host calls it (like `init` and `render`) when a
response arrives. The SDK hides the plumbing.

**Widget-facing API:**

```
// In init():
fetch(API_URL, Some("Authorization: Token ..."), on_launch_data);

// Callback — called by host when response arrives
fn on_launch_data(response: &FetchResponse) {
    let json = response.json();
    let name = json.str("/results/0/mission/name").unwrap_or("Unknown");
    let launch_unix = parse_date(&json.str("/results/0/net").unwrap_or_default()).unwrap_or(0);
    // ... store in state
    request_frame();
}

// Re-fetch every 5 minutes
fetch_after(300_000, API_URL, Some("Authorization: Token ..."), on_launch_data);
```

#### SDK side (`sdk/src/net.rs`)

1. `fetch(url, headers, callback)` stores the callback `fn(&FetchResponse)` in a thread-local `Vec` keyed by index.
   `headers` is `Option<&str>` for raw HTTP headers. Calls
   `host_fetch(url_ptr, url_len, headers_ptr, headers_len) -> request_id`. Maps `request_id -> callback_index` in a
   thread-local HashMap.

2. SDK exports `#[no_mangle] pub extern "C" fn __on_fetch_response(request_id, status, body_ptr, body_len)`.
   Auto-included like `__bmc_sdk_version`. Looks up callback by request_id, constructs `FetchResponse`, calls it.

3. `fetch_after(delay_ms, url, headers, callback)` calls `host_fetch_after(delay_ms, url_ptr, url_len, ...)`.

#### WASM memory for response bodies

SDK exports `__alloc(size) -> ptr` and `__dealloc(ptr, size)` in `sdk/src/alloc.rs`. Host calls `__alloc` to get a
buffer, writes response body, then calls `__on_fetch_response` with the pointer. `__alloc` uses `Vec::with_capacity` +
`leak` for a stable pointer.

#### Host side

1. `host_fetch(url_ptr, url_len, headers_ptr, headers_len) -> u32`: reads URL + optional headers from WASM memory,
   spawns background HTTP request via `ureq`, returns request_id.
2. `host_fetch_after(delay_ms, url_ptr, url_len, headers_ptr, headers_len) -> u32`: same but with a delay.
3. Before each `render()` call, host checks for completed requests. For each: calls `__alloc` to get a WASM buffer,
   writes body, calls `__on_fetch_response(id, status, ptr, len)`.
4. On hot-reload: pending requests are cancelled (dropped). New instance calls `init()` which re-issues `fetch()`.

#### JSON parsing (host-side)

Widgets should NOT bundle serde_json (~30KB+ WASM bloat). Host parses JSON and exposes a JSON Pointer query API:

```
host_json_parse(body_ptr, body_len) -> doc_id      // parse, return opaque handle
host_json_get_str(doc_id, path_ptr, path_len, out_ptr, out_len) -> i32  // byte len, -1=missing, -2=wrong type
host_json_get_i64(doc_id, path_ptr, path_len) -> i64
host_json_get_f64(doc_id, path_ptr, path_len) -> f64
host_json_get_bool(doc_id, path_ptr, path_len) -> i32  // -1=missing, 0=false, 1=true
host_json_free(doc_id)
```

Paths use JSON Pointer syntax (RFC 6901): `/results/0/mission/name`.

**SDK wrapper (`sdk/src/json.rs`):** `JsonDoc(u32)` wrapping doc_id with `.str()`, `.i64()`, `.f64()`, `.bool()` methods
and `Drop` that calls `host_json_free`.

**Host implementation:** `serde_json::Value` stored in `HashMap<u32, Value>` in HostState. Queries via
`Value::pointer()` which already supports RFC 6901.

#### Data model

```rust
struct LaunchData {
    mission_name: String,     // /results/0/mission/name
    launch_unix: i64,         // parsed from /results/0/net (ISO 8601)
    status: String,           // /results/0/status/name
    rocket: String,           // /results/0/rocket/configuration/full_name
    place: String,            // abbreviated from /results/0/pad/location/name + pad/name
    landing: String,          // derived from .../landing/attempt + type/abbrev
    booster: String,          // formatted from .../launcher_flight_number
    payload: String,          // /results/0/mission/type
    spacecraft: String,       // /results/0/rocket/spacecraft_stage/0/spacecraft/name or "N/A"
}
```

Place abbreviations (same as JS widget): "Cape Canaveral SFS" → "CCSFS", "Kennedy Space Center" → "KSC", "Vandenberg
SFB" → "VSFB", "SpaceX Starbase" → "Starbase".

Landing: null → "No attempt", missing → "Not confirmed", else use value. Booster: flight 1 → "Flight #1", flight N → "N×
flown".

- **API endpoint:**
  `https://ll.thespacedevs.com/2.3.0/launches/upcoming/?search=spacex&limit=1&status__ids=1&mode=detailed`
- **API key:** embedded in widget (OK per ticket)
- **Refresh:** every 5 minutes (matches existing widget TTL of 300s)

#### Implementation steps

1. **WASM allocator exports** — `sdk/src/alloc.rs` with `__alloc`, `__dealloc`
2. **Fetch host functions + SDK net module** — host_fetch/host_fetch_after in runtime, PendingFetch + background thread
   \+ mpsc in host_api, `sdk/src/net.rs`, testbed delivery loop
3. **JSON host functions + SDK json module** — host_json\_\* in runtime, JsonDocStore in host_api, `sdk/src/json.rs`
4. **SpaceX widget live data** — LaunchData, fetch() in init, on_launch_data callback

#### Files to modify

- `sdk/src/alloc.rs` (new) — WASM allocator exports
- `sdk/src/net.rs` (new) — `fetch`, `fetch_after`, `FetchResponse`, dispatch export
- `sdk/src/json.rs` (new) — `JsonDoc` wrapper
- `sdk/src/lib.rs` — add modules, re-exports
- `src/runtime.rs` — register fetch + JSON host functions
- `src/host_api.rs` — `PendingFetch`, `JsonDocStore`, background fetch thread
- `src/bin/testbed.rs` — deliver completed fetch responses before render
- `examples/spacex-launch/src/lib.rs` — live API data
- `Cargo.toml` — add `ureq` + `serde_json` to host deps

---

## SDK Addition: WidgetSize + SizeVariant

The host already calls `init(width, height)` with pixel dimensions. Rather than having widget authors match on raw pixel
values, the SDK provides a `SizeVariant` enum and a `WidgetSize` struct that carries both the classified variant and the
actual pixel dimensions.

**In `sdk/src/host.rs`:**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeVariant {
    Full,   // 1280×480
    Large,  //  638×480
    Medium, //  638×238
    Small,  //  317×238
}

#[derive(Debug, Clone, Copy)]
pub struct WidgetSize {
    pub variant: SizeVariant,
    pub width: u32,
    pub height: u32,
}

impl WidgetSize {
    pub fn from_dimensions(w: u32, h: u32) -> Self { ... }
}
```

Widget code then does:

```rust
static SIZE: Cell<WidgetSize> = const { Cell::new(WidgetSize {}) };

#[unsafe(no_mangle)]
pub extern "C" fn init(width: u32, height: u32) {
    SIZE.set(WidgetSize::from_dimensions(width, height));
}

fn render(delta_ms: u32) {
    let size = SIZE.get();
    let root = match size.variant {
        SizeVariant::Full => render_full(size.height, &data, &countdown, &status),
        SizeVariant::Large => render_large(&data, &countdown, &status),
        // ...
    };
    render_ui(size.width, size.height, root);
}
```

The struct carries the pixel dimensions directly so there's no magic numbers scattered through widget code.

---

## SDK Developer Experience

The actual widget code closely matches the original aspirational design. See `examples/spacex-launch/src/lib.rs` for the
full implementation (~440 lines). Key patterns that landed as designed:

- `include_bitmap!` for compile-time PNG embedding with lazy GPU upload
- `fetch(url, headers, callback)` / `fetch_after(delay_ms, ...)` for async data
- `JsonDoc` with JSON Pointer queries (`.str()`, `.i64()`, `.bool()`)
- `WidgetSize::from_dimensions()` → `match size.variant` for layout dispatch
- `render_ui(width, height, root)` submitting a `Node` tree for host-side flexbox layout
- `format_duration()` for countdown text, `request_frame_after(1_000)` for ticking

Notable deviations from the original sketch:

- `fetch` takes an `Option<&str>` headers param (needed for API auth)
- Launch time parsed from ISO 8601 string via `parse_date()`, not `net_epoch` (API field)
- `WidgetSize` is a struct with `variant`/`width`/`height` fields, not a bare enum
- Widget uses a `WidgetState` enum (Loading/Loaded/Error) instead of bare `Option<LaunchData>`
- Canvas draw commands use `Draw::Bitmap { ... }` enum variant, not a `canvas!()` macro

---

## Performance Notes (from profiling)

Profiled with samply on the testbed (4 tiles).

**Checkerboard background: was 19.9% — fixed.** `draw_checkerboard()` used per-cell `gl.scissor()` + `gl.clear()` in a
double loop (~2,400 GL calls per frame). Fixed by rendering once into an FBO at startup and blitting each frame. Now 0%
in the profile.

**cosmic_text: ~10% inclusive — investigated, working as expected.** `Buffer::line_layout` / `BufferLine::layout` show
up hot but the caching is correct. Verified that `layout_runs()` reads from `layout_opt()` (cached) and never triggers
re-layout. The cost comes from legitimate cache misses:

- Taffy's flexbox multi-pass measurement calls `measure_paragraph` with different `max_width` values (MaxContent,
  MinContent, final resolved width). Each distinct `max_width` is a different `ParagraphLayoutCache` key, so a single
  paragraph may be shaped up to 3× on first encounter.
- The countdown text changes every second, invalidating its cache key → 4 reshapings/second in the testbed.
- The testbed multiplies everything 4× (one runtime per tile, each with its own cache). Steady-state after first load:
  only countdown text triggers reshaping, everything else is cache hits. ~10% for text shaping across ~30 text nodes in
  4 simultaneous widget instances is reasonable — not a bug.

**wasmi interpreter: ~17% self time** — the floor cost of the WASM interpreter (`Executor::execute` + `CompiledFuncRef`
conversion + `Result::branch`). Nothing actionable short of switching wasmi compilation mode.

**taffy layout: ~7% inclusive** — flexbox layout computed every frame. Could be skipped for animation-only frames where
the tree hasn't changed (already partially implemented via `cached_tree_data`).

**eglSwapBuffers: ~14% inclusive** — Wayland compositor enforcing presentation timing. `SwapInterval::DontWait` is
accepted (no error) but the compositor still controls vsync. Not actionable on our end.

---

## Deferred (not in scope)

- **LED signaling** — deferred to [BDK-290](https://braiins.atlassian.net/browse/BDK-290), host SDK doesn't expose
  peripherals yet
- **Widget config UI** — the "Show Seconds" / "Add Widget" modal is part of the device's configuration web UI, not the
  widget itself. Widget receives params externally. For this task, `showSeconds` is a compile-time constant or
  hardcoded.

---

## Implementation Order

All phases completed:

1. **Phase 1** (Bitmap) — bitmap registry, `include_bitmap!` macro, GPU texture upload
2. **Phase 2** (Widget UI) — all 4 size variants, flexbox tree layout, canvas draw commands
3. **Phase 3a** (Live countdown) — `format_duration`, `SystemTime`, per-second frame scheduling
4. **Phase 3b** (Network fetch + JSON) — export-based callbacks, host-side JSON, live SpaceX data

---

## Verification

After each phase:

1. **Build + run:** `make dev EXAMPLE=spacex-launch` (build WASM + launch testbed with hot reload)
2. **Visual check:** Compare rendered output against Figma design screenshots
3. **Phase 3a:** Verify countdown ticks every second, displays correct remaining time
4. **Phase 3b:**
   - Widget loads, shows "Loading…" briefly, then populates with real SpaceX launch data
   - Countdown ticks with real `net_epoch` from API
   - All 4 size variants show correct data
   - Hot-reload: widget re-fetches on reload, no crashes
   - Network error: widget shows fallback text, retries after 30s
   - After 5 minutes: data refreshes automatically
5. **Document:** Note every friction point, API gap, or awkwardness — feeds into BDK-266

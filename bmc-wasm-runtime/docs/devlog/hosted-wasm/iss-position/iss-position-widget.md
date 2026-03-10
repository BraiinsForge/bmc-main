# BDK-304: ISS Position Widget (WASM) + Path Drawing Primitive

## Context

BDK-304: WASM replica of the existing ISS position widget (Chrome/HTML-based, in
`deckfeeder/assets/widgets/iss-position`). Like BDK-285 (spacex launch widget) before it, the real goal is
**stress-testing the WASM host API** — this time with network fetching, runtime-registered bitmaps, orbital mechanics,
and custom vector drawing. The ISS widget exercises a significantly broader API surface than SpaceX did:

| Capability          | SpaceX widget          | ISS widget                                            |
| ------------------- | ---------------------- | ----------------------------------------------------- |
| Network fetching    | Yes (one API)          | Yes (two APIs: position + TLE)                        |
| JSON parsing        | Yes                    | Yes                                                   |
| Runtime bitmaps     | No (compile-time only) | Yes (compile-time earth texture for 3D globe)         |
| Canvas drawing      | Simple (bitmap + rect) | Complex (3D globe, orbit path, marker icon)           |
| External Rust crate | No                     | Yes (`sgp4` — tests dependency impact on binary size) |
| Periodic refresh    | Yes (5 min)            | Yes (5 min, with live countdown)                      |

The existing Chrome widget renders a static screenshot once per 5-minute TTL. The WASM version brings live-rendered
attributes: a ticking "Next update" countdown (matching the original Figma design intent), and local control over all
rendering layers.

### Reference

- Static widget: `deckfeeder/assets/widgets/iss-position/`
- Figma design: `figma.com/design/vyuVYAgizKPSuSHJ7Uwd8C` node `12062-108043`
- SpaceX devlog: spacex-launch-widget (patterns reused here)

---

## Phase 1: Path Drawing Primitive (SDK + Host)

**Gap:** The SDK had `rect`, `circle`, `icon`, `bitmap` draw commands but no polyline, polygon, or path drawing. The ISS
widget needs an orbital ground track (stroked polyline) and a day/night terminator (filled polygon). Both require
variable-length point sequences rendered as vector paths.

A single `DRAW_PATH` command with flags controlling stroke/fill and straight/smooth interpolation keeps the protocol
compact and covers future path-drawing needs.

### Wire format

```
DRAW_PATH (0x44):
  [flags:u8]
  [point_count:u16]
  [points: point_count × (x:f32, y:f32)]
  [color:u32]
  [stroke_width:f32]   // only present when flags.fill == 0
```

**Flags byte:**

- bit 0: `closed` — join last point to first (0 = open, 1 = closed)
- bit 1: `smooth` — Catmull-Rom spline interpolation (0 = straight segments, 1 = smooth)
- bit 2: `fill` — fill the enclosed area (0 = stroke only, 1 = fill; implies closed)
- bits 3-7: reserved

Four useful combinations:

| flags   | Macro form                                     | Use case                  |
| ------- | ---------------------------------------------- | ------------------------- |
| `0b000` | `path!(pts, stroke: 4.0, color: WHITE)`        | Debug lines, simple paths |
| `0b010` | `path!(pts, stroke: 4.0, color: BLUE, smooth)` | Orbit ground track        |
| `0b101` | `path!(pts, fill, color: SHADE)`               | Terminator shade          |
| `0b111` | `path!(pts, fill, color: SHADE, smooth)`       | Smooth filled regions     |

### SDK side (`sdk/src/tree.rs`)

`Interpolation` enum, `Draw::Path` variant, and ergonomic `path!` macro:

```rust
path!(points, stroke: 4.0, color: BLUE_50);               // open polyline, linear
path!(points, stroke: 4.0, color: BLUE_50, smooth);       // open polyline, Catmull-Rom
path!(points, fill, color: SHADE_BLACK);                  // filled closed polygon, linear
path!(points, fill, color: SHADE_BLACK, smooth);          // filled closed polygon, smooth
path!(points, stroke: 2.0, color: WHITE, closed);         // closed stroked path
path!(points, stroke: 2.0, color: WHITE, closed, smooth); // closed stroked smooth path
```

The `fill` keyword implies `closed`. The `smooth` keyword switches interpolation to Catmull-Rom.

### Host side

**Deserialization** (`src/tree.rs`): `DrawCommand::Path` variant, `read_draw()` case for `DRAW_PATH`.

**Rendering** (`src/tree.rs` → `render_draw_inner()`): Build a FemtoVG `Path`, then stroke or fill:

- **Straight mode:** `move_to` + `line_to` for each point.
- **Smooth mode:** Convert control points to cubic Bézier curves via Catmull-Rom → cubic conversion. For N points, each
  segment between points[i] and points[i+1] gets control points derived from neighbors:
  ```
  cp1 = p[i]   + (p[i+1] - p[i-1]) / (6 * tension)
  cp2 = p[i+1] - (p[i+2] - p[i])   / (6 * tension)
  ```
  with `tension = 1.0` (standard Catmull-Rom). Edge segments use duplicated endpoints. FemtoVG renders these natively
  via `cubic_to()`.

### Files modified

- `protocol/src/nodes.rs` — `DRAW_PATH` constant
- `sdk/src/tree.rs` — `Draw::Path` variant, convenience functions, serialization
- `src/tree.rs` — `DrawCommand::Path`, deserialization, Catmull-Rom conversion, rendering

---

## Phase 2: Widget Skeleton + Data Table UI

Widget crate with all 4 size variants. Project structure, rendering layout, and table formatting.

### Layout reference (from Figma + static widget screenshots)

**FULL (1280×480):**

```
┌────────────────────────────────────────────────────────────────────────┐
│                                                    ┌──────────────────┐│
│ ISS Position                                       │  3D Globe        ││
│                                                    │  (earth texture) ││
│ Orbit period                        92 min         │                  ││
│ Next update                        4m 32s ←live    │   ~~~orbit~~~    ││
│ In sunlight                  In Earth shadow       │  shade ░░░░░░░   ││
│ Velocity                      27 565 km/h          │      ◉ ISS       ││
│ Over                       15.9°N, 149.6°E         │                  ││
│                                                    └──────────────────┘│
└────────────────────────────────────────────────────────────────────────┘
```

Left: 5-row data table (labels gray, values bold white). Right: 560×480 canvas with 3D globe (GL sphere shader), orbit
track projected onto the globe, ISS marker at center.

**LARGE (638×480):** Same 5-row table, no map panel.

**MEDIUM (638×238):** Header + 3 rows (Orbit, Velocity, Over). Abbreviated labels.

**SMALL (317×238):** Header + 3 rows (Orbit, Velocity, Over).

### Data model

```rust
struct IssData {
    latitude: f64,
    longitude: f64,
    velocity: f64,          // km/h from API
    visibility: Visibility, // Daylight | Eclipsed
    solar_lat: f64,
    solar_lon: f64,
    fetched_at: i64,        // unix timestamp of last successful fetch
}
```

TLE data is stored separately in a `TLE` thread-local `RefCell<Option<(String, String)>>` — it arrives asynchronously
from a different endpoint and doesn't need to be tied to the position data lifecycle.

---

## Phase 3: Network Fetching + Live Data

Two data sources (Mapbox tile fetch removed — replaced by local GL globe rendering):

### API endpoints

```
Position: https://api.wheretheiss.at/v1/satellites/25544
TLE:      https://api.wheretheiss.at/v1/satellites/25544/tles
```

No auth headers needed for wheretheiss.at.

### Fetch strategy

1. **On `init()`:** Fire position + TLE fetches in parallel.
2. **On position response:** Store data. Schedule re-fetch after `REFRESH_MS` (300s). Also schedules TLE re-fetch.
3. **On TLE response:** Store TLE data in the `TLE` thread-local.
4. **On error:** Show error notification, retry after `RETRY_MS` (30s).

### Live countdown: "Next update"

The original Figma design showed "Next update: 5 min" but the static widget changed it to "Last update" because a
countdown doesn't make sense for a static screenshot. The WASM version restores the original intent:

- Compute `remaining = REFRESH_MS / 1000 - (now - fetched_at)`
- Format as `"4m 32s"`
- Ticks every second via `request_frame_after(1_000)`
- When it hits zero, the re-fetch fires naturally (from `fetch_after` scheduled in step 2)

---

## Phase 4: 3D Globe Rendering

See [globe-rendering.md](globe-rendering.md) for the full technical design (phases A–D).

The full-size variant renders a 560×480 canvas with composited layers:

```
Layer 0 (bottom):  3D globe (GL sphere shader with earth texture, terminator, atmosphere)
Layer 1:           Orbit ground track projected onto globe (smooth polyline segments)
Layer 2 (top):     ISS marker icon at globe center
```

The globe rotates in real-time via SGP4 orbital propagation from TLE data, with exponential center smoothing. Host-side
transitions interpolate sphere parameters for smooth animation.

---

## Host-side Formatting

See [host-side-formatting.md](host-side-formatting.md) for the full design.

Widgets call `format_speed!(velocity, 0)` and get back a preference-formatted string like "27 565 km/h" or "17,126 mph".
The host owns the formatting logic and preferences; widgets just consume results.

---

## Deferred (not in scope)

- **Interactivity** — tap-to-toggle views, satellite selection, etc. — not decided yet
- **Middleman API server** — future architecture replaces direct API calls with a Braiins-owned proxy (guards keys,
  caches upstream data)
- **GPU optimization** — see Phase E in [globe-rendering.md](../../../examples/iss-position/devlogs/globe-rendering.md)
  (profile on real GC400 hardware first)

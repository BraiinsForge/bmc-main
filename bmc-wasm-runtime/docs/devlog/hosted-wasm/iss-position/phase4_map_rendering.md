# Phase 4: Map Rendering with Overlays (BDK-304)

## Context

The ISS widget's full-size variant composites a Mapbox tile with three local overlay layers drawn using SDK draw
primitives — specifically the `path!` macro with Catmull-Rom smoothing for the orbit track and linear fill for the
terminator shade.

## Layer order (bottom → top)

1. **Map tile** — Mapbox dark-v11 static PNG, fetched at runtime, registered as bitmap
2. **Terminator shade** — filled polygon from solar position, `path!(fill)`
3. **Orbit ground track** — smooth polyline via SGP4 from TLE, `path!(stroke, smooth)`
4. **ISS marker** — `include_icon!` SVG + background circles

All rendered locally in a single `canvas()` node. The orbit draws ABOVE the terminator (improvement over the static
widget where orbit is baked into the tile under the shade).

## Design

### Dependencies

- `sgp4` crate (v2.3, `default-features = false, features = ["std"]`) — SGP4 orbital propagation from TLE data

### State

```rust
thread_local! {
    static MAP_TILE: Cell<Option<u16>> = const { Cell::new(None) };
    static TLE: RefCell<Option<(String, String)>> = const { RefCell::new(None) };
}
```

`MAP_TILE` holds the registered bitmap ID. `TLE` holds the two TLE lines for SGP4 propagation. Both arrive
asynchronously after position data.

### Fetch strategy

```
init() → fetch position API + fetch TLE API (parallel)
              ↓                        ↓
on_position_data()              on_tle_data()
  → store IssData                → store TLE lines
  → build Mapbox URL             → request_frame()
  → fetch tile PNG
        ↓
on_map_tile()
  → register_bitmap → store id → request_frame()
```

TLE changes rarely — re-fetched on each position refresh (every 5 min). Could be less frequent but simplicity wins.

### Mapbox URL construction

Plain tile, no overlays in URL. Zoom 1, 560×480 @2x, centered on ISS position. Coordinates formatted as plain decimals
using integer math to avoid locale formatting issues (multiply by 10, split integer/fractional parts).

### Orbit ground track (Layer 2) — `path!(smooth)`

SGP4 propagation of TLE data over one orbital period + 20 minutes (~112 min total), 60 points. Catmull-Rom smoothing
interpolates between points for a smooth curve with minimal data. The extra 20 minutes extends the track past canvas
edges.

**Steps:**

1. Parse TLE lines with `sgp4::Elements::from_tle()`
2. Create `sgp4::Constants::from_elements()`
3. Propagate at 60 evenly-spaced intervals centered on current time (half past, half future)
4. Convert ECI position → geodetic lat/lon (GMST rotation + atan2)
5. Apply anchor correction (align SGP4 position at t=0 with API-reported position)
6. Unwrap longitude for continuity (no jumps > 180° between consecutive points)
7. Convert lat/lon → pixel (x, y) via `geo_to_pixel()` Mercator projection
8. Re-center: shift entire polyline by nearest multiple of 512px (world width at zoom 1) if the midpoint drifted
   off-canvas due to antimeridian wrapping
9. Draw: `path!(track, stroke: 4.0, color: ORBIT_COLOR, smooth)`

**Design choice — unwrapping vs antimeridian splitting:** The original plan called for splitting the polyline at
antimeridian crossings (±180° longitude). The implementation instead unwraps longitude so it increases monotonically,
producing a single continuous polyline. This is simpler (one path draw call, no segment management) and the Catmull-Rom
smoothing works better on a single continuous curve.

The tradeoff is that unwrapped longitudes can exceed ±180°, placing points one world-width (512px) off-canvas when the
ISS is near the antimeridian. This is corrected by the re-centering step (step 8), which shifts the entire polyline back
onto the canvas.

**Anchor correction:** SGP4 propagation from potentially stale TLEs can place the ISS several degrees from its actual
position. The anchor correction computes the longitude delta between the API-reported position and the SGP4-computed
position at the current time, then applies this offset to all track points. This keeps the track visually centered on
the map tile without requiring fresh TLEs.

**ECI → geodetic conversion:**

```rust
fn eci_to_geodetic(pos: &[f64; 3], gmst: f64) -> (f64, f64) {
    let lon = pos[1].atan2(pos[0]) - gmst;  // rotate TEME → ECEF by -GMST
    let lat = pos[2].atan2((pos[0] * pos[0] + pos[1] * pos[1]).sqrt());
    // normalize longitude to [-180, 180]
    (lat.to_degrees(), ((lon.to_degrees() + 540.0) % 360.0) - 180.0)
}
```

**GMST calculation:** Vallado (4th ed.) formula from Unix timestamp → Julian date → sidereal time in radians.

**Mercator projection (geo → pixel):**

```rust
fn geo_to_pixel(lat, lon, center_lat, center_lon, w, h) -> (f32, f32) {
    let scale = 512.0;  // full world at zoom 1 in CSS pixels
    let world_x = |lon| (lon + 180.0) / 360.0 * scale;
    let world_y = |lat| {
        let r = lat.to_radians();
        (1.0 - (r.tan() + 1.0 / r.cos()).ln() / PI) / 2.0 * scale
    };
    // offset from center
    (world_x(lon) - world_x(center_lon) + w / 2.0,
     world_y(lat) - world_y(center_lat) + h / 2.0)
}
```

Color: `#1243cd` at 80% opacity → `0x1243_CDCC`, stroke width 4px.

### Terminator shade (Layer 1) — `path!(fill)`

From the static widget. For 140 evenly-spaced points across the canvas width:

1. Convert x fraction → longitude relative to map center
2. `term_lat = atan(-cos(lon_diff) / tan(solar_lat_rad))`
3. Map to pixel y using linear approximation: `y = h/2 - (term_lat / 180) * h` (close enough to Mercator at zoom 1)
4. Build closed polygon: edge → curve → opposite edge

Color: `0x0000_0040` (black at 25% opacity). Night side is above when solar latitude < 0, below otherwise.

### ISS marker (Layer 3)

```rust
const ISS_ICON: Icon = include_icon!("assets/icon-iss.svg");
```

At canvas center (280, 240):

1. `circle(cx, cy, 40.0, 0x1243_CD33)` — outer glow (20% opacity)
2. `circle(cx, cy, 24.0, 0x1243_CDFF)` — solid circle
3. `icon(cx - 28, cy - 28, 56, 56, &ISS_ICON, WHITE)` — ISS shape

### Files modified

- `Cargo.toml` (workspace) — `sgp4` workspace dependency
- `examples/iss-position/Cargo.toml` — `sgp4.workspace = true`
- `examples/iss-position/src/lib.rs` — all rendering code

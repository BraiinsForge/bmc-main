# BDK-304: ISS Widget — 3D Globe Rendering

## Summary

Replaced the Mapbox Static Images API tile with a locally-rendered 3D globe: an equirectangular earth texture rendered
via a custom GL fragment shader with real-time SGP4-driven rotation, terminator shading from sun position, optional
atmosphere effects, and 3D-projected orbital track overlays via FemtoVG vector drawing.

### Key design decisions

- Texture source: Natural Earth 50m vector data → custom dark-themed equirectangular render
- Rendering: GPU fragment shader (OpenGL ES 2.0, GLSL ES 1.00)
- Zero-copy FBO → femtovg texture sharing (no pixel readback)
- Real-time position: SGP4 orbital propagation from TLE data, exponential smoothing
- Draw command: `sphere!` macro with keyword args, generic (not earth-specific)

### Target hardware

- **CPU:** Dual Cortex-A7 @ 650 MHz (NEON) — widget logic runs in wasmi interpreter, NOT native
- **GPU:** Vivante GC400 @ 533 MHz, OpenGL ES 2.0, GLSL ES 1.00
- **Frame budget:** 33 ms (30 fps) — globe shader ~2-4 ms, FemtoVG overlay ~2-5 ms

---

## Phase A: Natural Earth Texture Rendering ✓

Offline Python script renders dark equirectangular textures from Natural Earth 50m vectors.

**Files:**

- `tools/texture_render.py` — cartopy-based renderer, two themes
- `tools/_textures.py` — shared catalog with render constants
- `tools/texture_download.py` — downloads additional satellite/NASA textures
- `tools/texture_preview.py` — side-by-side texture comparison viewer
- `tools/pyproject.toml` — uv project (cartopy, matplotlib)
- `Makefile` — `make render`, `make download`, `make preview`, `make lint`

**Output:** 2048×1024 JPEG at quality 95, ~230 KB each. Two themes:

1. `natural-earth-dark` — neutral gray palette (Mapbox dark-v11 style)
2. `natural-earth-gmaps` — blue-tinted palette (Google Maps dark style)

Additional downloaded textures: Blue Marble, Black Marble, SSS composites.

---

## Phase B: `DrawCommand::Sphere` Protocol Extension ✓

Generic 3D sphere draw command, not earth-specific. Any equirectangular texture can be mapped.

**Wire format:** `[0x45][f32×4 rect][u16 bitmap_id][u8 flags][f32×5 params]` = 44 bytes

| Field        | Type | Description                               |
| ------------ | ---- | ----------------------------------------- |
| `x, y, w, h` | f32  | Canvas position and size                  |
| `bitmap_id`  | u16  | Registered equirectangular texture        |
| `flags`      | u8   | Bit 0: atmosphere enable                  |
| `center_lat` | f32  | Latitude of globe center (degrees)        |
| `center_lon` | f32  | Longitude of globe center (degrees)       |
| `zoom`       | f32  | Camera distance (>1.0 = farther)          |
| `light_lat`  | f32  | Light source latitude (NaN = no shading)  |
| `light_lon`  | f32  | Light source longitude (NaN = no shading) |

**Files:**

| File                    | Change                                                               |
| ----------------------- | -------------------------------------------------------------------- |
| `protocol/src/nodes.rs` | `DRAW_SPHERE = 0x45`                                                 |
| `sdk/src/tree.rs`       | `Draw::Sphere`, `Draw::sphere()`, `sphere!` macro, serialize         |
| `sdk/src/lib.rs`        | Export `Draw` (constructors are associated methods)                  |
| `src/tree.rs`           | `DrawCommand::Sphere`, decode, layout bounds, render dispatch        |
| `src/renderer.rs`       | `draw_sphere()` on `Renderer` trait                                  |
| `src/gpu/renderer.rs`   | `FemtoVgRenderer::draw_sphere()` with lazy init + NaN light handling |

**SDK macro:**

```rust
// Basic (no light, no atmosphere)
sphere!(&TEX, at: (x, y, w, h), center: (lat, lon), zoom: 1.8)

// With light source (terminator shading)
sphere!(&TEX, at: (x, y, w, h), center: (lat, lon), zoom: 1.8,
    light: (sun_lat, sun_lon))

// With atmosphere (limb darkening + edge glow)
sphere!(&TEX, at: (x, y, w, h), center: (lat, lon), zoom: 1.8,
    light: (sun_lat, sun_lon), atmosphere)
```

---

## Phase C: GL Sphere Renderer ✓

**File:** `src/gpu/sphere.rs` — `SphereRenderer` (~470 lines)

### Architecture

```
draw_sphere() called
 ├─ Lazy-init SphereRenderer on first call
 ├─ Lazy-init texture from registered bitmap (via femtovg native texture)
 ├─ Dirty check (lat/lon/zoom/light/atmosphere)
 │   └─ If clean: skip render, reuse cached FBO
 ├─ Bind offscreen FBO
 ├─ Run fragment shader (ray-sphere + equirectangular sampling)
 └─ Draw FBO texture via femtovg (zero-copy — no pixel readback)
```

### Key implementation details

- **Zero-copy FBO sharing:** `canvas.create_image_from_native_texture()` registers the FBO color attachment directly
  with femtovg. No `glReadPixels` needed.
- **Y-flip in vertex shader:** `v_uv = vec2(a_pos.x, -a_pos.y)` handles GL FBO bottom-up vs femtovg top-down coordinate
  mismatch. `ImageFlags::FLIP_Y` caused disappearance in testing.
- **Dirty tracking:** Parameters compared with epsilon (0.001 ≈ 0.06°). NaN initial values force first render.
  Atmosphere tracked as bool.
- **VAO handling:** Optional — created if GL supports it (required on core profile / ES 3.0+), gracefully skipped on ES
  2.0 without the extension.
- **GL state cleanup:** Disables scissor, blend, depth, cull, stencil before drawing — femtovg leaves these in
  unpredictable state after flush.

### Fragment shader features

- Ray-sphere intersection from camera at `(0, 0, zoom)`
- Globe rotation via `Ry(lon) * Rx(-lat)` inverse rotation
- Equirectangular UV mapping: `lon = atan(p.x, p.z)`, `lat = asin(p.y)`
- **Light shading:** `smoothstep(-0.1, 0.15, dot(p, light_dir))`, dark side at 55% brightness. Disabled when
  `u_light_dir` is zero-length (NaN sentinel from SDK).
- **Atmosphere:** Limb darkening `1.0 - rim² * 0.7` + bluish edge glow `vec3(0.12, 0.22, 0.45) * pow(rim, 1.5)`. Gated
  by `u_atmosphere` uniform.

---

## Phase D: ISS Widget ✓

**File:** `examples/iss-position/src/lib.rs`

### Removed (from original Mapbox-based widget)

- `MAPBOX_TOKEN`, `MAP_TILE` thread_local, `on_map_tile()`, `map_tile_url()`
- `geo_to_pixel()` (Web Mercator projection)
- Mapbox tile fetch in `on_position_data()`

### Added

- **SGP4 orbital propagation:** Real-time ISS position from TLE data via `sgp4` crate. Falls back to API position when
  TLE unavailable.
- **Exponential globe center smoothing:** `GLOBE_SMOOTH_MS = 300.0` time constant prevents jumps between position
  updates. Handles longitude wrapping at ±180°.
- **Intuitive zoom remap:** `GLOBE_ZOOM = 1.0` (user-facing) → `globe_zoom_to_camera()` maps to shader camera distance
  (1.8). Range clamped to [0.6, 1.6].
- **3D orbit projection:** `project_orbit_to_globe()` applies the same rotation + perspective as the shader. Back-face
  cull at `vz ≤ 0.40` prevents orbit peek-through. Catmull-Rom spline smoothing on visible segments.
- **Orbit anchoring:** Optional longitude correction when using API fallback position. Disabled during SGP4-only motion
  to prevent jumps.
- **Host-side transitions:** Sphere always wrapped in `.transition(250, Easing::EaseOut)` for smooth parameter
  interpolation. The host `PrevDrawValues` tracks sphere-specific fields (`center_lat/lon`, `zoom`, `light_lat/lon`)
  with `shortest_angle_delta_deg` for longitude wrapping.
- **TLE-gated map panel:** Globe + orbit track only shown after TLE data loads, preventing the visual pop of orbit track
  appearing later.
- **Debug time multiplier:** `TIME_SPEED = 1.0` (real-time). Set to e.g. 60.0 for ~1 orbit per 1.5 min. Applied
  consistently to both SGP4 propagation and GMST.
- **Frame rate:** 33 ms (30 fps) for Full variant with 3D globe, 1000 ms for text-only variants.

### Layout variants

| Variant | Size     | Content                          |
| ------- | -------- | -------------------------------- |
| Full    | 1280×480 | Data table + 3D globe with orbit |
| Large   | 638×480  | Data table (5 rows)              |
| Medium  | 638×238  | Data table (3 rows, compact)     |
| Small   | 317×238  | Data table (3 rows, compact)     |

---

## Phase E: Optimization (future)

1. Profile on real GC400 hardware — measure actual shader frame time
2. Binary shader caching (`GL_OES_get_program_binary`) — if init time >500 ms
3. Half-res globe (280×240 → upscale) — if frame budget tight
4. `atan`/`asin` polynomial approximations — if ALU-bound
5. Dirty threshold tuning — currently re-renders on any 0.001-rad change

---

## Files changed

| File                                         | Phase | Description                                          |
| -------------------------------------------- | ----- | ---------------------------------------------------- |
| `examples/iss-position/tools/*.py`           | A     | Texture render/download/preview scripts              |
| `examples/iss-position/textures/*.jpg`       | A     | Equirectangular earth textures                       |
| `examples/iss-position/Makefile`             | A     | Texture tooling targets                              |
| `protocol/src/nodes.rs`                      | B     | `DRAW_SPHERE = 0x45`                                 |
| `sdk/src/tree.rs`                            | B     | `Draw::Sphere`, `Draw::sphere()`, `sphere!` macro    |
| `sdk/src/lib.rs`                             | B     | Export `Draw` (constructors are associated methods)  |
| `src/tree.rs`                                | B+D   | `DrawCommand::Sphere`, decode, transitions, override |
| `src/renderer.rs`                            | B     | `draw_sphere()` trait method                         |
| `src/gpu/sphere.rs`                          | C     | `SphereRenderer` — GL shader + FBO                   |
| `src/gpu/mod.rs`                             | C     | `mod sphere`                                         |
| `src/gpu/renderer.rs`                        | C     | Lazy init, NaN light, atmosphere passthrough         |
| `src/host_api.rs`                            | D     | `PrevDrawValues` sphere fields                       |
| `examples/iss-position/src/lib.rs`           | D     | ISS widget — SGP4, smoothing, orbit, transitions     |
| `examples/iss-position/textures/viewer.html` | A     | Browser-based texture comparison viewer              |

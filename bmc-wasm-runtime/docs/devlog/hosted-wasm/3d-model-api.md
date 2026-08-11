# 3D Mesh Draw API Design

## Context

The WASM widget runtime already has a `Draw::sphere()` command that renders a textured globe via a custom GL shader in
an offscreen FBO, shared zero-copy with femtovg. This extends that pattern to support arbitrary 3D meshes — uploaded
from WASM, rendered entirely on GPU, with quaternion-based transforms that are also GPU-side and animatable by the host.

Target platform: **STM32MP157C** with **Vivante GC400** GPU (OpenGL ES 2.0 only, tile-based renderer) and **256MB DDR3**
(shared system + GPU memory).

## Guest-Side API (SDK)

```
static ROCKET: Mesh = include_mesh!("assets/rocket.glb");

Draw::mesh(x, y, w, h, &ROCKET, MeshView {
    fov: 45.0,
    distance: 3.0,
    orientation: Orientation::from_euler(pitch, yaw, roll),
    position: [0.0; 3],
    scale: 1.0,
    light: Some(LightAngles { pitch: 45.0, yaw: -30.0 }),
    highlight: None,
})
```

### `Orientation` — quaternion with human-readable API

```
Orientation::none()
Orientation::from_euler(pitch, yaw, roll)
Orientation::from_axis_angle(ax, ay, az, angle_deg)
Orientation::look_at(lat, lon)
orientation_a.then(orientation_b)
// With math-3d feature: From<glam::Quat> / Into<glam::Quat>
```

### `Mesh.face_normals` — per-face metadata from GLB extras

Blender export scripts embed face normals (in glTF Y-up space) into the mesh node's `extras` JSON. The `include_mesh!`
proc macro extracts them at compile time into `Mesh.face_normals: &[[f32; 3]]`. Widgets use these for face selection,
orientation computation, and highlight targeting — single source of truth in the model file.

## Wire Format

```
DRAW_MESH (0x47): 88 bytes
  x: f32, y: f32, w: f32, h: f32,         // viewport rect
  mesh_id: u16,                             // registered mesh handle
  fov: f32, distance: f32,                  // camera
  qx: f32, qy: f32, qz: f32, qw: f32,     // orientation quaternion
  px: f32, py: f32, pz: f32, scale: f32,    // transform
  light_pitch: f32, light_yaw: f32,         // NaN = no light
  hl_u_min..hl_v_max: 4× f32,              // highlight UV rect (NaN = none)
  hl_r, hl_g, hl_b: 3× f32,               // highlight color
```

## Binary Mesh Format (packed by `include_mesh!`)

```
Header (40 bytes):
  [0..4]   magic: u32 ("MDL1")
  [4..8]   vertex_count: u32
  [8..12]  index_count: u32
  [12..16] vertex_offset: u32
  [16..20] index_offset: u32
  [20..24] texture_offset: u32
  [24..26] texture_width: u16
  [26..28] texture_height: u16
  [28]     texture_format: u8 (0=RGBA8, 1=ETC1)
  [29]     flags: u8 (HAS_TEXTURE=0x01, HAS_UVS=0x02, HAS_TANGENTS=0x04, HAS_NORMAL_MAP=0x08)
  [30..34] normal_map_offset: u32
  [34..36] normal_map_width: u16
  [36..38] normal_map_height: u16
  [38..40] _reserved

AABB (24 bytes): min_xyz, max_xyz as 6× f32

Vertices (per vertex, quantized):
  pos: 3×i16 (normalized to AABB)
  normal: u32 (10/10/10/2 packed)
  uv: 2×u16 (optional, +4 bytes)
  tangent: 4×i16 (optional, +8 bytes)

Indices: u16 per index
Albedo texture: ETC1 compressed (format=1) or raw RGBA8 (format=0)
Normal map: same format as albedo (optional)
```

## Shader (GLSL ES 1.00)

Vertex: MVP transform, TBN matrix from tangent + normal (when tangents present). Fragment: diffuse + hemisphere ambient,
optional normal map perturbation, UV-rect highlight tint.

## Devlog

### Phase 1: Static mesh on screen

Full vertical slice: proc macro, wire format, host renderer, FBO compositing, dirty-tracking.

- `../../../../widgets-wasm-examples/mesh-demo/` — textured Suzanne (3000 tri, 512×512 baked clay texture)
- Touch-drag rotation with relative cursor tracking

### Phase 2: Transforms + transitions

Quaternion nlerp interpolation, `.transition()` on `Draw::mesh()`, D6 dice demo.

- `PrevDrawValues` extended with `Quat` / `Vec3` for orientation and position
- `nlerp()` with short-path sign flip, `MeshOverride` pattern in `render_draw_inner`
- D6: beveled cube, 3x3 texture atlas, per-face UV mapping, auto-smooth normals
- `tools/D6.py` dual-mode: `--reset-texture` (pure Python) / `make D6` (Blender)
- Bounce easing functions: `EaseOutBack`, `EaseInOutBack`, `EaseOutBounce`, `EaseOutElastic`

### Phase 3: Normal maps

Tangent-space normal mapping with per-vertex tangent vectors.

- Header 32→40 bytes, `FLAG_HAS_TANGENTS` / `FLAG_HAS_NORMAL_MAP`
- Proc macro extracts `TANGENT` attribute + `normalTexture` from glTF
- Vertex shader builds TBN matrix, fragment shader perturbs shading normal

### Phase 4: FBO aliasing fix

Opt-in MSAA on the mesh FBO via `RuntimeConfig.mesh_msaa_samples`.

- Desktop: MSAA renderbuffers + `glBlitFramebuffer` resolve. Two FBOs when MSAA > 0.
- Config threaded: `RuntimeConfig` → `FemtoVgRenderer::new` → `MeshRenderer::new`.
- Testbed and capture set 4×. Device hosts default to 0.

### Phase 5: Bounce easing

New easing functions for natural settle animations.

- `EaseOutBounce` for D6 dice landing
- WASM-driven tumble with drifting axis + deceleration for roll animation

### Phase 6: glam integration + dice fixes

Replace hand-rolled quaternion/matrix math with `glam` crate. Fix D6/D20 face selection, orientation, and highlight.

**SDK**: `glam` as optional `math-3d` feature with `libm` backend (wasm-friendly, no host-stdlib pull). `Orientation`
gets `From/Into<Quat>`. `host::random_u32()` host-seeded PRNG for deterministic capture/replay.

**Host**: `glam` unconditional. `quat_to_mat3` uses `glam::Mat3::from_quat`. `host_random_u32` xorshift64 PRNG with
`RuntimeConfig::rng_seed` (`None` = auto-seed, `Some(s)` = deterministic). Capture/recording use `rng_seed: Some(42)`.

**Dice face fixes**:

1. Blender→glTF coordinate conversion: face normals converted via `(x, y, z) → (x, z, -y)`
2. Renderer rotation transpose: `compute_mvp`/`flatten_mat3` fixed to put R columns in MV columns
3. Tangent-frame orientation: full `Mat3::from_cols` construction with projected world-up
4. D20 UV mapping: tangent-frame projection with centroid-centered normalization
5. Highlight V-flip: removed double-flip (D20.py flips for glTF, OpenGL upload is unflipped)
6. Random dice rolls via `host::random_u32()`

### Phase 7: D20 polish

- Standard D20 face layout: opposite faces sum to 21, min adjacent diff = 4 (`DISPLAY_NUMBER` permutation in D20.py)
- Centroid-centered UV mapping: numbers appear centered on triangular faces
- Face normals embedded in GLB `extras` → `Mesh.face_normals` (single source of truth)
- Error overlay when die model missing face data

### Phase 8: ETC1 texture compression + visual polish

Compile-time ETC1 compression via `intel_tex_2` (Intel ISPC encoder, pre-compiled kernels).

- `include_mesh!` proc macro compresses albedo + normal map textures to ETC1 at build time
- Host uploads via `glCompressedTexImage2D` with `GL_ETC1_RGB8_OES`
- 8:1 compression ratio vs RGBA8 (1024×1024: 4MB → 512KB per texture)
- `texture_format` byte in header discriminates RGBA8 vs ETC1
- No alpha channel — fine for albedo and tangent-space normal maps
- Textures bumped to 1024×1024 to avoid ETC1 block artifacts on text edges
- D6.py / D20.py pip sizes, digit sizes, and normal map sample distance now scale with `IMG_SIZE`

Fragment shader improvements:

- **Blinn-Phong specular**: half-vector with fixed view direction `(0,0,1)`, shininess 32, configurable intensity
- **Edge darkening**: rim-based falloff `1 - rim² × 0.3` for depth without AO
- **Configurable lighting**: `MeshView.ambient` (shadow brightness) and `MeshView.specular` (highlight strength) exposed
  as uniforms, interpolated during transitions

### Phase 9: Multi-dice tray + atlas FBO

Google Dice-style tray: add/remove D6/D20, tap to remove, Roll All, total display.

**Atlas FBO** (`gpu/mesh.rs`): Replaced single 480×480 FBO with a 960×960 atlas divided into a 3×3 grid of 320px slots.
Each `draw_mesh()` call takes a `slot_index` and renders into its own viewport region via `glViewport` + `glScissor`.
femtovg samples sub-rects of the single atlas texture via `draw_bitmap_subrect()`. One FBO bind per frame, predictable
memory (~5.5MB RGBA+depth), 9 independent mesh renders. MSAA blit is scoped per-slot.

**Slot allocation** (`tree.rs`): `AnimationContext.mesh_slot_counter` auto-increments per `draw_mesh()` call, giving
each mesh a unique atlas slot within a frame.

**Renderer pipeline** (`renderer.rs`): `Renderer::draw_mesh()` trait gains `slot_index: u8`. `FemtoVgRenderer` passes it
through to `MeshRenderer::render()` and uses `draw_bitmap_subrect()` to sample the correct atlas region.

**Dice tray** (`mesh-demo`): Full-screen wrapping grid with floating overlays:

- Dice fill entire widget area in a `flex-wrap: wrap` row with centered justify
- Cell width = `area_w / cols` controls wrap boundaries for balanced rows
- Column count from a lookup table by dice count (e.g. 7→4 cols = 4+3 rows)
- Floating overlays (absolute positioned): mode tabs top-left, sum badge top-right, action buttons bottom-left
- Tap die to remove, Roll button, +D6/+D20 buttons, Σ total

### Phase 9b: `flex-wrap` layout primitive

Added `wrap: bool` to `PropsData`, packed into bit 8 of the existing `cross_align` u32 (no wire format size change,
backward compatible). When `props.wrap` is true, taffy gets `FlexWrap::Wrap` + `JustifyContent::Center` +
`AlignContent::Center` — equivalent to CSS `flex-wrap: wrap; justify-content: center; align-content: center`.

## Next steps

### More dice types (D4, D8, D10, D12)

Complete the Google Dice set. Requires new Blender export scripts and GLB models following the D6/D20 pattern.

### Visual polish

- **Distressed/worn texture**: hand-painted albedo + normal map

# mesh-demo — 3D Mesh Rendering in WASM Widgets

A demo widget showcasing real-time 3D mesh rendering inside the Braiins Deck WASM widget runtime. Renders textured, lit
3D models entirely on GPU (OpenGL ES 2.0) from a sandboxed WebAssembly module, with quaternion-based orientation, slerp
animation, and touch interaction.

## What it demonstrates

- **Suzanne** — textured monkey head, drag-to-rotate with smooth easing
- **Dice Tray** — Google Dice-style multi-die tray with all standard RPG dice (D4, D6, D8, D10, D12, D20):
  - Add/remove dice (double-tap to remove, D4–D20 buttons), up to 9 simultaneous
  - Roll All with independent slerp animations per die
  - Running total (Σ badge)
  - Responsive wrapping grid layout with balanced rows
  - D4: beveled tetrahedron with numbered faces
  - D6: beveled cube with pip textures, normal-mapped indentations
  - D8: beveled octahedron with numbered faces (opposite faces sum to 9)
  - D10: beveled pentagonal trapezohedron with 0-9 numbering (opposite faces sum to 9)
  - D12: beveled dodecahedron with numbered faces (opposite faces sum to 13)
  - D20: icosahedron with standard layout (opposite faces sum to 21), gold face highlight

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│ WASM module (mesh-demo)                                     │
│   - Owns game state (current face, roll animation, drag)    │
│   - Emits Draw::mesh() commands with orientation quaternion  │
│   - Reads face normals from Mesh.face_normals (GLB extras)  │
│   - Uses host::random_u32() for dice rolls (deterministic)  │
└──────────────┬──────────────────────────────────────────────┘
               │ serialized draw tree (binary protocol)
┌──────────────▼──────────────────────────────────────────────┐
│ Host runtime (bmc-wasm-runtime)                             │
│   - Deserializes draw commands, drives layout (Taffy)       │
│   - MeshRenderer: 960×960 atlas FBO, 3×3 slot grid         │
│   - Each die renders into its own atlas slot (glViewport)   │
│   - Composites atlas sub-rects into femtovg (zero-copy)     │
│   - Slerp interpolation for smooth orientation transitions  │
└─────────────────────────────────────────────────────────────┘
```

### Model pipeline

```
Blender script (tools/D4.py … D20.py, shared utils in tools/_common.py)
    ↓  generates mesh geometry, UV maps, face normals
    ↓  textures rendered via PIL + BraiinsSans-Bold.otf (anti-aliased)
    ↓  normal maps: Sobel edge detection (numbered dice) or concave bowls (D6 pips)
    ↓  exports .glb with face normals in glTF extras (Y-up space)
include_mesh!("assets/D20.glb")  — proc macro at compile time
    ↓  parses glTF, validates constraints, quantizes vertices
    ↓  packs into optimized binary (10-22 bytes/vertex)
    ↓  extracts face normals from extras
Mesh { data, face_normals }  — static, embedded in WASM binary
    ↓  data uploaded to GPU on first use (VBO/IBO/textures)
    ↓  face_normals used by WASM for orientation math
```

### Key SDK features used

| Feature              | Description                                                                   |
| -------------------- | ----------------------------------------------------------------------------- |
| `Draw::mesh()`       | Render a 3D mesh with camera, orientation, lighting, and highlight parameters |
| `include_mesh!`      | Compile-time GLB→binary packing with validation and quantization              |
| `Mesh.face_normals`  | Per-face normals from GLB extras for orientation computation                  |
| `Orientation`        | Quaternion-based orientation with euler/axis-angle/look-at constructors       |
| `host::random_u32()` | Host-seeded PRNG — deterministic in capture/replay mode                       |
| `MeshView.highlight` | UV-rect highlight tint for indicating the rolled face                         |
| `.transition()`      | Host-driven interpolation for smooth drag response                            |

### Shader

GLSL ES 1.00 targeting Vivante GC400 (OpenGL ES 2.0, tile-based):

- Perspective projection with configurable FOV and distance
- Diffuse + hemisphere ambient lighting with directional light
- Tangent-space normal mapping (optional, per-mesh)
- UV-rect highlight tint (gold overlay on rolled face)
- Quaternion→rotation matrix on CPU, uploaded as MVP uniform

### Binary mesh format

Optimized for minimal GPU memory and fast upload:

- Positions: quantized to `i16` within AABB (6 bytes/vertex)
- Normals: packed 10/10/10/2 format (4 bytes/vertex)
- UVs: quantized to `u16` (4 bytes/vertex, optional)
- Tangents: quantized to `i16` (8 bytes/vertex, optional)
- Textures: raw RGBA8 (albedo + optional normal map)
- Indices: `u16`

## Building

```bash
# Enter the examples workspace
cd widgets-wasm-examples

# Build the WASM module
cargo build -p mesh-demo --target wasm32-unknown-unknown --release

# Or use the Makefile which also runs wasm-opt
cd mesh-demo && make
```

### Regenerating models

Requires Blender 4.0+ (tested with 5.0) and Pillow for font texture generation (numbered dice). The Makefile
auto-detects Blender via PATH, flatpak, or snap, and uses `nix-shell -p python3Packages.pillow` for texture
pre-generation.

```bash
# Regenerate a single die (e.g. D10)
make -C widgets-wasm-examples/mesh-demo D10

# Regenerate all models
make -C widgets-wasm-examples/mesh-demo generate

# Reset textures only (no Blender needed, still requires PIL)
make -C widgets-wasm-examples/mesh-demo reset-D10
```

## Constraints (GC400 / STM32MP157)

- Max 5,000 triangles per mesh (budget for 30fps on tile-based GPU)
- Max 65,535 vertices (u16 index limit)
- Max 1024×1024 textures, power-of-2 dimensions (ES 2.0 NPOT limitation)
- ETC1 compressed textures (compile-time, 8:1 vs RGBA8)
- Single directional light + hemisphere ambient (no shadow maps)
- MSAA optional (4× in testbed/capture, 0 on device by default)

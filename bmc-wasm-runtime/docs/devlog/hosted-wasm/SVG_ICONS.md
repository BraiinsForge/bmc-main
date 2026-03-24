# SVG Icon Rendering

## Context

Widgets currently have rects, circles, and text but no icon support. Icons are essential for UI (settings, close,
arrows, status indicators). The goal is build-time SVG compilation with zero WASM runtime overhead and zero-boilerplate
DX.

**Design principles:**

- SVG parsed at compile time by a proc macro — no SVG parsing at runtime
- Icon binary data embedded as `&'static [u8]` in WASM `.data` section
- On first use, SDK calls `host_register_icon()` to transfer data to host — host parses into FemtoVG paths and returns
  opaque u16 ID
- Tree buffer carries only the u16 ID per icon draw (not full data every frame)
- Tinting: `color != TRANSPARENT` → monochrome tint; `color == TRANSPARENT` → original SVG colors

## DX

```
use bmc_wasm_sdk::*;

// Compile-time SVG → compact binary path data
const SETTINGS: Icon = include_icon!("assets/settings.svg");
const ARROW: Icon = include_icon!("assets/arrow-right.svg");

// Usage in canvas draw commands — works with all transforms and animations
canvas(props!(width: 48.0, height: 48.0), [
    icon(8.0, 8.0, 32.0, 32.0, &SETTINGS, GRAY_10),
    icon(8.0, 8.0, 24.0, 24.0, &ARROW, RED_50)
        .animate(Alpha, 0.0, 1.0, 1_000, EaseInOut, PingPong),
])
```

Behind the scenes: `icon()` lazily calls `host_register_icon()` on first use, caches the returned ID in a thread-local
registry, and emits `Draw::Icon { icon_id, ... }`.

## Architecture

```
 Compile time (host)                 Runtime
┌──────────────────────┐
│ bmc-wasm-sdk-macros  │
│  (proc-macro crate)  │
│                      │
│  usvg parses SVG     │     ┌───────────────────────┐
│  → compact binary    │     │  WASM widget binary   │
│    path ops          │────▶│  (icon data in .data) │
│  → &'static [u8]     │     └──────────┬────────────┘
└──────────────────────┘                │ first use: host_register_icon(ptr, len) → id
                            ┌───────────▼────────────┐
                            │  bmc-wasm-runtime      │
                            │  host                  │
                            │                        │
                            │  IconRegistry:         │
                            │    id → FemtoVG Paths  │
                            │  Scale viewbox→target  │
                            │  Apply tint color      │
                            └────────────────────────┘
                                       ▲
                                       │ tree buffer: DRAW_ICON + u16 id (23 bytes)
```

## Icon binary format

Emitted by the proc macro, stored as `&'static [u8]` in WASM:

```
[viewbox_w: f32][viewbox_h: f32][path_count: u16]
  for each path:
    [flags: u8]              // bit 0: has_fill, bit 1: has_stroke, bit 2: even-odd fill
    [fill_color: u32]        // RGBA, present if has_fill
    [stroke_color: u32]      // RGBA, present if has_stroke
    [stroke_width: f32]      // present if has_stroke
    [op_count: u16]
      for each op:
        0x00 MoveTo  [x: f32][y: f32]
        0x01 LineTo  [x: f32][y: f32]
        0x02 QuadTo  [cx: f32][cy: f32][x: f32][y: f32]
        0x03 CubicTo [cx1: f32][cy1: f32][cx2: f32][cy2: f32][x: f32][y: f32]
        0x04 Close
```

Flags byte bit 2 (`ICON_FLAG_EVENODD`, 0x04) indicates the path uses SVG `fill-rule="evenodd"`. The icon compiler
detects this via usvg's `Fill::rule()` (the attribute is inheritable, so it works whether set on `<svg>` or `<path>`).

usvg simplifies all SVG elements (rects, ellipses, transforms, CSS, etc.) down to bezier paths. Coordinates are in
viewbox space — the host scales to target dimensions at render time.

## Even-odd fill rule

Many icons (Carbon's `error--solid`, `warning--solid`, `checkmark--solid`, `info--solid`) use compound paths with
`fill-rule="evenodd"` to create cutouts (e.g. the diagonal slash in the error icon). The icon compiler stores this as a
flag bit per path; the host renderer passes `FillRule::EvenOdd` to FemtoVG's `fill_path`.

**Critical gotcha:** FemtoVG's anti-alias fringe (~1px wide triangle strip along path edges) bleeds into narrow cutouts
from both sides, filling them in. For a 2px-wide slash cutout, the two fringes overlap and the cutout disappears
entirely. The fix is to disable AA on even-odd fills:

```
// from src/gpu/icons.rs — draw_icon()
if icon_path.is_evenodd {
    paint.set_fill_rule(FillRule::EvenOdd);
    paint.set_anti_alias(false);
}
```

FemtoVG's own text renderer does the same thing — glyph outlines use even-odd fill with AA disabled
(`fill_path_internal(path, color, false, FillRule::EvenOdd)` in `text.rs`).

## Wire format (tree buffer)

```
[DRAW_ICON: u8][x: f32][y: f32][w: f32][h: f32][color: u32][icon_id: u16]
```

Only 23 bytes per icon draw command. The icon data was already transferred to the host during registration (first use).
No per-frame data transfer.

## Registration flow

1. `include_icon!("star.svg")` compiles SVG → `Icon { data: &'static [u8] }` at build time
2. First call to `icon(x, y, w, h, &STAR, color)`:
   - Checks thread-local `Vec<(usize, u16)>` for `data.as_ptr()`
   - Not found → calls `host_register_icon(data.as_ptr(), data.len()) → icon_id`
   - Stores `(ptr_as_usize, icon_id)` in registry
3. Returns `Draw::Icon { x, y, w, h, color, icon_id }`
4. Subsequent calls reuse cached `icon_id` — zero overhead

Host side:

- `host_register_icon` reads data from WASM memory, parses binary, builds FemtoVG paths, stores in
  `HashMap<u16, RegisteredIcon>`, returns ID
- Icons persist for runtime lifetime. On hot-reload, runtime + WASM are recreated, so re-registration happens naturally

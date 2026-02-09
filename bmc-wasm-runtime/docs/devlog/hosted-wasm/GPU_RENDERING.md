# GPU Rendering Architecture

GPU-accelerated widget rendering using FemtoVG + cosmic-text.

## Stack

| Layer | Crate | Role |
|-------|-------|------|
| Windowing (testbed only) | winit 0.30, glutin 0.32 | GL context, event loop |
| GPU rendering | femtovg 0.9 | Shapes, text, paths via OpenGL |
| Paragraph layout | cosmic-text 0.17 | Shaping, line-breaking, rich text |
| WASM execution | wasmi | Interpreted widget bytecode |
| Layout engine | taffy | Flexbox layout |

## Ownership

```
Host (testbed / future remote-widget host)
  provides: GL proc address loader, event loop, swap_buffers
  calls:    WasmWidgetRuntime API

WasmWidgetRuntime
  └─ wasmi::Store<HostState>
       └─ HostState
            ├─ FemtoVgRenderer
            │    ├─ femtovg::Canvas<OpenGl>
            │    ├─ FontId (regular + bold)
            │    ├─ cosmic_text::FontSystem
            │    └─ ParagraphLayoutCache
            ├─ InteractionState
            ├─ animation / transition / modal states
            └─ frame control flags
```

The runtime owns everything. The host only provides a GL proc loader at init and calls
`renderer()` / `render()` / `push_touch_event()` during the frame loop.

Host functions use safe partial struct borrows on `HostState` fields (renderer + interaction
as disjoint borrows — no raw pointers needed).

## Frame flow

```rust
// Init — host provides GL proc loader, crate creates everything else
let runtime = unsafe {
    WasmWidgetRuntime::new(&wasm_bytes, |s| gl_display.get_proc_address(s), w, h)?
};

// Each frame
runtime.renderer().begin_frame(w, h);
runtime.render(delta_ms)?;           // WASM executes, tree renders
runtime.renderer().flush();
surface.swap_buffers(&gl_context)?;  // host's responsibility
```

Each `runtime.renderer()` returns `&mut FemtoVgRenderer` — NLL ensures the borrow ends
before the next call.

## Module structure

| File | Purpose |
|------|---------|
| `src/renderer.rs` | `Renderer` trait — f32 coords, u32 RGBA colors |
| `src/gpu/renderer.rs` | `FemtoVgRenderer` impl — shapes, text, transform stack |
| `src/gpu/text.rs` | `ParagraphLayoutCache` — cosmic-text layout + FemtoVG rendering |
| `src/tree.rs` | Deserialize widget tree, taffy layout, render via `&mut dyn Renderer` |
| `src/runtime.rs` | `WasmWidgetRuntime` — wasmi store, host function registration |
| `src/host_api.rs` | WASM host function implementations |
| `src/components/button.rs` | Immediate-mode button with `&mut dyn Renderer` |
| `src/bin/testbed.rs` | Dev testbed — winit + glutin, hot-reload, perf overlay |

## Paragraph rendering: cosmic-text + FemtoVG

cosmic-text shapes and line-breaks the text. FemtoVG renders each glyph run segment on
the GPU. Both use rustybuzz internally with the same BraiinsSans font binary, so glyph
advances match.

**Measure path** (taffy callback): cosmic-text `Buffer` → `set_rich_text()` →
`shape_until_scroll()` → read `layout_runs()` for width/height. Cached by content hash
with frame-based GC.

**Draw path**: walk cached `layout_runs()`, group glyphs by originating span, call FemtoVG
`fill_text()` per segment at positions from cosmic-text's glyph data. Decorations
(underline, strikethrough) drawn as thin filled rects relative to the baseline.

### cosmic-text `LayoutRun` coordinate model

This is the most important gotcha in the codebase:

**`run.line_y` is the alphabetic BASELINE, not the top of the line.**

cosmic-text computes it as:
```
line_y = line_top + centering_offset + max_ascent
```
where `centering_offset = (line_height − glyph_height) / 2`.

`run.line_top` is the actual top of the line box.

When rendering with FemtoVG, we must use `Baseline::Alphabetic` with `line_y`:
```rust
paint.set_text_baseline(femtovg::Baseline::Alphabetic);
canvas.fill_text(x, y + run.line_y, text, &paint);  // line_y = baseline
```

Using `Baseline::Top` with `line_y` would place text far too low (baseline treated as
top edge). This was the root cause of a text positioning bug during the migration.

Decorations are positioned relative to this baseline:
- **Underline**: `baseline_y + font_size * 0.1` (just below baseline)
- **Strikethrough**: `baseline_y - font_size * 0.3` (approximate x-height)

## Font handling

BraiinsSans regular + bold are embedded via `include_bytes!` from
`bmc-display/ui/assets/fonts/`. Loaded into both:

- **FemtoVG** (`canvas.add_font_mem`) — for GPU glyph rendering
- **cosmic-text** (`font_db.load_font_data`) — for shaping and layout

cosmic-text's `FontSystem` is created with an empty DB (`new_with_locale_and_db`) to avoid
loading system fonts on the target hardware.

Font selection: weight < 600 → regular, ≥ 600 → bold.

## Key decisions

- **f32 coordinates** throughout the `Renderer` trait — avoids integer truncation artifacts
  from the old tiny-skia path
- **No SwashCache** — cosmic-text is used only for layout (shaping + line-breaking), not
  glyph rasterization. FemtoVG handles all rendering.
- **`Renderer` trait is object-safe** — `process_tree` accepts `&mut dyn Renderer`,
  allowing future backend swaps without touching tree logic
- **femtovg 0.9** pinned to match the corinthia production branch. Note: femtovg 0.9.2
  pins glutin 0.30 internally — must use `OpenGl::new_from_function` (not
  `new_from_glutin_display`)

## Testbed

The testbed (`src/bin/testbed.rs`) acts as a minimal host, mimicking the future
remote-widget host role on BMC:

- **winit 0.30** `ApplicationHandler` for event-driven rendering
- **glutin 0.32** for GL context (EGL on Wayland, GLX on X11)
- **Hot-reload**: file watcher on the WASM binary, recreates `WasmWidgetRuntime` on change
  (GL context persists)
- **Perf overlay**: frame time chart (microsecond precision) + FPS counter
- **Memory stats**: reads `/proc/self/status` at exit for VmPeak/VmHWM/VmRSS with
  per-phase breakdown (GL baseline vs WASM runtime delta)
- **No vsync** by default (`SwapInterval::DontWait`) for uncapped throughput measurement

### Scroll convention

winit's `MouseScrollDelta::LineDelta` positive y means "content should move down"
(scroll up). The interaction system expects positive delta = scroll down. The testbed
negates and scales: `(-y * 30.0) as i32`.

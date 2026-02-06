# GPU Rendering: Replace tiny-skia + minifb with FemtoVG + winit

## Context

The bmc-wasm-runtime testbed uses tiny-skia (CPU rasterizer) + minifb (limited windowing). tiny-skia doesn't work on the
target hardware (STM32MP157C Vivante GPU, OpenGL ES 2.0). The corinthia branch (
`origin/mv/BDK-226/corinthia-demo-wayland`) already uses FemtoVG 0.9.2 + glow 0.16 for production widget rendering on
device. This POC migrates bmc-wasm-runtime to the same GPU stack.

The bmc-wasm-runtime crate is the integration target — the testbed is a simulacrum of the future remote-widget host in
BMC. The crate owns the rendering stack; the host only provides a GL context and event loop.

**Key decisions from discussion:**

- f32 coordinates throughout the Renderer trait
- Pin femtovg to 0.9 (match corinthia)
- tiny-skia is OUT, minifb is OUT
- Keep cosmic-text for paragraph layout (line-breaking, shaping) — FemtoVG for GPU rendering
- No SwashCache needed (cosmic-text used only for layout, not glyph rasterization)

## Architecture

```
Host (testbed / future remote-widget)
  provides: glow::Context, event loop, swap_buffers
  calls:    WasmWidgetRuntime API

WasmWidgetRuntime
  └─ wasmi::Store<HostState>
       └─ HostState
            ├─ FemtoVgRenderer          ← crate owns the renderer
            │    ├─ femtovg::Canvas<OpenGl>
            │    ├─ cosmic_text::FontSystem
            │    └─ ParagraphLayoutCache
            ├─ InteractionState
            ├─ animation/transition/modal states
            └─ frame control flags
```

**Ownership**: The runtime owns everything including the renderer. HostState holds FemtoVgRenderer directly — host
functions access it via `caller.data_mut().renderer` with safe partial borrows on struct fields (no raw pointers
needed).

**Frame flow** (from host's perspective):

```
// init — host provides GL context, crate creates everything else
let runtime = WasmWidgetRuntime::new(&wasm_bytes, gl, width, height)?;

// each frame
runtime.renderer().begin_frame(width, height);  // borrow ends at semicolon
draw_background(runtime.renderer());             // testbed-specific, optional
runtime.render(delta_ms)?;                       // WASM executes, tree renders
draw_stats(runtime.renderer());                  // testbed-specific, optional
runtime.renderer().flush();
surface.swap_buffers(&gl_context)?;              // host's responsibility
```

Each `runtime.renderer()` returns a temporary `&mut FemtoVgRenderer` — NLL ensures the borrow ends before the next call.

## Step 1: Workspace dependencies

**File: `Cargo.toml` (workspace root)**

Add to `[workspace.dependencies]`:

```toml
femtovg = "0.9"
glow = "0.16"
winit = "0.30"
glutin = "0.32"
glutin-winit = "0.5"
raw-window-handle = "0.6"
```

**File: `bmc-wasm-runtime/Cargo.toml`**

```toml
[dependencies]
# ... existing: anyhow, bmc-wasm-protocol, chrono, tracing, wasmi, taffy
cosmic-text.workspace = true   # KEEP for paragraph layout
femtovg.workspace = true       # GPU rendering
glow.workspace = true          # OpenGL abstraction
# REMOVE: tiny-skia, minifb

# Testbed-only
winit = { workspace = true, optional = true }
glutin = { workspace = true, optional = true }
glutin-winit = { workspace = true, optional = true }
raw-window-handle = { workspace = true, optional = true }
notify = { workspace = true, optional = true }

[features]
testbed = ["winit", "glutin", "glutin-winit", "raw-window-handle", "notify"]
```

## Step 2: Renderer trait — `src/renderer.rs` (NEW)

All coordinates f32. Color u32 in `0xRRGGBBAA` format.

```rust
pub trait Renderer {
    // Shapes
    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: u32);
    fn fill_rounded_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, color: u32);
    fn fill_circle(&mut self, cx: f32, cy: f32, r: f32, color: u32);
    fn stroke_rect(&mut self, x: f32, y: f32, w: f32, h: f32, border_width: f32, color: u32);
    fn draw_line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, width: f32, color: u32);

    // Transform stack (for canvas DrawCommand::Rotated)
    fn save(&mut self);
    fn restore(&mut self);
    fn translate(&mut self, x: f32, y: f32);
    fn rotate(&mut self, angle_radians: f32);

    // Scissor clipping (for modal scroll)
    fn push_scissor(&mut self, x: f32, y: f32, w: f32, h: f32);
    fn pop_scissor(&mut self);

    // Simple text
    fn draw_text(&mut self, text: &str, x: f32, y: f32, size: f32, color: u32);
    fn measure_text(&mut self, text: &str, size: f32) -> f32;

    // Rich text paragraphs (cosmic-text layout + FemtoVG rendering)
    fn measure_paragraph(&mut self, style: &TextStyle, spans: &[SpanData], max_width: Option<f32>) -> (f32, f32);
    fn draw_paragraph(&mut self, style: &TextStyle, spans: &[SpanData], x: f32, y: f32, max_width: f32);
    fn draw_paragraph_clipped(&mut self, style: &TextStyle, spans: &[SpanData], x: f32, y: f32, max_width: f32, clip_top: f32, clip_bottom: f32);

    // Frame lifecycle
    fn begin_frame(&mut self, width: u32, height: u32);
    fn flush(&mut self);
    fn width(&self) -> f32;
    fn height(&self) -> f32;
}
```

## Step 3: FemtoVgRenderer — `src/gpu/renderer.rs` (NEW)

```rust
pub struct FemtoVgRenderer {
    canvas: Canvas<OpenGl>,
    font_regular: FontId,    // FemtoVG font for rendering
    font_bold: FontId,       // FemtoVG font for rendering
    font_system: FontSystem, // cosmic-text for paragraph layout
    paragraph_cache: ParagraphLayoutCache,
    width: f32,
    height: f32,
    frame_counter: u64,
}
```

**Construction**: Takes `glow::Context`, loads BraiinsSans fonts into both FemtoVG (`add_font_mem`) and cosmic-text (
`db_mut().load_font_data`). Uses `FontSystem::new_with_locale_and_db` with empty DB to avoid loading system fonts.

**Shape methods**: Direct FemtoVG mapping:

- `fill_rect` → `Path::new().rect()` + `canvas.fill_path()`
- `fill_rounded_rect` → `Path::new().rounded_rect()` + `canvas.fill_path()`
- `fill_circle` → `Path::new().circle()` + `canvas.fill_path()`
- `stroke_rect` → `Path::new().rect()` + `canvas.stroke_path()`
- `draw_line` → `Path::new().move_to().line_to()` + `canvas.stroke_path()`

**Transform/scissor**: Direct `canvas.save/restore/translate/rotate/scissor/reset_scissor`.

**Simple text**: FemtoVG `fill_text`/`measure_text` with Paint configured from size/color/font.

**Font selection**: weight < 600 → regular, >= 600 → bold (same as corinthia branch).

**Frame lifecycle**:

- `begin_frame` → `canvas.set_size()` + `canvas.clear_rect()` + cache GC
- `flush` → `canvas.flush()`

## Step 4: Paragraph rendering — `src/gpu/text.rs` (NEW)

Hybrid cosmic-text (layout) + FemtoVG (rendering). Both use rustybuzz internally with same BraiinsSans font binary, so
glyph advances match.

**ParagraphLayoutCache**: `HashMap<u64, ParagraphLayoutEntry>` with frame-based GC (same pattern as current
`ShapedTextCache`).

```rust
struct ParagraphLayoutEntry {
    buffer: Buffer,        // cosmic-text shaped buffer — kept for layout_runs() during render
    width: f32,
    height: f32,
    max_width: f32,
    last_used_frame: u64,
}
```

**measure_paragraph**: Check cache → if miss, create cosmic-text `Buffer`, call `set_rich_text()` with spans converted
to `cosmic_text::Attrs`, `shape_until_scroll()`, read `layout_runs()` for dimensions. Cache result.

**draw_paragraph**: Get cached Buffer, walk `layout_runs()`. For each run, group glyphs by originating span. For each
contiguous same-span segment, call FemtoVG `fill_text()` at the x-position from cosmic-text's glyph data. Draw
underline/strikethrough decorations as thin filled rects.

**draw_paragraph_clipped**: Wrap `draw_paragraph` in `push_scissor/pop_scissor`.

## Step 5: Refactor tree.rs

**`process_tree` signature** changes from 12 params to 9:

```
pub fn process_tree(
    data: &[u8],
    width: f32, height: f32,           // was u32
    renderer: &mut dyn Renderer,       // replaces pixmap + font_system + swash_cache + text_cache
    interaction: &mut InteractionState,
    modal_states: &mut HashMap<u16, ModalState>,
    animation_states: &mut HashMap<u64, AnimationState>,
    transition_states: &mut HashMap<(u16, u16), TransitionState>,
    frame_counter: u64,
    delta_ms: u32,
) -> Result<(TreeResult, bool)>
```

**Taffy measure callback**: The closure captures `renderer` as `&mut dyn Renderer` and calls
`renderer.measure_paragraph()`. After `compute_layout_with_measure` returns, the borrow ends and `renderer` is available
for the render pass. Same borrow pattern as the current `font_system`/`text_cache`.

**Mechanical changes** (~1900 lines):

- `fill_rect(&mut pixmap, x, y, w, h, color)` → `renderer.fill_rect(x as f32, y as f32, w as f32, h as f32, color)`
- `draw_rounded_rect(&mut pixmap, ...)` → `renderer.fill_rounded_rect(...)`
- `draw_circle(&mut pixmap, ...)` → `renderer.fill_circle(...)`
- `render_paragraph(pixmap, font_system, swash_cache, text_cache, ...)` → `renderer.draw_paragraph(...)`
- `measure_paragraph(font_system, text_cache, ...)` → `renderer.measure_paragraph(...)`
- Rotation: `tiny_skia::Transform::from_rotate_at()` + manual math → `renderer.save()` / `translate(pivot)` /
  `rotate(angle)` / draw / `restore()`
- Modal body clipping: pixel-level `blit_paragraph` → `renderer.push_scissor()` / `draw_paragraph` / `pop_scissor()`

## Step 6: Refactor host_api.rs + runtime.rs

**HostState** — renderer lives here (no raw pointers):

```rust
pub struct HostState {
    pub renderer: FemtoVgRenderer,                              // NEW — replaces pixmap + font_system + swash_cache + text_cache
    pub interaction: InteractionState,
    pub modal_states: HashMap<u16, ModalState>,
    pub animation_states: HashMap<u64, AnimationState>,
    pub transition_states: HashMap<(u16, u16), TransitionState>,
    pub frame_counter: u64,
    // ... frame control flags, cached_tree_data, etc (unchanged)
}
```

**Host functions** use safe partial struct borrows:

```
// host_submit_tree
let state = caller.data_mut();
tree::process_tree(
    &data, width as f32, height as f32,
    &mut state.renderer,          // disjoint field borrow
    &mut state.interaction,       // disjoint field borrow
    &mut state.modal_states,      // disjoint field borrow
    &mut state.animation_states,
    &mut state.transition_states,
    state.frame_counter,
    state.delta_ms,
);
```

```
// host_fill_rect — direct renderer access
let state = caller.data_mut();
state.renderer.fill_rect(x as f32, y as f32, w as f32, h as f32, color);
```

```
// host_button — renderer + interaction
let state = caller.data_mut();
let clicked = draw_button(
    &mut state.renderer, &mut state.interaction,
    &key, &label, x as f32, y as f32, w as f32, h as f32, ButtonStyle::from(style),
);
```

**WasmWidgetRuntime** — public API for the host:

```
impl WasmWidgetRuntime {
    pub fn new(wasm_bytes: &[u8], gl: glow::Context, width: u32, height: u32) -> Result<Self>;
    pub fn render(&mut self, delta_ms: u32) -> Result<()>;     // WASM execution + tree processing
    pub fn renderer(&mut self) -> &mut FemtoVgRenderer;         // host accesses renderer
    pub fn push_touch_event(&mut self, event: TouchEvent);
    pub fn wants_next_frame(&self) -> bool;
    pub fn next_frame_delay(&self) -> Option<u32>;
}
```

**Hot-reload**: Only `WasmWidgetRuntime` is recreated (new WASM module + store). The `glow::Context` and glutin surface
live in the testbed and persist. A new `FemtoVgRenderer` is created inside the new runtime from the same GL context.
Font atlas is re-uploaded but this is a dev-time operation.

## Step 7: Refactor button.rs

```
pub fn draw_button(
    renderer: &mut dyn Renderer,
    interaction: &mut InteractionState,
    key: &str, label: &str,
    x: f32, y: f32, w: f32, h: f32,
    style: ButtonStyle,
) -> bool
```

Same logic, swap `pixmap`/`font_system`/`swash_cache`/`text_cache` calls to `renderer.*` calls.

## Step 8: Rewrite testbed — `src/bin/testbed.rs`

The testbed mimics the remote-widget host role: provides GL context + events, calls runtime API.

winit 0.30 `ApplicationHandler` trait + glutin OpenGL context.

**Init**: parse args → `EventLoop::new()` → `DisplayBuilder` for window + GL config → create GL surface + context →
`glow::Context` → `WasmWidgetRuntime::new(wasm_bytes, gl, width, height)` → file watcher.

**Event loop**:

- `WindowEvent::CursorMoved/MouseInput/MouseWheel` → convert to TouchEvent, push to runtime
- `WindowEvent::KeyboardInput(Escape)` / `CloseRequested` → exit
- `WindowEvent::RedrawRequested` → check hot-reload, compute delta, render frame:
  ```
  runtime.renderer().begin_frame(w, h);
  draw_background(runtime.renderer());
  runtime.render(delta_ms)?;
  draw_stats(runtime.renderer());
  runtime.renderer().flush();
  surface.swap_buffers()?;
  ```

**Background**: FemtoVG `Paint::linear_gradient()` or two blended rects.

**Stats overlay**: `renderer.draw_text()` for labels, `renderer.fill_rect()` for frame chart bars. No separate pixmap.

**Hot-reload**: Drop old `WasmWidgetRuntime`, create new one with fresh `glow::Context` clone or by re-creating from GL
loader. GL context persists in glutin.

## Step 9: Cleanup

- DELETE `src/drawing/` module (shapes.rs, text.rs, mod.rs)
- REMOVE `tiny-skia` from bmc-wasm-runtime/Cargo.toml (keep in workspace if other crates use it)
- REMOVE `minifb` from bmc-wasm-runtime/Cargo.toml (keep in workspace if other crates use it)
- Keep `cosmic-text` (used by FemtoVgRenderer for paragraph layout)
- Update `src/lib.rs`: remove `pub mod drawing`, add `pub mod renderer` + `pub mod gpu`

## File change summary

| File                          | Action                                                                                                           |
|-------------------------------|------------------------------------------------------------------------------------------------------------------|
| `Cargo.toml` (workspace)      | +femtovg, +glow, +winit, +glutin, +glutin-winit, +raw-window-handle                                              |
| `bmc-wasm-runtime/Cargo.toml` | +femtovg, +glow, +winit (opt), +glutin (opt), +glutin-winit (opt), +raw-window-handle (opt); -tiny-skia, -minifb |
| `src/renderer.rs`             | NEW — Renderer trait                                                                                             |
| `src/gpu/mod.rs`              | NEW — re-exports FemtoVgRenderer                                                                                 |
| `src/gpu/renderer.rs`         | NEW — FemtoVgRenderer (~300 lines)                                                                               |
| `src/gpu/text.rs`             | NEW — ParagraphLayoutCache + cosmic-text/FemtoVG integration (~250 lines)                                        |
| `src/lib.rs`                  | Remove `pub mod drawing`, add `pub mod renderer` + `pub mod gpu`                                                 |
| `src/tree.rs`                 | Refactor process_tree + all render fns to use `&mut dyn Renderer` (~1900 lines touched)                          |
| `src/host_api.rs`             | Replace pixmap/font_system/swash_cache/text_cache with FemtoVgRenderer                                           |
| `src/runtime.rs`              | new() takes glow::Context, expose renderer(), remove get_overlay()                                               |
| `src/components/button.rs`    | Take `&mut dyn Renderer` instead of 4 params                                                                     |
| `src/bin/testbed.rs`          | Complete rewrite with winit + glutin — thin host role                                                            |
| `src/drawing/mod.rs`          | DELETE                                                                                                           |
| `src/drawing/shapes.rs`       | DELETE                                                                                                           |
| `src/drawing/text.rs`         | DELETE (paragraph cache logic extracted to `src/gpu/text.rs`)                                                    |

## Verification

1. `cargo run --bin testbed --features testbed -- examples/hello-widget/target/wasm32-unknown-unknown/release/hello_widget.wasm`
2. Visual: shapes, text, rounded rects, buttons, modals, animations render correctly
3. Wayland: window has server-side decorations (fixes minifb limitation)
4. Hot-reload: modify + rebuild WASM while testbed runs
5. Interaction: button clicks, modal scroll, touch events work
6. Performance: perf overlay shows frame times — target is matching or improving on the current 2ms/247FPS
